//! External command execution abstraction.
//!
//! Provides a trait-based interface for running external commands (ssh, tmux,
//! fzf, etc.), enabling mock injection in tests.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tracing::debug;

/// Resolve the path for an SSH-related command.
///
/// On MSYS2, searches known Windows OpenSSH directories for `{name}.exe`.
/// Otherwise returns the bare command name for PATH lookup.
pub(crate) fn resolve_ssh_command(name: &str) -> String {
    crate::util::path::resolve_win_ssh_command(name).unwrap_or_else(|| String::from(name))
}

/// Output from an external command execution.
#[derive(Debug, Clone)]
pub struct CommandOutput {
    /// Process exit code (0 = success).
    pub exit_code: i32,
    /// Captured stdout content.
    pub stdout: String,
    /// Captured stderr content.
    pub stderr: String,
}

/// Result of `ssh-add -l` listing agent keys.
#[derive(Debug, Clone)]
pub struct AgentListResult {
    /// Whether the agent is available.
    pub available: bool,
    /// Raw output lines from `ssh-add -l`.
    pub keys: Vec<String>,
}

/// Abstraction for external command execution.
///
/// Implementations can run real commands or return mock responses for testing.
pub trait CommandRunner {
    /// Run a command with a timeout.
    ///
    /// # Errors
    /// Returns an error if the command cannot be spawned or times out.
    fn run(&self, cmd: &str, args: &[&str], timeout_secs: u64) -> Result<CommandOutput>;

    /// Resolve SSH config for a host via `ssh -G`.
    ///
    /// # Errors
    /// Returns an error if `ssh -G` fails.
    fn ssh_resolve(&self, host: &str, config_args: &[String]) -> Result<String>;

    /// Run a command interactively, inheriting stdin/stdout/stderr.
    ///
    /// Unlike [`run`](CommandRunner::run), this does **not** capture output.
    /// Use this for commands that need terminal access (e.g. `tmux attach`).
    ///
    /// The default implementation delegates to [`run`](CommandRunner::run) and
    /// returns the exit code, which is suitable for non-interactive contexts
    /// (e.g. tests).
    ///
    /// # Errors
    /// Returns an error if the command cannot be spawned.
    fn run_interactive(&self, cmd: &str, args: &[&str]) -> Result<i32> {
        self.run(cmd, args, 0).map(|output| output.exit_code)
    }

    /// List agent keys via `ssh-add -l`.
    ///
    /// # Errors
    /// Returns an error if the agent is unreachable.
    fn ssh_agent_list(&self) -> Result<AgentListResult>;

    /// Get key fingerprint via `ssh-keygen -lf`.
    ///
    /// # Errors
    /// Returns an error if the key file is invalid.
    fn ssh_keygen_fingerprint(&self, path: &Path) -> Result<String>;
}

/// Execute an SSH-family command with `-F` config arguments prepended.
///
/// Builds the full argument list (`-F cfg1 -F cfg2 … user_args…`), runs the
/// command via `std::process::Command::status()`, and returns the exit code.
/// Used for interactive sessions (SSH, SCP) that may run for a long time.
pub fn exec_with_config(command_name: &str, args: &[String], config_args: &[String]) -> i32 {
    let cmd = resolve_ssh_command(command_name);
    let mut full_args: Vec<&str> = Vec::new();
    for cfg in config_args {
        full_args.push("-F");
        full_args.push(cfg.as_str());
    }
    for a in args {
        full_args.push(a.as_str());
    }
    let mut command = std::process::Command::new(&cmd);
    command.args(&full_args);
    // MSYS2: set HOME to Windows mixed path so Include resolves correctly
    if let Some(home) = crate::util::path::msys2_home() {
        command.env("HOME", &home);
    }
    match command.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => {
            tracing::error!("failed to execute {command_name}: {e:#}");
            1
        }
    }
}

