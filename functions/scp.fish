#!/usr/bin/env fish

function scp --description 'SCP with logging support'
	__fterm_debug "=== SCP function called ==="
	__fterm_debug "argv: $argv"
	__fterm_debug "__fssh_scp_cmd: $__fssh_scp_cmd"
	__fterm_debug "__fssh_ssh_config: $__fssh_ssh_config"
	__fterm_debug "__fssh_ssh_add_cmd: $__fssh_ssh_add_cmd"

	# Record transfer start time (epoch seconds)
	builtin set --local start_time (command date '+%s')

	# Skip tmux check for dry-run options (they don't need logging)
	if not __fssh_scp_is_dry_run $argv
		# Ensure running inside tmux for logging
		if not __fterm_ensure_tmux scp $argv
			return $status
		end
	end

	# Load SSH environment if exists
	if builtin set --query SSH_ENV; and builtin test -f "$SSH_ENV"
		set_color blue
		builtin echo "[INFO ] Loading ssh environment: $SSH_ENV ..."
		set_color normal
		builtin source "$SSH_ENV" >/dev/null
		__fterm_debug "Loaded SSH_ENV: $SSH_ENV"
	end

	# Check SSH agent connection and cache key list for downstream functions.
	# This prevents repeated ssh-add calls that can freeze the terminal when
	# the agent is unresponsive via agent forwarding (5 calls → 1 call).
	builtin set --global __fterm_cached_agent_keys
	builtin set --global __fterm_cached_agent_status 1
	builtin set --local agent_output (__fterm_run_ssh_cmd ssh-add -l)
	if builtin test $status -eq 0
		builtin set --global __fterm_cached_agent_status 0
		builtin set --global __fterm_cached_agent_keys $agent_output
		set_color blue
		builtin echo "[INFO ] ssh-add connection successful."
		set_color normal
		__fterm_debug "ssh-add check passed, cached (count $agent_output) keys"
	else
		set_color red
		builtin echo "[ERROR] ssh-add connection failed."
		set_color normal
		__fterm_debug "ssh-add check failed"
		builtin set --erase __fterm_cached_agent_keys
		builtin set --erase __fterm_cached_agent_status
		return 1
	end

	# Extract remote hosts from arguments
	builtin set --local remote_hosts (__fssh_scp_extract_hosts $argv)
	__fterm_debug "remote_hosts: $remote_hosts"

	# Run config check for each remote host (skip for dry-run)
	if not __fssh_scp_is_dry_run $argv
		set_color blue
		builtin echo "[INFO ] Running SSH config validation..."
		set_color normal

		for remote_host in $remote_hosts
			# Extract host part (remove user@ prefix if present)
			builtin set --local check_host "$remote_host"
			if builtin string match --quiet '*@*' -- "$remote_host"
				builtin set check_host (builtin string replace --regex '^[^@]+@' '' "$remote_host")
			end

			if not __fssh_config_check "$check_host"
				set_color red
				builtin echo "[ERROR] Config validation failed for host: $check_host"
				builtin echo "[ERROR] Transfer aborted."
				set_color normal
				return 1
			end
		end
	end

	# Build user@host format for each host (add user from config if not specified)
	builtin set --local hosts_with_user
	for host in $remote_hosts
		if string match --quiet '*@*' -- "$host"
			builtin set --append hosts_with_user "$host"
		else
			# Get user from ssh config
			builtin set --local conn_info (string split \t -- (__fssh_ssh_get_connection_info "$host"))
			builtin set --local ssh_user "$conn_info[1]"
			if builtin test -n "$ssh_user"
				builtin set --append hosts_with_user "$ssh_user@$host"
			else
				builtin set --append hosts_with_user "$host"
			end
		end
	end
	builtin set --local hosts_str (string join ' ' -- $hosts_with_user)
	__fterm_debug "hosts_with_user: $hosts_str"

	# Generate log file path (only for actual connections, not dry-run)
	# Format: YYYYMMDDTHHMMSS_{session}-{window}{pane}_scp_{user}@{hosts}.log
	builtin set --local log_file ""
	builtin set --local is_dry_run 0
	if __fssh_scp_is_dry_run $argv
		builtin set is_dry_run 1
		__fterm_debug "Dry-run mode detected, logging disabled"
	else if type --query tmux; and builtin set --query TMUX
		builtin set --local date_path "$(command date '+%Y/%m/%d')"
		builtin set --local timestamp_file "$(command date '+%Y%m%dT%H%M%S')"
		builtin set --local pane_info "$(command tmux display-message -p "#{session_name}-#{window_index}#{pane_index}")"
		# Sanitize hosts for filename (replace spaces with _)
		builtin set --local hosts_filename (string replace --all ' ' '_' -- "$hosts_str")
		builtin set log_file "$FTERM_LOG_DIR_PREFIX$date_path/"$timestamp_file"_"$pane_info"_scp_$hosts_filename.log"

		__fterm_debug "log_file: $log_file"

		# Start logging
		__fterm_start_logging "$log_file" "$remote_hosts[1]" "scp"
	else
		set_color yellow
		builtin echo "[WARN ] tmux not available or not in tmux session, logging disabled"
		set_color normal
		__fterm_debug "tmux not available or not in tmux session, logging disabled"
	end

	# Show splash
	__fssh_scp_status_splash "$hosts_str" "$log_file" $argv

	# Execute SCP
	builtin set --local scp_cmd "$(__fssh_get_scp_cmd)"
	builtin set --local config_args (__fterm_get_ssh_config_args)

	# Set HOME for Windows OpenSSH to resolve Include paths correctly
	builtin set --local original_home "$HOME"
	if builtin set --query MSYSTEM; and builtin set --query USERPROFILE
		builtin set --export HOME "$(command cygpath -m "$USERPROFILE")"
	end

	__fterm_debug "Executing: $scp_cmd $config_args $argv"

	# Set tmux pane title before transfer
	builtin set --local original_pane_title ""
	if type --query tmux; and builtin set --query TMUX
		# Debug: show all panes state before modification
		__fterm_debug "tmux: === pane state before modification ==="
		for pane_info in (command tmux list-panes -a -F '#{session_name}:#{window_index}.#{pane_index} #{window_name} #{pane_title} #{pane_current_command}')
			__fterm_debug "tmux: $pane_info"
		end

		# Save current pane title for restoration after transfer
		set original_pane_title "$(command tmux display-message -p '#{pane_title}')"
		__fterm_debug "tmux: original_pane_title: $original_pane_title"

		command tmux set-window-option automatic-rename off
		__fterm_debug "tmux: set automatic-rename off"

		command tmux set-window-option allow-rename off
		__fterm_debug "tmux: set allow-rename off"

		builtin set --local new_pane_title "scp:$hosts_str"
		command tmux select-pane -T "$new_pane_title"
		__fterm_debug "tmux: select-pane -T $new_pane_title"

		# Set pane-specific user option for pane-border-format to display
		command tmux set-option -p @fterm_ssh_host "$new_pane_title"
		__fterm_debug "tmux: set-option -p @fterm_ssh_host $new_pane_title"

		# Verify settings
		builtin set --local current_pane_title "$(command tmux display-message -p '#{pane_title}')"
		__fterm_debug "tmux: current_pane_title after set: $current_pane_title"
		builtin set --local current_ssh_host "$(command tmux show-options -p -v @fterm_ssh_host)"
		__fterm_debug "tmux: @fterm_ssh_host after set: $current_ssh_host"
	end

	command "$scp_cmd" $config_args $argv
	builtin set --local scp_status $status

	# Restore original HOME
	if builtin test -n "$original_home"
		builtin set --export HOME "$original_home"
	end

	# Calculate transfer duration
	builtin set --local end_time (command date '+%s')
	builtin set --local duration (math $end_time - $start_time)
	builtin set --local duration_str (__fterm_format_duration $duration)

	# Output completion info
	if builtin test $scp_status -eq 0
		set_color green
		builtin echo ""
		builtin echo "#= >> Transfer Complete << ====================================================="
	else
		set_color red
		builtin echo ""
		builtin echo "#= >> Transfer Failed (exit: $scp_status) << ============================================="
	end
	builtin echo "#=    Timestamp     | $(command date +%Y-%m-%dT%H:%M:%S%z)"
	builtin echo "#=    Duration      | $duration_str"
	builtin echo "#=    Logfile       | $log_file"
	builtin echo "#= ============================================================================="
	set_color normal

	# Stop logging and cleanup (only if not dry-run)
	if test "$is_dry_run" -eq 0; and type --query tmux; and builtin set --query TMUX
		# Debug: show all panes state after transfer
		__fterm_debug "tmux: === pane state after transfer ==="
		for pane_info in (command tmux list-panes -a -F '#{session_name}:#{window_index}.#{pane_index} #{window_name} #{pane_title} #{pane_current_command}')
			__fterm_debug "tmux: $pane_info"
		end

		command tmux select-pane -P 'default'
		__fterm_debug "tmux: select-pane -P default"

		# Restore original pane title
		if builtin test -n "$original_pane_title"
			command tmux select-pane -T "$original_pane_title"
			__fterm_debug "tmux: restored pane title to: $original_pane_title"
		else
			command tmux select-pane -T "fish"
			__fterm_debug "tmux: restored pane title to: fish (default)"
		end

		# Unset pane-specific @fterm_ssh_host option
		command tmux set-option -p -u @fterm_ssh_host
		__fterm_debug "tmux: unset @fterm_ssh_host"

		command tmux set-window-option allow-rename on
		__fterm_debug "tmux: set allow-rename on"

		command tmux set-window-option automatic-rename on
		__fterm_debug "tmux: set automatic-rename on"

		__fterm_stop_logging "$log_file"
	end

	# Clean up agent key cache
	builtin set --erase __fterm_cached_agent_keys
	builtin set --erase __fterm_cached_agent_status

	__fterm_debug "=== SCP function completed ==="
	return $scp_status
end
