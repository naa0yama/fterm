#!/usr/bin/env fish

function __fssh_config_check --description 'Validate SSH config before connection'
	# Arguments: [target_host] (optional, if not provided checks all hosts)
	builtin set --local target_host "$argv[1]"

	__fterm_debug "=== __fssh_config_check called ==="
	__fterm_debug "target_host: $target_host"

	# Initialize message buffer
	builtin set --global __fssh_check_messages

	# Initialize counters
	builtin set --local error_count 0
	builtin set --local warn_count 0

	# Check 1: ControlMaster directory
	builtin set --local cm_result (__fssh_check_cm_dir)
	if builtin test "$cm_result" = "created"
		builtin set warn_count (math "$warn_count" + 1)
	end

	# Check 11: Syntax check (early exit if config is broken)
	if not __fssh_check_syntax
		set_color red
		builtin echo "[ERROR] SSH config syntax check failed. Please fix the config first."
		set_color normal
		builtin set --erase __fssh_check_messages
		return 1
	end

	# Check 10: Duplicate hosts
	builtin set --local dup_result (__fssh_check_hosts_duplicate)
	if builtin test -n "$dup_result"
		builtin set warn_count (math "$warn_count" + (count $dup_result))
	end

	# Get hosts to check
	builtin set --local hosts_to_check
	if builtin test -n "$target_host"
		builtin set hosts_to_check "$target_host"
	else
		builtin set hosts_to_check (__fssh_get_hosts)
	end

	# Track checked ProxyJump hosts to avoid duplicate checks
	builtin set --local checked_proxyjump_hosts

	for host in $hosts_to_check
		__fterm_debug "Checking host: $host"

		# Check 2: Host prefix consistency
		if not __fssh_check_host_prefix "$host"
			builtin set error_count (math "$error_count" + 1)
		end

		# Check 3,4: Basic config (HostName, User, Port, IdentitiesOnly, IdentityFile)
		builtin set --local basic_result (__fssh_check_basic_config "$host")
		builtin set --local basic_error_lines (builtin echo "$basic_result" | command grep "^ERROR:" || true)
		builtin set --local basic_warn_lines (builtin echo "$basic_result" | command grep "^WARN:" || true)
		if builtin test -n "$basic_error_lines"
			builtin set error_count (math "$error_count" + (count $basic_error_lines))
		end
		if builtin test -n "$basic_warn_lines"
			builtin set warn_count (math "$warn_count" + (count $basic_warn_lines))
		end

		# Check 6,7,8: IdentityFile checks
		builtin set --local id_result (__fssh_check_identity_file "$host")
		builtin set --local id_error_lines (builtin echo "$id_result" | command grep "^ERROR:" || true)
		builtin set --local id_warn_lines (builtin echo "$id_result" | command grep "^WARN:" || true)
		if builtin test -n "$id_error_lines"
			builtin set error_count (math "$error_count" + (count $id_error_lines))
		end
		if builtin test -n "$id_warn_lines"
			builtin set warn_count (math "$warn_count" + (count $id_warn_lines))
		end

		# Check 5,9: ProxyJump validation
		builtin set --local proxy_result (__fssh_check_proxyjump "$host" $checked_proxyjump_hosts)
		builtin set --local proxy_error_lines (builtin echo "$proxy_result" | command grep "^ERROR:" || true)
		builtin set --local proxy_warn_lines (builtin echo "$proxy_result" | command grep "^WARN:" || true)
		if builtin test -n "$proxy_error_lines"
			builtin set error_count (math "$error_count" + (count $proxy_error_lines))
		end
		if builtin test -n "$proxy_warn_lines"
			builtin set warn_count (math "$warn_count" + (count $proxy_warn_lines))
		end

		# Track ProxyJump hosts that were checked
		builtin set --local proxy_host (__fssh_get_config_value "$host" "proxyjump")
		if builtin test -n "$proxy_host"; and builtin test "$proxy_host" != "none"
			if not builtin contains "$proxy_host" $checked_proxyjump_hosts
				builtin set --append checked_proxyjump_hosts "$proxy_host"
			end
		end

		# Check 12: ControlPath writability
		builtin set --local cp_result (__fssh_check_control_path "$host")
		if builtin test -n "$cp_result"
			builtin set warn_count (math "$warn_count" + 1)
		end
	end

	# Summary
	if builtin test "$error_count" -gt 0
		set_color red
		builtin echo ""
		builtin echo "[ERROR] Config check failed: $error_count error(s), $warn_count warning(s)"
		set_color normal
	else if builtin test "$warn_count" -gt 0
		set_color yellow
		builtin echo ""
		builtin echo "[WARN ] Config check passed with $warn_count warning(s)"
		set_color normal
	else
		set_color green
		builtin echo ""
		builtin echo "[OK   ] Config check passed"
		set_color normal
	end

	# Display collected messages
	if builtin test (count $__fssh_check_messages) -gt 0
		for msg in $__fssh_check_messages
			builtin set --local level (builtin string sub --length 1 "$msg")
			builtin set --local text (builtin string sub --start 3 "$msg")
			switch "$level"
				case "E"
					set_color red
				case "W"
					set_color yellow
			end
			builtin echo "  $text"
			set_color normal
		end
	end

	# Cleanup message buffer
	builtin set --erase __fssh_check_messages

	if builtin test "$error_count" -gt 0
		return 1
	else
		return 0
	end
end

# Helper function to get a specific config value for a host
function __fssh_get_config_value --description 'Get a specific SSH config value for a host'
	builtin set --local host "$argv[1]"
	builtin set --local key "$argv[2]"

	builtin set --local config_args (__fterm_get_ssh_config_args)

	# Set HOME for Windows OpenSSH
	builtin set --local original_home "$HOME"
	if builtin set --query MSYSTEM; and builtin set --query USERPROFILE
		builtin set --export HOME "$(command cygpath -m "$USERPROFILE")"
	end

	builtin set --local value (__fterm_run_ssh_cmd ssh $config_args -G "$host" 2>/dev/null | command awk -v key="$key" 'tolower($1) == key { print $2 }')

	# Restore HOME
	if builtin test -n "$original_home"
		builtin set --export HOME "$original_home"
	end

	builtin echo "$value"
end
