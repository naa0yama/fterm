#!/usr/bin/env fish

function __fssh_check_identity_file --description 'Check IdentityFile existence, type, agent registration, and permissions'
	# Arguments: [host]
	builtin set --local host "$argv[1]"

	__fterm_debug "=== __fssh_check_identity_file called ==="
	__fterm_debug "host: $host"

	builtin set --local config_args (__fterm_get_ssh_config_args)

	# Set HOME for Windows OpenSSH
	builtin set --local original_home "$HOME"
	if builtin set --query MSYSTEM; and builtin set --query USERPROFILE
		builtin set --export HOME "$(command cygpath -m "$USERPROFILE")"
	end

	# Get IdentityFile values (can be multiple)
	builtin set --local identity_files (__fterm_run_ssh_cmd ssh $config_args -G "$host" 2>/dev/null | command awk '/^identityfile / { print $2 }')

	__fterm_debug "identity_files: $identity_files"

	# No IdentityFile configured
	if builtin test -z "$identity_files"
		# Restore HOME
		if builtin test -n "$original_home"
			builtin set --export HOME "$original_home"
		end
		__fterm_debug "No IdentityFile configured for: $host"
		return 0
	end

	# Check ssh-agent availability (use cache from ssh.fish/scp.fish if available)
	builtin set --local agent_available 0
	builtin set --local agent_keys ""

	if builtin set --query __fterm_cached_agent_status; and builtin test "$__fterm_cached_agent_status" -eq 0
		builtin set agent_available 1
		builtin set agent_keys $__fterm_cached_agent_keys
		__fterm_debug "Using cached agent keys"
	else if __fterm_run_ssh_cmd ssh-add -l >/dev/null 2>&1
		builtin set agent_available 1
		builtin set agent_keys (__fterm_run_ssh_cmd ssh-add -l 2>/dev/null)
		__fterm_debug "ssh-agent available, keys: $agent_keys"
	else
		__fterm_debug "ssh-agent not available or no keys"
	end

	builtin set --local has_error 0
	builtin set --local has_warn 0

	for id_file in $identity_files
		__fterm_debug "Checking identity file: $id_file"

		# Expand ~ to home directory
		builtin set --local expanded_file (builtin string replace '~' "$HOME" "$id_file")

		# For MSYS2, convert path for file system access
		builtin set --local test_path "$expanded_file"
		if builtin set --query MSYSTEM
			builtin set test_path (command cygpath -u "$expanded_file" 2>/dev/null)
			if builtin test -z "$test_path"
				builtin set test_path "$expanded_file"
			end
		end

		__fterm_debug "expanded_file: $expanded_file, test_path: $test_path"

		# Check 6: File existence
		if not builtin test -f "$test_path"
			builtin set --global --append __fssh_check_messages "E:[ERROR] Host '$host': IdentityFile not found: $id_file"
			builtin echo "ERROR:identityfile_not_found"
			builtin set has_error 1
			continue
		end

		# Determine key type using ssh-keygen
		# ssh-keygen -y -f <file> extracts public key from private key
		# If successful -> private key
		# If failed -> public key or invalid
		builtin set --local is_private_key 0
		builtin set --local is_public_key 0
		builtin set --local key_fingerprint ""

		# Try to extract public key from file (works only for private keys)
		if __fterm_run_ssh_cmd ssh-keygen -y -f "$expanded_file" >/dev/null 2>&1
			builtin set is_private_key 1
			# Get fingerprint of the private key
			builtin set key_fingerprint (__fterm_run_ssh_cmd ssh-keygen -lf "$expanded_file" 2>/dev/null | command awk '{ print $2 }')
			__fterm_debug "File is a private key: $id_file"
		else
			# Not a private key, check if it's a valid public key
			if __fterm_run_ssh_cmd ssh-keygen -lf "$expanded_file" >/dev/null 2>&1
				builtin set is_public_key 1
				builtin set key_fingerprint (__fterm_run_ssh_cmd ssh-keygen -lf "$expanded_file" 2>/dev/null | command awk '{ print $2 }')
				__fterm_debug "File is a public key: $id_file"
			else
				builtin set --global --append __fssh_check_messages "E:[ERROR] Host '$host': IdentityFile is not a valid key: $id_file"
				builtin echo "ERROR:identityfile_invalid"
				builtin set has_error 1
				continue
			end
		end

		__fterm_debug "key_fingerprint: $key_fingerprint"

		# Check 7: ssh-agent and key relationship
		if builtin test "$is_public_key" -eq 1
			# Public key specified
			if builtin test "$agent_available" -eq 1
				# ssh-agent available, check if corresponding key is loaded
				if builtin test -n "$key_fingerprint"
					if not builtin string match --quiet "*$key_fingerprint*" "$agent_keys"
						builtin set --global --append __fssh_check_messages "E:[ERROR] Host '$host': Public key specified but not found in ssh-agent"
						builtin set --global --append __fssh_check_messages "E:        IdentityFile: $id_file"
						builtin set --global --append __fssh_check_messages "E:        Fingerprint: $key_fingerprint"
						builtin set --global --append __fssh_check_messages "E:        Run: ssh-add <corresponding-private-key>"
						builtin echo "ERROR:pubkey_not_in_agent"
						builtin set has_error 1
					else
						__fterm_debug "Public key found in agent: $id_file"
					end
				end
			else
				# ssh-agent not available, public key cannot be used
				builtin set --global --append __fssh_check_messages "E:[ERROR] Host '$host': Public key specified but ssh-agent not available"
				builtin set --global --append __fssh_check_messages "E:        IdentityFile: $id_file"
				builtin set --global --append __fssh_check_messages "E:        Cannot authenticate without ssh-agent for public key"
				builtin echo "ERROR:pubkey_no_agent"
				builtin set has_error 1
			end
		else if builtin test "$is_private_key" -eq 1
			# Private key specified - warn about direct usage
			builtin set --global --append __fssh_check_messages "W:[WARN ] Host '$host': Private key directly specified in config"
			builtin set --global --append __fssh_check_messages "W:        IdentityFile: $id_file"
			builtin set --global --append __fssh_check_messages "W:        Consider using public key with ssh-agent for better security"
			builtin echo "WARN:private_key_direct"
			builtin set has_warn 1

			# Check 8: File permissions for private key (should be 600)
			if not builtin set --query MSYSTEM
				# Skip permission check on Windows (MSYS2)
				builtin set --local file_perms (command stat -c '%a' "$test_path" 2>/dev/null)
				__fterm_debug "file_perms: $file_perms"

				if builtin test -n "$file_perms"; and builtin test "$file_perms" != "600"
					builtin set --global --append __fssh_check_messages "W:[WARN ] Host '$host': Private key has insecure permissions: $file_perms"
					builtin set --global --append __fssh_check_messages "W:        IdentityFile: $id_file"
					builtin set --global --append __fssh_check_messages "W:        Recommended: chmod 600 $test_path"
					builtin echo "WARN:insecure_permissions"
					builtin set has_warn 1
				end
			end
		end
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
		__fterm_debug "IdentityFile check passed for: $host"
		return 0
	end
end
