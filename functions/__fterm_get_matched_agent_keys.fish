#!/usr/bin/env fish

function __fterm_get_matched_agent_keys --description 'Get agent keys matching IdentityFile'
	builtin set --local target_host "$argv[1]"
	__fterm_debug "=== __fterm_get_matched_agent_keys called ==="
	__fterm_debug "target_host: $target_host"

	builtin set --local config_args (__fterm_get_ssh_config_args)
	__fterm_debug "config_args: $config_args"

	# Set HOME for Windows OpenSSH
	builtin set --local original_home "$HOME"
	if builtin set --query MSYSTEM; and builtin set --query USERPROFILE
		builtin set --export HOME "$(command cygpath -m "$USERPROFILE")"
		__fterm_debug "HOME changed for Windows: $HOME"
	end

	# Get identity files from ssh -G
	builtin set --local identity_files (__fterm_run_ssh_cmd ssh $config_args -G "$target_host" | command awk '/^identityfile / { print $2 }')
	__fterm_debug "identity_files: $identity_files"

	# Get agent keys (use cache from ssh.fish/scp.fish if available)
	builtin set --local agent_output
	builtin set --local agent_status 1
	if builtin set --query __fterm_cached_agent_status; and builtin test "$__fterm_cached_agent_status" -eq 0
		builtin set agent_output $__fterm_cached_agent_keys
		builtin set agent_status 0
		__fterm_debug "Using cached agent keys"
	else
		builtin set agent_output (__fterm_run_ssh_cmd ssh-add -l)
		builtin set agent_status $status
	end
	__fterm_debug "ssh-add -l status: $agent_status"
	__fterm_debug "agent_output: $agent_output"

	if builtin test $agent_status -eq 0; and builtin test -n "$agent_output"
		for id_file in $identity_files
			builtin set --local expanded_file (string replace "~" "$HOME" "$id_file")
			# MSYS2: Convert to UNIX path for fish test -f
			builtin set --local test_path "$expanded_file"
			if builtin set --query MSYSTEM
				builtin set test_path (command cygpath -u "$expanded_file")
			end
			__fterm_debug "Checking id_file: $id_file -> expanded: $expanded_file (test_path: $test_path)"
			if builtin test -f "$test_path"
				builtin set --local id_fingerprint (__fterm_run_ssh_cmd ssh-keygen -lf "$expanded_file" | command awk '{print $2}')
				__fterm_debug "id_fingerprint for $expanded_file: $id_fingerprint"
				if builtin test -n "$id_fingerprint"
					for agent_key in $agent_output
						__fterm_debug "Comparing agent_key: $agent_key with fingerprint: $id_fingerprint"
						if string match -q "*$id_fingerprint*" "$agent_key"
							__fterm_debug "MATCH FOUND: $agent_key"
							builtin echo "$agent_key (from: $id_file)"
						end
					end
				end
			else
				__fterm_debug "File not found: $expanded_file"
			end
		end
	else
		__fterm_debug "ssh-add check failed or no keys in agent"
	end

	# Restore HOME
	if builtin test -n "$original_home"
		builtin set --export HOME "$original_home"
	end
end
