#!/usr/bin/env fish

function __fssh_check_host_prefix --description 'Check Host prefix matches Default wildcard pattern'
	# Arguments: [host]
	builtin set --local host "$argv[1]"

	__fterm_debug "=== __fssh_check_host_prefix called ==="
	__fterm_debug "host: $host"

	# Extract prefix from host (org.env from org.env.hostname)
	# Pattern: first two dot-separated parts
	builtin set --local parts (builtin string split '.' "$host")
	builtin set --local parts_count (count $parts)

	__fterm_debug "parts: $parts"
	__fterm_debug "parts_count: $parts_count"

	# If less than 3 parts, cannot determine prefix pattern
	if builtin test "$parts_count" -lt 2
		__fterm_debug "Host does not follow org.env.hostname pattern, skipping prefix check"
		return 0
	end

	# Build prefix (all parts except the last one)
	builtin set --local prefix_parts
	for i in (seq 1 (math "$parts_count" - 1))
		builtin set --append prefix_parts "$parts[$i]"
	end
	builtin set --local prefix (builtin string join '.' $prefix_parts)
	builtin set --local wildcard_pattern "$prefix.*"

	__fterm_debug "prefix: $prefix"
	__fterm_debug "wildcard_pattern: $wildcard_pattern"

	# Get SSH home and config files
	builtin set --local ssh_home (__fssh_get_ssh_home)
	builtin set --local config_file "$ssh_home/config"

	if not builtin test -f "$config_file"
		__fterm_debug "config file not found: $config_file"
		return 0
	end

	# Reset visited files tracker
	builtin set --erase __fssh_visited_files

	# Get all included config files
	builtin set --local config_files (__fssh_get_included_files "$config_file" "$ssh_home")

	# Cleanup visited files tracker
	builtin set --erase __fssh_visited_files

	# Find the config file containing this host definition
	builtin set --local host_file ""
	builtin set --local found_wildcard 0

	for file in $config_files
		if not builtin test -f "$file"
			continue
		end

		# Check if this file contains the host definition
		builtin set --local has_host (command grep -E "^Host\s+.*\b$host\b" "$file" 2>/dev/null)
		if builtin test -n "$has_host"
			builtin set host_file "$file"
			__fterm_debug "Found host definition in: $file"
		end

		# Check if this file contains the wildcard pattern
		# Escape dots for regex
		builtin set --local escaped_pattern (builtin string replace --all '.' '\\.' "$wildcard_pattern")
		builtin set --local has_wildcard (command grep -E "^Host\s+.*$escaped_pattern" "$file" 2>/dev/null)
		if builtin test -n "$has_wildcard"
			builtin set found_wildcard 1
			__fterm_debug "Found wildcard pattern '$wildcard_pattern' in: $file"
		end

		# Also check for exact host match in defaults (Host org.env.hostname as default)
		builtin set --local has_exact_default (command grep -E "^Host\s+$host\s*\$" "$file" 2>/dev/null)
		if builtin test -n "$has_exact_default"
			builtin set found_wildcard 1
			__fterm_debug "Found exact host as default in: $file"
		end
	end

	# If we found the host but no matching wildcard pattern
	if builtin test -n "$host_file"; and builtin test "$found_wildcard" -eq 0
		# Check if there's a broader wildcard that could match (e.g., org.*)
		builtin set --local broader_found 0
		for i in (seq 1 (math "$parts_count" - 2))
			builtin set --local broader_prefix_parts
			for j in (seq 1 $i)
				builtin set --append broader_prefix_parts "$parts[$j]"
			end
			builtin set --local broader_prefix (builtin string join '.' $broader_prefix_parts)
			builtin set --local broader_pattern "$broader_prefix.*"

			for file in $config_files
				if builtin test -f "$file"
					builtin set --local escaped_broader (builtin string replace --all '.' '\\.' "$broader_pattern")
					if command grep -qE "^Host\s+.*$escaped_broader" "$file" 2>/dev/null
						builtin set broader_found 1
						__fterm_debug "Found broader wildcard pattern '$broader_pattern' in: $file"
						break
					end
				end
			end
			if builtin test "$broader_found" -eq 1
				break
			end
		end

		# Also check for Host * (global default)
		if builtin test "$broader_found" -eq 0
			for file in $config_files
				if builtin test -f "$file"
					if command grep -qE '^Host\s+\*\s*$' "$file" 2>/dev/null
						builtin set broader_found 1
						__fterm_debug "Found global wildcard 'Host *' in: $file"
						break
					end
				end
			end
		end

		if builtin test "$broader_found" -eq 0
			builtin set --global --append __fssh_check_messages "E:[ERROR] Host '$host': No matching default pattern found"
			builtin set --global --append __fssh_check_messages "E:        Expected: Host $wildcard_pattern"
			builtin set --global --append __fssh_check_messages "E:        In file: $host_file"
			return 1
		end
	end

	__fterm_debug "Host prefix check passed for: $host"
	return 0
end