/// Execute a command as a passthrough, replacing the process on Unix.
///
/// On Unix, uses `exec()` to replace the current process (never returns on
/// success). On non-Unix, spawns a child process and returns the exit code.
///
/// # Errors
///
/// Returns an error if the command cannot be spawned.
pub fn exec_passthrough(cmd: &str, args: &[&str]) -> Result<i32> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        let err = Command::new(cmd).args(args).exec();
        // exec() only returns on error
        Err(err).with_context(|| format!("failed to exec {cmd}"))
    }

    #[cfg(not(unix))]
    {
        let status = Command::new(cmd)
            .args(args)
            .status()
            .with_context(|| format!("failed to execute {cmd}"))?;
        Ok(status.code().unwrap_or(1))
    }
}

// ---------------------------------------------------------------------------
// RealCommandRunner
// ---------------------------------------------------------------------------

/// Production implementation that executes real OS commands.
#[derive(Debug, Default)]
pub struct RealCommandRunner;

impl RealCommandRunner {
    /// Create a new `RealCommandRunner`.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl CommandRunner for RealCommandRunner {
    /// Run an external command, capturing stdout and stderr.
    ///
    /// Non-zero exit codes are **not** treated as errors – the caller decides
    /// how to interpret the exit code via [`CommandOutput`].
    ///
    /// # Errors
    /// Returns an error only when the command cannot be spawned.
    fn run(&self, cmd: &str, args: &[&str], timeout_secs: u64) -> Result<CommandOutput> {
        debug!(
            command = cmd,
            ?args,
            timeout_secs,
            "spawning external command"
        );

        if timeout_secs > 0 {
            let mut child = Command::new(cmd)
                .args(args)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .with_context(|| format!("failed to spawn command: {cmd}"))?;

            #[allow(clippy::arithmetic_side_effects)]
            let deadline = Instant::now() + Duration::from_secs(timeout_secs);
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        let stdout_bytes = child.stdout.take().map_or_else(Vec::new, |mut r| {
                            let mut buf = Vec::new();
                            std::io::Read::read_to_end(&mut r, &mut buf).unwrap_or(0);
                            buf
                        });
                        let stderr_bytes = child.stderr.take().map_or_else(Vec::new, |mut r| {
                            let mut buf = Vec::new();
                            std::io::Read::read_to_end(&mut r, &mut buf).unwrap_or(0);
                            buf
                        });
                        let stdout = String::from_utf8_lossy(&stdout_bytes).replace('\r', "");
                        let stderr = String::from_utf8_lossy(&stderr_bytes).replace('\r', "");
                        let exit_code = status.code().unwrap_or(-1);
                        debug!(command = cmd, exit_code, "command finished");
                        return Ok(CommandOutput {
                            exit_code,
                            stdout,
                            stderr,
                        });
                    }
                    Ok(None) => {
                        if Instant::now() >= deadline {
                            let _ = child.kill();
                            let _ = child.wait();
                            anyhow::bail!("command timed out after {timeout_secs}s: {cmd}");
                        }
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    Err(e) => {
                        return Err(e)
                            .with_context(|| format!("failed to wait for command: {cmd}"));
                    }
                }
            }
        }

        let output = Command::new(cmd)
            .args(args)
            .output()
            .with_context(|| format!("failed to spawn command: {cmd}"))?;

        // Strip \r for Windows compatibility.
        let stdout = String::from_utf8_lossy(&output.stdout).replace('\r', "");
        let stderr = String::from_utf8_lossy(&output.stderr).replace('\r', "");

        let exit_code = output.status.code().unwrap_or(-1);
        debug!(command = cmd, exit_code, "command finished");

        Ok(CommandOutput {
            exit_code,
            stdout,
            stderr,
        })
    }

    /// Run a command interactively with inherited stdio.
    ///
    /// # Errors
    /// Returns an error if the command cannot be spawned.
    fn run_interactive(&self, cmd: &str, args: &[&str]) -> Result<i32> {
        debug!(command = cmd, ?args, "spawning interactive command");

        let status = Command::new(cmd)
            .args(args)
            .status()
            .with_context(|| format!("failed to spawn interactive command: {cmd}"))?;

        let exit_code = status.code().unwrap_or(-1);
        debug!(command = cmd, exit_code, "interactive command finished");

        Ok(exit_code)
    }

    /// Resolve SSH configuration for `host` by running `ssh -G`.
    ///
    /// # Errors
    /// Returns an error if `ssh -G` cannot be spawned or exits with non-zero.
    fn ssh_resolve(&self, host: &str, config_args: &[String]) -> Result<String> {
        let mut args: Vec<&str> = Vec::new();
        for arg in config_args {
            args.push("-F");
            args.push(arg.as_str());
        }
        args.push("-G");
        args.push(host);

        let ssh_cmd = resolve_ssh_command("ssh");
        let result = self
            .run(&ssh_cmd, &args, 10)
            .with_context(|| format!("ssh_resolve failed for host: {host}"))?;

        if result.exit_code != 0 {
            anyhow::bail!(
                "ssh -G {host} exited with code {}: {}",
                result.exit_code,
                result.stderr.trim()
            );
        }

        Ok(result.stdout)
    }

    /// List keys held by the SSH agent.
    ///
    /// # Errors
    /// Returns an error if `ssh-add` cannot be spawned.
    fn ssh_agent_list(&self) -> Result<AgentListResult> {
        let ssh_add_cmd = resolve_ssh_command("ssh-add");
        let result = self
            .run(&ssh_add_cmd, &["-l"], 5)
            .context("failed to list SSH agent keys")?;

        if result.exit_code == 0 {
            let keys = result
                .stdout
                .lines()
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect();
            Ok(AgentListResult {
                available: true,
                keys,
            })
        } else {
            Ok(AgentListResult {
                available: false,
                keys: Vec::new(),
            })
        }
    }

    /// Obtain the fingerprint of an SSH key file.
    ///
    /// # Errors
    /// Returns an error if `ssh-keygen` cannot be spawned or fails.
    fn ssh_keygen_fingerprint(&self, path: &Path) -> Result<String> {
        let path_str = path.to_str().context("key path contains invalid UTF-8")?;

        let ssh_keygen_cmd = resolve_ssh_command("ssh-keygen");
        let result = self
            .run(&ssh_keygen_cmd, &["-lf", path_str], 5)
            .with_context(|| format!("ssh_keygen_fingerprint failed for: {}", path.display()))?;

        if result.exit_code != 0 {
            anyhow::bail!(
                "ssh-keygen -lf {} exited with code {}: {}",
                path.display(),
                result.exit_code,
                result.stderr.trim()
            );
        }

        Ok(result.stdout.trim().to_owned())
    }
}

