#!/usr/bin/env fish

function __fssh_check_hosts_duplicate --description 'Check for duplicate Host definitions'
	__fterm_debug "=== __fssh_check_hosts_duplicate called ==="

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

	__fterm_debug "config_files: $config_files"

	# Collect all host definitions with their source files
	builtin set --local all_hosts
	builtin set --local host_sources

	for file in $config_files
		if not builtin test -f "$file"
			continue
		end

		while builtin read --local line
			# Match Host directive (must start at beginning of line)
			if builtin string match --quiet --regex '^Host\s+' "$line"
				# Extract hosts after "Host"
				builtin set --local hosts_str (builtin string replace --regex '^Host\s+' '' "$line")
				# Split by whitespace
				for host in (builtin string split --no-empty ' ' "$hosts_str")
					# Skip wildcards
					if builtin string match --quiet --regex '[*?]' "$host"
						continue
					end
					builtin set --append all_hosts "$host"
					builtin set --append host_sources "$file"
				end
			end
		end <"$file"
	end

	__fterm_debug "all_hosts count: "(count $all_hosts)

	# Find duplicates
	builtin set --local seen_hosts
	builtin set --local seen_files
	builtin set --local duplicates

	for i in (seq 1 (count $all_hosts))
		builtin set --local host "$all_hosts[$i]"
		builtin set --local source "$host_sources[$i]"

		# Check if we've seen this host before
		builtin set --local idx 0
		for j in (seq 1 (count $seen_hosts))
			if builtin test "$seen_hosts[$j]" = "$host"
				builtin set idx $j
				break
			end
		end

		if builtin test "$idx" -gt 0
			# Duplicate found
			if not builtin contains "$host" $duplicates
				builtin set --append duplicates "$host"
				builtin set --global --append __fssh_check_messages "W:[WARN ] Duplicate Host definition: $host"
				builtin set --global --append __fssh_check_messages "W:        First defined in: $seen_files[$idx]"
				builtin set --global --append __fssh_check_messages "W:        Also defined in: $source"
				builtin echo "WARN:duplicate_host:$host"
			else
				# Already reported, just note additional occurrence
				builtin set --global --append __fssh_check_messages "W:        Also defined in: $source"
			end
		else
			builtin set --append seen_hosts "$host"
			builtin set --append seen_files "$source"
		end
	end

	if builtin test (count $duplicates) -gt 0
		__fterm_debug "Found "(count $duplicates)" duplicate host(s)"
	else
		__fterm_debug "No duplicate hosts found"
	end

	# Return list of duplicates for counting
	for dup in $duplicates
		builtin echo "$dup"
	end
end
