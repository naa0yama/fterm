# Fish → Rust Migration Review Findings

## Phase 1: Critical (affects SSH connection correctness)

### P1-01: Missing `-F` config args in SSH/SCP execution

- **Files**: `src/command/ssh.rs`, `src/command/scp.rs`
- **Issue**: `exec_ssh_status()` / `exec_scp_status()` do not prepend `-F` config args. When `FSSH_SSH_CONF_DIR` is set, SSH/SCP uses wrong config.
- **Fix**: Prepend `build_config_args()` result to the command args in `exec_ssh_status`, `exec_ssh`, `exec_scp_status`, `exec_scp`.
- **Status**: DONE

### P1-02: `ssh -G` output values lowercased

- **Files**: `src/config/connection.rs`
- **Issue**: `line.to_lowercase()` lowercases both key and value. Username `Deploy` becomes `deploy`, causing auth failure.
- **Fix**: Only lowercase the key, not the value.
- **Status**: DONE

### P1-03: Command timeout not implemented

- **Files**: `src/external.rs`
- **Issue**: `_timeout_secs` parameter is ignored. `ssh-add -l` or `ssh-keygen` freeze hangs the process.
- **Fix**: Implement timeout using polling loop with `try_wait()`.
- **Status**: DONE

### P1-04: Include directive multiple path split

- **Files**: `src/config/include.rs`
- **Issue**: `Include path1 path2` treated as single path. Config files are missed.
- **Fix**: Split pattern string by whitespace and process each pattern individually.
- **Status**: DONE

### P1-05: Include `~` expansion ignores MSYS2

- **Files**: `src/config/include.rs`
- **Issue**: Uses `std::env::var("HOME")` directly instead of `resolve_home()` for `~` expansion.
- **Fix**: Use `path::resolve_home()` instead.
- **Status**: DONE

## Phase 2: High (data loss, MSYS2 compat)

### P2-01: fgen overwrites without confirmation

- **Files**: `src/command/fgen.rs`
- **Issue**: Existing config files overwritten without prompt. Fish version asks `Overwrite? [y/N]:`.
- **Fix**: Check file existence and prompt for confirmation before writing.
- **Status**: DONE

### P2-02: fgen continues after template creation

- **Files**: `src/command/fgen.rs`
- **Issue**: After creating default template, proceeds to generate config. Fish version exits with "edit template first" message.
- **Fix**: Return early after template creation with instructional message.
- **Status**: DONE

### P2-03: SSH_ENV environment file not loaded

- **Files**: `src/command/ssh.rs`, `src/command/scp.rs`, `src/util/ssh_env.rs`
- **Issue**: Fish version sources `$SSH_ENV` file before agent check. Rust version skips this entirely.
- **Fix**: Added `ssh_env::load()` that parses `KEY=VALUE` lines from the `SSH_ENV` file.
- **Status**: DONE

### P2-04: MSYS2 HOME switch missing for SSH/SCP exec

- **Files**: `src/command/ssh.rs`, `src/command/scp.rs`, `src/util/path.rs`
- **Issue**: Fish version temporarily sets `HOME` to `cygpath -m "$USERPROFILE"` during SSH/SCP execution.
- **Fix**: Added `path::msys2_home()` and `.env("HOME", ...)` on the Command when MSYS2 is detected.
- **Status**: DONE

### P2-05: SCP missing pre-connect splash banner

- **Files**: `src/command/scp.rs`, `src/util/splash.rs`
- **Issue**: No banner before SCP execution showing SSH config details and agent keys.
- **Fix**: Added `scp_connect_banner()` and call it in `setup_scp_session`.
- **Status**: DONE

### P2-06: Identity validation lacks private/public key distinction

- **Files**: `src/validate/identity.rs`
- **Issue**: Does not distinguish private vs public keys. Public key without agent = certain auth failure, should be ERROR not WARN.
- **Fix**: Check `.pub` extension; use `CheckLevel::Error` for public keys not in agent.
- **Status**: DONE

### P2-07: ControlPath writable check inaccurate

- **Files**: `src/validate/control_path.rs`
- **Issue**: `readonly()` checks mode bits, not effective user access. Should use OS-level access check.
- **Fix**: Use `nix::unistd::access` with `W_OK` on Unix.
- **Status**: DONE