// ---------------------------------------------------------------------------
// MockCommandRunner (test only)
// ---------------------------------------------------------------------------

/// Mock implementation of [`CommandRunner`] for unit tests.
///
/// Allows pre-registering responses keyed by command string. Unregistered
/// commands return a default success response.
#[cfg(test)]
#[derive(Debug, Default)]
pub struct MockCommandRunner {
    /// Keyed by `"cmd arg1 arg2 …"`.
    run_responses: std::sync::Mutex<std::collections::HashMap<String, CommandOutput>>,
    /// Keyed by `"cmd arg1 arg2 …"` — exit code only.
    interactive_responses: std::sync::Mutex<std::collections::HashMap<String, i32>>,
    /// Keyed by host name. `Err(msg)` simulates a failed resolve.
    ssh_resolve_responses:
        std::sync::Mutex<std::collections::HashMap<String, std::result::Result<String, String>>>,
    /// Single response for `ssh_agent_list`.
    agent_list_response: std::sync::Mutex<Option<AgentListResult>>,
    /// Keyed by path string. `Err(msg)` simulates a failed fingerprint.
    fingerprint_responses:
        std::sync::Mutex<std::collections::HashMap<String, std::result::Result<String, String>>>,
}

#[cfg(test)]
impl MockCommandRunner {
    /// Create an empty mock.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a response for a specific command invocation.
    ///
    /// # Panics
    /// Panics if the internal mutex is poisoned.
    #[must_use]
    pub fn with_run_response(self, key: &str, output: CommandOutput) -> Self {
        #[allow(clippy::unwrap_used)]
        self.run_responses
            .lock()
            .unwrap()
            .insert(String::from(key), output);
        self
    }

