#!/usr/bin/env fish

function __fterm_run_ssh_cmd --description 'Run SSH command with timeout and CR removal for Windows compatibility'
	# Usage: __fterm_run_ssh_cmd <command> [args...]
	# Commands: ssh, ssh-add, ssh-keygen
	# Example: __fterm_run_ssh_cmd ssh -G hostname
	# Example: __fterm_run_ssh_cmd ssh-add -l

	if builtin test (count $argv) -lt 1
		__fterm_debug "__fterm_run_ssh_cmd: No command specified"
		return 1
	end

	builtin set --local cmd_name "$argv[1]"
	builtin set --local cmd_args $argv[2..-1]
	builtin set --local cmd_path ""

	# Determine command path
	switch "$cmd_name"
		case ssh
			if builtin set --query __fssh_ssh_cmd
				builtin set cmd_path "$__fssh_ssh_cmd"
			else
				builtin set cmd_path "ssh"
			end
		case ssh-add
			if builtin set --query __fssh_ssh_add_cmd
				builtin set cmd_path "$__fssh_ssh_add_cmd"
			else
				builtin set cmd_path "ssh-add"
			end
		case ssh-keygen
			if builtin set --query __fssh_ssh_keygen_cmd
				builtin set cmd_path "$__fssh_ssh_keygen_cmd"
			else
				builtin set cmd_path "ssh-keygen"
			end
		case '*'
			__fterm_debug "__fterm_run_ssh_cmd: Unknown command: $cmd_name"
			return 1
	end

	__fterm_debug "__fterm_run_ssh_cmd: cmd_path=$cmd_path, args=$cmd_args"

	# Execute with timeout and remove CR for Windows compatibility
	# timeout is required because gpg-agent can freeze, which would freeze the terminal
	builtin set --local output (command timeout --foreground --kill-after=1 1 "$cmd_path" $cmd_args 2>/dev/null | string replace -a \r '')
	builtin set --local cmd_status $pipestatus[1]

	__fterm_debug "__fterm_run_ssh_cmd: status=$cmd_status"

	if builtin test $cmd_status -ne 0
		return $cmd_status
	end

	# Output result
	for line in $output
		builtin echo "$line"
	end
end
