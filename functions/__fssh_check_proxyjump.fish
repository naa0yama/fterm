#!/usr/bin/env fish

function __fssh_check_proxyjump --description 'Check ProxyJump existence and detect circular references'
	# Arguments: [host] [already_checked_hosts...]
	builtin set --local host "$argv[1]"
	builtin set --local visited_hosts $argv[2..-1]

	__fterm_debug "=== __fssh_check_proxyjump called ==="
	__fterm_debug "host: $host"
	__fterm_debug "visited_hosts: $visited_hosts"

	builtin set --local config_args (__fterm_get_ssh_config_args)

	# Set HOME for Windows OpenSSH
	builtin set --local original_home "$HOME"
	if builtin set --query MSYSTEM; and builtin set --query USERPROFILE
		builtin set --export HOME "$(command cygpath -m "$USERPROFILE")"
	end

	# Get ProxyJump value
	builtin set --local proxyjump (__fterm_run_ssh_cmd ssh $config_args -G "$host" 2>/dev/null | command awk '/^proxyjump / { print $2 }')

	# Restore HOME
	if builtin test -n "$original_home"
		builtin set --export HOME "$original_home"
	end

	__fterm_debug "proxyjump: $proxyjump"

	# No ProxyJump configured or set to none
	if builtin test -z "$proxyjump"; or builtin test "$proxyjump" = "none"
		__fterm_debug "No ProxyJump configured for: $host"
		return 0
	end

	# Handle multiple ProxyJump hosts (comma-separated chain)
	builtin set --local proxy_hosts (builtin string split ',' "$proxyjump")

	for proxy_host in $proxy_hosts
		# Trim whitespace
		builtin set proxy_host (builtin string trim "$proxy_host")

		if builtin test -z "$proxy_host"
			continue
		end

		__fterm_debug "Checking proxy host: $proxy_host"

		# Check 9: Circular reference detection
		if builtin contains "$proxy_host" $visited_hosts
			builtin set --global --append __fssh_check_messages "E:[ERROR] Host '$host': Circular ProxyJump reference detected"
			builtin set --global --append __fssh_check_messages "E:        Chain: $visited_hosts -> $host -> $proxy_host"
			builtin echo "ERROR:circular_proxyjump"
			return 1
		end

		# Add current host to visited list
		builtin set --local new_visited $visited_hosts "$host"

		# Check if ProxyJump host exists in config
		builtin set --local known_hosts (__fssh_get_hosts)

		if not builtin contains "$proxy_host" $known_hosts
			# Check if it might be a direct hostname/IP (not an alias)
			# If it contains @ or looks like an IP/hostname without dots in pattern
			if builtin string match --quiet '*@*' "$proxy_host"
				# user@host format, extract host part
				builtin set --local proxy_hostname (builtin string replace --regex '^[^@]+@' '' "$proxy_host")
				__fterm_debug "ProxyJump uses direct connection format: $proxy_host"
			else if builtin string match --quiet --regex '^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$' "$proxy_host"
				# Direct IP address
				__fterm_debug "ProxyJump uses direct IP: $proxy_host"
			else if not builtin string match --quiet '*.*' "$proxy_host"
				# Single hostname without dots (might be resolvable hostname)
				__fterm_debug "ProxyJump uses simple hostname: $proxy_host"
			else
				# Looks like an alias pattern but not found
				builtin set --global --append __fssh_check_messages "E:[ERROR] Host '$host': ProxyJump host not found in config: $proxy_host"
				builtin echo "ERROR:proxyjump_not_found"
				return 1
			end
		else
			# ProxyJump host exists, verify its configuration
			__fterm_debug "ProxyJump host found in config: $proxy_host"

			# Check basic config for proxy host (Check 3,4 for proxy)
			builtin set --local proxy_basic_result (__fssh_check_basic_config "$proxy_host" 2>&1)
			builtin set --local proxy_basic_errors (builtin echo "$proxy_basic_result" | command grep -c "^ERROR:" || builtin echo 0)

			if builtin string match -qr '[1-9]' -- "$proxy_basic_errors"
				builtin set --global --append __fssh_check_messages "E:[ERROR] Host '$host': ProxyJump host '$proxy_host' has configuration errors"
				builtin echo "ERROR:proxyjump_config_error"
			end

			# Check identity file for proxy host (Check 6,7,8 for proxy)
			builtin set --local proxy_id_result (__fssh_check_identity_file "$proxy_host" 2>&1)
			builtin set --local proxy_id_errors (builtin echo "$proxy_id_result" | command grep -c "^ERROR:" || builtin echo 0)

			if builtin string match -qr '[1-9]' -- "$proxy_id_errors"
				builtin set --global --append __fssh_check_messages "E:[ERROR] Host '$host': ProxyJump host '$proxy_host' has identity file errors"
				builtin echo "ERROR:proxyjump_identity_error"
			end

			# Recursively check ProxyJump chain
			builtin set --local recursive_result (__fssh_check_proxyjump "$proxy_host" $new_visited)
			builtin set --local recursive_errors (builtin echo "$recursive_result" | command grep -c "^ERROR:" || builtin echo 0)

			if builtin string match -qr '[1-9]' -- "$recursive_errors"
				# Errors already reported by recursive call
				builtin echo "$recursive_result" | command grep "^ERROR:"
			end
		end
	end

	__fterm_debug "ProxyJump check passed for: $host"
	return 0
end