    /// Register a response for an interactive command invocation.
    ///
    /// # Panics
    /// Panics if the internal mutex is poisoned.
    #[must_use]
    pub fn with_interactive_response(self, key: &str, exit_code: i32) -> Self {
        #[allow(clippy::unwrap_used)]
        self.interactive_responses
            .lock()
            .unwrap()
            .insert(String::from(key), exit_code);
        self
    }

    /// Register a successful response for `ssh_resolve`.
    ///
    /// # Panics
    /// Panics if the internal mutex is poisoned.
    #[must_use]
    pub fn with_ssh_resolve(self, host: &str, output: &str) -> Self {
        #[allow(clippy::unwrap_used)]
        self.ssh_resolve_responses
            .lock()
            .unwrap()
            .insert(String::from(host), Ok(String::from(output)));
        self
    }

    /// Register an error response for `ssh_resolve`.
    ///
    /// # Panics
    /// Panics if the internal mutex is poisoned.
    #[must_use]
    pub fn with_ssh_resolve_error(self, host: &str, error_msg: &str) -> Self {
        #[allow(clippy::unwrap_used)]
        self.ssh_resolve_responses
            .lock()
            .unwrap()
            .insert(String::from(host), Err(String::from(error_msg)));
        self
    }

    /// Register a response for `ssh_agent_list`.
    ///
    /// # Panics
    /// Panics if the internal mutex is poisoned.
    #[must_use]
    pub fn with_agent_list(self, result: AgentListResult) -> Self {
        #[allow(clippy::unwrap_used)]
        {
            *self.agent_list_response.lock().unwrap() = Some(result);
        }
        self
    }

    /// Register a successful response for `ssh_keygen_fingerprint`.
    ///
    /// # Panics
    /// Panics if the internal mutex is poisoned.
    #[must_use]
    pub fn with_fingerprint(self, path: &str, fingerprint: &str) -> Self {
        #[allow(clippy::unwrap_used)]
        self.fingerprint_responses
            .lock()
            .unwrap()
            .insert(String::from(path), Ok(String::from(fingerprint)));
        self
    }

    /// Register an error response for `ssh_keygen_fingerprint`.
    ///
    /// # Panics
    /// Panics if the internal mutex is poisoned.
    #[must_use]
    pub fn with_fingerprint_error(self, path: &str, error_msg: &str) -> Self {
        #[allow(clippy::unwrap_used)]
        self.fingerprint_responses
            .lock()
            .unwrap()
            .insert(String::from(path), Err(String::from(error_msg)));
        self
    }
}

#[cfg(test)]
impl MockCommandRunner {
    /// Build the lookup key from a command and its arguments.
    fn command_key(cmd: &str, args: &[&str]) -> String {
        if args.is_empty() {
            String::from(cmd)
        } else {
            format!("{cmd} {}", args.join(" "))
        }
    }
}

#[cfg(test)]
impl CommandRunner for MockCommandRunner {
    fn run(&self, cmd: &str, args: &[&str], _timeout_secs: u64) -> Result<CommandOutput> {
        let key = Self::command_key(cmd, args);
        #[allow(clippy::unwrap_used)]
        let guard = self.run_responses.lock().unwrap();
        Ok(guard.get(&key).cloned().unwrap_or(CommandOutput {
            exit_code: 0,
            stdout: String::new(),
            stderr: String::new(),
        }))
    }

    fn run_interactive(&self, cmd: &str, args: &[&str]) -> Result<i32> {
        let key = Self::command_key(cmd, args);
        #[allow(clippy::unwrap_used)]
        let guard = self.interactive_responses.lock().unwrap();
        Ok(guard.get(&key).copied().unwrap_or(0))
    }

