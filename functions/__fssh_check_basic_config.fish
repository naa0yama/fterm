#!/usr/bin/env fish

function __fssh_check_basic_config --description 'Check basic SSH config (HostName, User, Port, IdentitiesOnly, IdentityFile)'
	# Arguments: [host]
	builtin set --local host "$argv[1]"

	__fterm_debug "=== __fssh_check_basic_config called ==="
	__fterm_debug "host: $host"

	builtin set --local config_args (__fterm_get_ssh_config_args)
	__fterm_debug "config_args: $config_args"

	builtin set --local has_error 0
	builtin set --local has_warn 0

	# Set HOME for Windows OpenSSH
	builtin set --local original_home "$HOME"
	if builtin set --query MSYSTEM; and builtin set --query USERPROFILE
		builtin set --export HOME "$(command cygpath -m "$USERPROFILE")"
		__fterm_debug "HOME set for Windows: $HOME"
	end

	# Get all config values at once
	builtin set --local config_output (__fterm_run_ssh_cmd ssh $config_args -G "$host")
	__fterm_debug "config_output line count: "(count $config_output)

	if builtin test -z "$config_output"
		builtin set --global --append __fssh_check_messages "E:[ERROR] Host '$host': Failed to get SSH config (ssh -G failed)"
		builtin echo "ERROR:ssh_config_failed"

		# Restore HOME
		if builtin test -n "$original_home"
			builtin set --export HOME "$original_home"
		end
		return 1
	end

	# Parse config values (use printf to preserve newlines in list)
	builtin set --local hostname_val (printf '%s\n' $config_output | command awk '/^hostname / { print $2 }')
	builtin set --local user_val (printf '%s\n' $config_output | command awk '/^user / { print $2 }')
	builtin set --local port_val (printf '%s\n' $config_output | command awk '/^port / { print $2 }')
	builtin set --local identities_only (printf '%s\n' $config_output | command awk '/^identitiesonly / { print $2 }')
	builtin set --local identity_file (printf '%s\n' $config_output | command awk '/^identityfile / { print $2 }')

	__fterm_debug "hostname: $hostname_val"
	__fterm_debug "user: $user_val"
	__fterm_debug "port: $port_val"
	__fterm_debug "identitiesonly: $identities_only"
	__fterm_debug "identityfile: $identity_file"

	# Check 3: Required fields (HostName, User, Port) - ERROR if missing
	# Note: HostName defaults to the host alias if not specified
	if builtin test -z "$hostname_val"; or builtin test "$hostname_val" = "$host"
		# If HostName equals the alias, it might be intentional (connecting to actual hostname)
		# But we should warn if it looks like an alias pattern (contains dots like org.env.hostname)
		if builtin string match --quiet '*.*.*' "$host"
			builtin set --global --append __fssh_check_messages "E:[ERROR] Host '$host': HostName not configured (using alias as hostname)"
			builtin echo "ERROR:hostname_missing"
			builtin set has_error 1
		end
	end

	if builtin test -z "$user_val"
		builtin set --global --append __fssh_check_messages "E:[ERROR] Host '$host': User not configured"
		builtin echo "ERROR:user_missing"
		builtin set has_error 1
	end

	if builtin test -z "$port_val"
		builtin set --global --append __fssh_check_messages "E:[ERROR] Host '$host': Port not configured"
		builtin echo "ERROR:port_missing"
		builtin set has_error 1
	end

	# Check 4: Recommended fields (IdentitiesOnly, IdentityFile) - WARN if missing
	if builtin test -z "$identities_only"; or builtin test "$identities_only" != "yes"
		builtin set --global --append __fssh_check_messages "W:[WARN ] Host '$host': IdentitiesOnly not set to 'yes'"
		builtin set --global --append __fssh_check_messages "W:        Recommendation: Set 'IdentitiesOnly yes' to prevent trying all agent keys"
		builtin echo "WARN:identitiesonly_not_set"
		builtin set has_warn 1
	end

	if builtin test -z "$identity_file"
		builtin set --global --append __fssh_check_messages "W:[WARN ] Host '$host': IdentityFile not configured"
		builtin set --global --append __fssh_check_messages "W:        Recommendation: Specify an IdentityFile for explicit key selection"
		builtin echo "WARN:identityfile_missing"
		builtin set has_warn 1
	end

	# Restore HOME
	if builtin test -n "$original_home"
		builtin set --export HOME "$original_home"
	end

	if builtin test "$has_error" -eq 1
		return 1
	else if builtin test "$has_warn" -eq 1
		return 0
	else
		__fterm_debug "Basic config check passed for: $host"
		return 0
	end
end
