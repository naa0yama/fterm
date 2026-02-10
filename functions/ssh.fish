#!/usr/bin/env fish

function ssh --description 'SSH with logging support'
	__fterm_debug "=== SSH function called ==="
	__fterm_debug "argv: $argv"
	__fterm_debug "__fssh_ssh_cmd: $__fssh_ssh_cmd"
	__fterm_debug "__fssh_ssh_config: $__fssh_ssh_config"
	__fterm_debug "__fssh_ssh_add_cmd: $__fssh_ssh_add_cmd"

	# Record connection start time (epoch seconds)
	builtin set --local start_time (command date '+%s')

	# Skip tmux check for dry-run options (they don't need logging)
	if not __fssh_ssh_is_dry_run $argv
		# Ensure running inside tmux for logging
		if not __fterm_ensure_tmux ssh $argv
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

	# Get target host (last argument)
	builtin set --local target_host "$argv[-1]"
	__fterm_debug "target_host: $target_host"

	# Run config check before connection (skip for dry-run)
	if not __fssh_ssh_is_dry_run $argv
		set_color blue
		builtin echo "[INFO ] Running SSH config validation..."
		set_color normal

		if not __fssh_config_check "$target_host"
			set_color red
			builtin echo "[ERROR] Config validation failed. Connection aborted."
			set_color normal
			return 1
		end
	end

	# Get connection info: user, host, port
	builtin set --local conn_info (string split \t -- (__fssh_ssh_get_connection_info "$target_host"))
	builtin set --local ssh_user "$conn_info[1]"
	builtin set --local ssh_host "$conn_info[2]"
	builtin set --local ssh_port "$conn_info[3]"
	builtin set --local ssh_info "$ssh_user@$ssh_host:$ssh_port"

	__fterm_debug "Connection info - user: $ssh_user, host: $ssh_host, port: $ssh_port"

	# Generate log file path (only for actual connections, not dry-run)
	# Format: YYYYMMDDTHHMMSS_{session}-{window}{pane}_ssh_{user}@{host}.log
	builtin set --local log_file ""
	builtin set --local is_dry_run 0
	if __fssh_ssh_is_dry_run $argv
		builtin set is_dry_run 1
		__fterm_debug "Dry-run mode detected, logging disabled"
	else if type --query tmux; and builtin set --query TMUX
		builtin set --local date_path "$(command date '+%Y/%m/%d')"
		builtin set --local timestamp_file "$(command date '+%Y%m%dT%H%M%S')"
		builtin set --local pane_info "$(command tmux display-message -p "#{session_name}-#{window_index}#{pane_index}")"
		# Use user@target_host if @ not already in target_host
		if string match --quiet '*@*' -- "$target_host"
			builtin set log_file "$FTERM_LOG_DIR_PREFIX$date_path/"$timestamp_file"_"$pane_info"_ssh_$target_host.log"
		else
			builtin set log_file "$FTERM_LOG_DIR_PREFIX$date_path/"$timestamp_file"_"$pane_info"_ssh_$ssh_user@$target_host.log"
		end

		__fterm_debug "log_file: $log_file"

		# Start logging
		__fterm_start_logging "$log_file" "$target_host" "$ssh_info"
	else
		set_color yellow
		builtin echo "[WARN ] tmux not available or not in tmux session, logging disabled"
		set_color normal
		__fterm_debug "tmux not available or not in tmux session, logging disabled"
	end

	# Show splash
	__fssh_ssh_status_splash "$ssh_info" "$log_file" "$target_host" $argv

	# Execute SSH
	builtin set --local ssh_cmd "$(__fterm_get_ssh_cmd)"
	builtin set --local config_args (__fterm_get_ssh_config_args)

	# Set HOME for Windows OpenSSH to resolve Include paths correctly
	builtin set --local original_home "$HOME"
	if builtin set --query MSYSTEM; and builtin set --query USERPROFILE
		builtin set --export HOME "$(command cygpath -m "$USERPROFILE")"
	end

	__fterm_debug "Executing: $ssh_cmd $config_args $argv"

	# Set tmux pane title before connection
	builtin set --local original_pane_title ""
	if type --query tmux; and builtin set --query TMUX
		# Debug: show all panes state before modification
		__fterm_debug "tmux: === pane state before modification ==="
		for pane_info in (command tmux list-panes -a -F '#{session_name}:#{window_index}.#{pane_index} #{window_name} #{pane_title} #{pane_current_command}')
			__fterm_debug "tmux: $pane_info"
		end

		# Save current pane title for restoration after disconnect
		set original_pane_title "$(command tmux display-message -p '#{pane_title}')"
		__fterm_debug "tmux: original_pane_title: $original_pane_title"

		# Get current SSH connection count for this window (default 0)
		builtin set --local current_count "$(command tmux show-window-options -v @fterm_ssh_count 2>/dev/null)"
		if builtin test -z "$current_count"
			builtin set current_count 0
		end

		# Increment count
		builtin set --local new_count (math $current_count + 1)
		command tmux set-window-option @fterm_ssh_count "$new_count"
		__fterm_debug "tmux: @fterm_ssh_count incremented to $new_count"

		# Set rename options off (safe to call multiple times)
		command tmux set-window-option automatic-rename off
		__fterm_debug "tmux: set automatic-rename off"

		command tmux set-window-option allow-rename off
		__fterm_debug "tmux: set allow-rename off"

		builtin set --local new_pane_title "ssh:$ssh_user@$target_host"
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

	command "$ssh_cmd" $config_args $argv

	# Restore original HOME
	if builtin test -n "$original_home"
		builtin set --export HOME "$original_home"
	end

	# Calculate connection duration
	builtin set --local end_time (command date '+%s')
	builtin set --local duration (math $end_time - $start_time)
	builtin set --local duration_str (__fterm_format_duration $duration)

	# Output disconnection info
	set_color blue
	builtin echo ""
	builtin echo "#= >> Disconnected << =========================================================="
	builtin echo "#=    Timestamp     | $(command date +%Y-%m-%dT%H:%M:%S%z)"
	builtin echo "#=    Duration      | $duration_str"
	builtin echo "#=    Logfile       | $log_file"
	builtin echo "#= ============================================================================="
	set_color normal

	# Stop logging and cleanup (only if not dry-run)
	if test "$is_dry_run" -eq 0; and type --query tmux; and builtin set --query TMUX
		# Debug: show all panes state after disconnect
		__fterm_debug "tmux: === pane state after disconnect ==="
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

		# Decrement SSH connection count for this window
		builtin set --local current_count "$(command tmux show-window-options -v @fterm_ssh_count 2>/dev/null)"
		if builtin test -z "$current_count"; or builtin test "$current_count" -le 0
			builtin set current_count 1
		end

		builtin set --local new_count (math $current_count - 1)
		command tmux set-window-option @fterm_ssh_count "$new_count"
		__fterm_debug "tmux: @fterm_ssh_count decremented to $new_count"

		# Only restore rename options when all SSH connections in this window are closed
		if builtin test "$new_count" -eq 0
			command tmux set-window-option allow-rename on
			__fterm_debug "tmux: set allow-rename on (all SSH connections closed)"

			command tmux set-window-option automatic-rename on
			__fterm_debug "tmux: set automatic-rename on (all SSH connections closed)"

			# Unset the count option
			command tmux set-window-option -u @fterm_ssh_count
			__fterm_debug "tmux: unset @fterm_ssh_count"
		else
			__fterm_debug "tmux: keeping allow-rename off ($new_count SSH connections remaining)"
		end

		__fterm_stop_logging "$log_file"
	end

	# Clean up agent key cache
	builtin set --erase __fterm_cached_agent_keys
	builtin set --erase __fterm_cached_agent_status

	__fterm_debug "=== SSH function completed ==="
end