    fn ssh_resolve(&self, host: &str, _config_args: &[String]) -> Result<String> {
        #[allow(clippy::unwrap_used)]
        let guard = self.ssh_resolve_responses.lock().unwrap();
        match guard.get(host) {
            Some(Ok(s)) => Ok(s.clone()),
            Some(Err(msg)) => anyhow::bail!("{msg}"),
            None => Ok(String::new()),
        }
    }

    fn ssh_agent_list(&self) -> Result<AgentListResult> {
        #[allow(clippy::unwrap_used)]
        let guard = self.agent_list_response.lock().unwrap();
        Ok(guard.clone().unwrap_or(AgentListResult {
            available: false,
            keys: Vec::new(),
        }))
    }

    fn ssh_keygen_fingerprint(&self, path: &Path) -> Result<String> {
        let key = path.to_string_lossy().to_string();
        #[allow(clippy::unwrap_used)]
        let guard = self.fingerprint_responses.lock().unwrap();
        match guard.get(&key) {
            Some(Ok(s)) => Ok(s.clone()),
            Some(Err(msg)) => anyhow::bail!("{msg}"),
            None => Ok(String::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn mock_run_returns_registered_response() {
        // Arrange
        let runner = MockCommandRunner::new().with_run_response(
            "echo hello",
            CommandOutput {
                exit_code: 0,
                stdout: String::from("hello\n"),
                stderr: String::new(),
            },
        );

        // Act
        let out = runner.run("echo", &["hello"], 5).unwrap();

        // Assert
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout, "hello\n");
    }

    #[test]
    fn mock_run_returns_default_for_unknown_command() {
        // Arrange
        let runner = MockCommandRunner::new();

        // Act
        let out = runner.run("unknown", &[], 5).unwrap();

        // Assert
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.is_empty());
    }

    #[test]
    fn mock_ssh_resolve_returns_registered_output() {
        // Arrange
        let runner =
            MockCommandRunner::new().with_ssh_resolve("myhost", "hostname myhost\nport 22\n");

        // Act
        let result = runner.ssh_resolve("myhost", &[]).unwrap();

        // Assert
        assert!(result.contains("hostname myhost"));
    }

    #[test]
    fn mock_agent_list_returns_registered_result() {
        // Arrange
        let runner = MockCommandRunner::new().with_agent_list(AgentListResult {
            available: true,
            keys: vec![String::from("SHA256:abc123 user@host (RSA)")],
        });

        // Act
        let result = runner.ssh_agent_list().unwrap();

        // Assert
        assert!(result.available);
        assert_eq!(result.keys.len(), 1);
    }

    #[test]
    fn mock_fingerprint_returns_registered_value() {
        // Arrange
        let runner =
            MockCommandRunner::new().with_fingerprint("/home/user/.ssh/id_rsa", "SHA256:abc123");

        // Act
        let fp = runner
            .ssh_keygen_fingerprint(Path::new("/home/user/.ssh/id_rsa"))
            .unwrap();

        // Assert
        assert_eq!(fp, "SHA256:abc123");
    }

    #[test]
    fn mock_run_builds_key_correctly_with_args() {
        // Arrange
        let runner = MockCommandRunner::new().with_run_response(
            "git commit -m msg",
            CommandOutput {
                exit_code: 0,
                stdout: String::from("committed"),
                stderr: String::new(),
            },
        );

        // Act
        let out = runner.run("git", &["commit", "-m", "msg"], 10).unwrap();

        // Assert – key is "cmd arg1 arg2 …" joined by spaces
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout, "committed");
    }

    #[test]
    fn mock_ssh_resolve_returns_empty_for_unknown_host() {
        // Arrange
        let runner = MockCommandRunner::new();

        // Act
        let result = runner.ssh_resolve("no-such-host", &[]).unwrap();

        // Assert – default is empty string
        assert!(result.is_empty());
    }

    #[test]
    fn mock_agent_list_returns_unavailable_by_default() {
        // Arrange
        let runner = MockCommandRunner::new();

        // Act
        let result = runner.ssh_agent_list().unwrap();

        // Assert – without registration, agent is unavailable with no keys
        assert!(!result.available);
        assert!(result.keys.is_empty());
    }

