#!/usr/bin/env fish

function __fssh_check_cm_dir --description 'Check and create ControlMaster directory'
	__fterm_debug "=== __fssh_check_cm_dir called ==="

	# Get SSH home directory
	builtin set --local ssh_home (__fssh_get_ssh_home)
	builtin set --local cm_dir "$ssh_home/conf.d/cm"

	__fterm_debug "cm_dir: $cm_dir"

	if builtin test -d "$cm_dir"
		__fterm_debug "ControlMaster directory exists: $cm_dir"
		return 0
	end

	# Directory does not exist, create it
	builtin set --global --append __fssh_check_messages "W:[WARN ] ControlMaster directory not found: $cm_dir"
	builtin set --global --append __fssh_check_messages "W:[WARN ] Creating directory..."

	if command mkdir -p "$cm_dir" 2>/dev/null
		# Set proper permissions (700 for directory containing sockets)
		command chmod 700 "$cm_dir" 2>/dev/null
		builtin set --global --append __fssh_check_messages "W:[OK   ] Created ControlMaster directory: $cm_dir"
		__fterm_debug "Created ControlMaster directory: $cm_dir"
		builtin echo "created"
		return 0
	else
		builtin set --global --append __fssh_check_messages "E:[ERROR] Failed to create ControlMaster directory: $cm_dir"
		__fterm_debug "Failed to create ControlMaster directory: $cm_dir"
		builtin echo "error"
		return 1
	end
end