## Phase 3: Medium (UX compatibility)

### P3-01: Pane title not saved/restored

- **Files**: `src/command/ssh.rs`, `src/command/scp.rs`, `src/tmux/pane.rs`
- **Fix**: Added `pane::get_title()`, save original before overwriting, restore on teardown.
- **Status**: DONE

### P3-02: No agent key caching

- **Files**: `src/command/ssh.rs`
- **Status**: WONTFIX (agent key lookup is fast enough; caching adds complexity for marginal gain)

### P3-03: Disconnect banner missing log file path

- **Files**: `src/util/splash.rs`
- **Status**: DONE

### P3-04: Connect banner missing Config Name and Command args

- **Files**: `src/util/splash.rs`, `src/command/ssh.rs`
- **Fix**: Added `config_name` parameter to `ssh_connect_banner()`, displayed as "Config:" field.
- **Status**: DONE

### P3-05: SCP result banner missing timestamp/duration/log path

- **Files**: `src/util/splash.rs`, `src/command/scp.rs`
- **Status**: DONE

### P3-06: SCP log path missing tmux identifiers

- **Files**: `src/command/scp.rs`
- **Fix**: Added `get_tmux_identifiers()` to SCP, matching SSH log path format.
- **Status**: DONE

### P3-07: SCP teardown missing window::enable_rename

- **Files**: `src/command/scp.rs`
- **Status**: DONE

### P3-08: SCP missing user@host format construction

- **Files**: `src/command/scp.rs`
- **Status**: WONTFIX (SCP correctly joins hosts with underscore; user@host is already in log path)

### P3-09: fgen no retry on empty input

- **Files**: `src/command/fgen.rs`
- **Fix**: Added `prompt_required()` that retries up to 3 times on empty input.
- **Status**: DONE

### P3-10: fgen prompt missing input examples

- **Files**: `src/command/fgen.rs`
- **Status**: DONE

### P3-11: flog sort inconsistency (mtime vs lexical)

- **Files**: `src/command/flog.rs`, `src/util/files.rs`
- **Status**: WONTFIX (both sort newest first; filenames contain timestamps so lexical reverse = mtime order)

### P3-12: flog no error on missing env var

- **Files**: `src/command/flog.rs`, `src/util/log_dir.rs`
- **Status**: WONTFIX (graceful fallback to default is intentional; not a bug)

### P3-13: fssh exact mode default is false (should be true)

- **Files**: `src/command/fssh.rs`
- **Status**: DONE

### P3-14: FSSH_SSH_CONF_DIR semantics differ

- **Files**: `src/config/home.rs`
- **Status**: WONTFIX (correctly implemented - env var overrides default ~/.ssh)

### P3-15: Log header missing section headings

- **Files**: `src/logging/start.rs`
- **Status**: DONE

### P3-16: Pane logging state not set via tmux option

- **Files**: `src/logging/start.rs`, `src/logging/stop.rs`
- **Fix**: Set `@fterm_logging` pane option in start, unset in stop.
- **Status**: DONE

### P3-17: @fterm_ssh_count not unset when 0

- **Files**: `src/tmux/window.rs`
- **Status**: DONE

### P3-18: Terminal title not reset after tmux detach

- **Files**: `src/command/ssh.rs`, `src/command/scp.rs`
- **Fix**: Added `\x1b]0;\x07` escape sequence in teardown to reset terminal title.
- **Status**: DONE

## Phase 4: Minor (cosmetic, full parity)

### P4-01: Bash completions missing SSH/SCP option completions

### P4-02: SCP host completion missing colon suffix

### P4-03: Bash fssh/flog stderr not suppressed

### P4-04: ProxyJump duplicate check cache missing

### P4-05: Host directive case only handles Host/host

### P4-06: CM dir creation success message missing

### P4-07: fgen output to stderr (Fish uses stdout)

### P4-08: fgen placeholder match precision (org.dev vs org.dev.)

### P4-09: @fterm_ssh_host value format differs

### P4-10: flog auto-creates directory (Fish errors)

### P4-11: Empty details/keys still writes separator line

### P4-12: logging stop missing file existence pre-check