    #[test]
    fn mock_fingerprint_returns_empty_for_unknown_path() {
        // Arrange
        let runner = MockCommandRunner::new();

        // Act
        let fp = runner
            .ssh_keygen_fingerprint(Path::new("/nonexistent/key"))
            .unwrap();

        // Assert – default is empty string
        assert!(fp.is_empty());
    }

    #[test]
    fn mock_run_returns_default_for_command_with_no_args() {
        // Arrange
        let runner = MockCommandRunner::new().with_run_response(
            "ls -la",
            CommandOutput {
                exit_code: 0,
                stdout: String::from("files"),
                stderr: String::new(),
            },
        );

        // Act – call with no args; key is just "whoami" (no match for "ls -la")
        let out = runner.run("whoami", &[], 5).unwrap();

        // Assert – falls back to default empty response
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.is_empty());
        assert!(out.stderr.is_empty());
    }

    // -----------------------------------------------------------------------
    // RealCommandRunner tests
    // -----------------------------------------------------------------------

    #[test]
    fn real_runner_new_creates_instance() {
        // Arrange & Act
        let runner = RealCommandRunner::new();

        // Assert – just verify construction succeeds and Debug is implemented
        let debug_str = format!("{runner:?}");
        assert_eq!(debug_str, "RealCommandRunner");
    }

    #[test]
    fn real_runner_run_returns_success_for_true() {
        // Arrange
        let runner = RealCommandRunner::new();

        // Act
        let result = runner.run("true", &[], 5).unwrap();

        // Assert
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn real_runner_run_returns_nonzero_for_false() {
        // Arrange
        let runner = RealCommandRunner::new();

        // Act
        let result = runner.run("false", &[], 5).unwrap();

        // Assert
        assert_ne!(result.exit_code, 0);
    }

    #[test]
    fn real_runner_run_captures_stdout() {
        // Arrange
        let runner = RealCommandRunner::new();

        // Act
        let result = runner.run("echo", &["hello"], 5).unwrap();

        // Assert
        assert_eq!(result.stdout, "hello\n");
    }

    #[test]
    fn real_runner_run_returns_error_for_nonexistent_command() {
        // Arrange
        let runner = RealCommandRunner::new();

        // Act
        let result = runner.run("this_command_does_not_exist_xyz", &[], 5);

        // Assert
        assert!(result.is_err());
    }

    #[test]
    fn real_runner_run_interactive_returns_success_for_true() {
        // Arrange
        let runner = RealCommandRunner::new();

        // Act
        let exit_code = runner.run_interactive("true", &[]).unwrap();

        // Assert
        assert_eq!(exit_code, 0);
    }

    #[test]
    fn real_runner_run_interactive_returns_nonzero_for_false() {
        // Arrange
        let runner = RealCommandRunner::new();

        // Act
        let exit_code = runner.run_interactive("false", &[]).unwrap();

        // Assert
        assert_ne!(exit_code, 0);
    }

    #[test]
    fn real_runner_ssh_agent_list_returns_result() {
        // Arrange
        let runner = RealCommandRunner::new();

        // Act
        let result = runner.ssh_agent_list();

        // Assert — succeeds even if no agent is running (returns unavailable)
        assert!(result.is_ok());
    }

    #[test]
    fn real_runner_ssh_keygen_fingerprint_returns_error_for_nonexistent_file() {
        // Arrange
        let runner = RealCommandRunner::new();

        // Act
        let result = runner.ssh_keygen_fingerprint(Path::new("/nonexistent/key"));

        // Assert — should fail for a non-existent file
        assert!(result.is_err());
    }

    #[test]
    fn real_runner_ssh_resolve_returns_result_for_localhost() {
        // Arrange
        let runner = RealCommandRunner::new();

        // Act — just verify it does not panic
        let _result = runner.ssh_resolve("localhost", &[]);
    }
}
