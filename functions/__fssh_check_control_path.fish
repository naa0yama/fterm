#!/usr/bin/env fish

function __fssh_check_control_path --description 'Check ControlPath directory writability'
	# Arguments: [host]
	builtin set --local host "$argv[1]"

	__fterm_debug "=== __fssh_check_control_path called ==="
	__fterm_debug "host: $host"

	builtin set --local config_args (__fterm_get_ssh_config_args)

	# Set HOME for Windows OpenSSH
	builtin set --local original_home "$HOME"
	if builtin set --query MSYSTEM; and builtin set --query USERPROFILE
		builtin set --export HOME "$(command cygpath -m "$USERPROFILE")"
	end

	# Get ControlMaster and ControlPath values
	builtin set --local control_master (__fterm_run_ssh_cmd ssh $config_args -G "$host" 2>/dev/null | command awk '/^controlmaster / { print $2 }')
	builtin set --local control_path (__fterm_run_ssh_cmd ssh $config_args -G "$host" 2>/dev/null | command awk '/^controlpath / { print $2 }')

	# Restore HOME
	if builtin test -n "$original_home"
		builtin set --export HOME "$original_home"
	end

	__fterm_debug "control_master: $control_master"
	__fterm_debug "control_path: $control_path"

	# Skip check if ControlMaster is not enabled
	if builtin test -z "$control_master"; or builtin test "$control_master" = "no"
		__fterm_debug "ControlMaster not enabled, skipping check"
		return 0
	end

	# Skip check if ControlPath is none or not set
	if builtin test -z "$control_path"; or builtin test "$control_path" = "none"
		__fterm_debug "ControlPath not configured, skipping check"
		return 0
	end

	# Extract directory from ControlPath
	# ControlPath often contains placeholders like %C, %h, %p, %r, etc.
	# We need to get the directory part
	builtin set --local control_dir (command dirname "$control_path")

	# Expand ~ to home directory
	builtin set control_dir (builtin string replace '~' "$HOME" "$control_dir")

	# For MSYS2, convert path
	if builtin set --query MSYSTEM
		builtin set control_dir (command cygpath -u "$control_dir" 2>/dev/null)
		if builtin test -z "$control_dir"
			__fterm_debug "Failed to convert ControlPath directory"
			return 0
		end
	end

	__fterm_debug "control_dir: $control_dir"

	# Check if directory exists
	if not builtin test -d "$control_dir"
		builtin set --global --append __fssh_check_messages "W:[WARN ] Host '$host': ControlPath directory does not exist: $control_dir"
		builtin set --global --append __fssh_check_messages "W:        ControlMaster connections may fail"
		builtin echo "WARN:controlpath_dir_missing"
		return 0
	end

	# Check if directory is writable
	if not builtin test -w "$control_dir"
		builtin set --global --append __fssh_check_messages "W:[WARN ] Host '$host': ControlPath directory is not writable: $control_dir"
		builtin set --global --append __fssh_check_messages "W:        ControlMaster connections may fail"
		builtin echo "WARN:controlpath_not_writable"
		return 0
	end

	__fterm_debug "ControlPath check passed for: $host"
	return 0
end
