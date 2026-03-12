//! Stop session logging and compress the log file.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::Local;
use tracing::{debug, warn};

use crate::external::CommandRunner;

/// Stop session logging, append a disconnect marker, and compress the log.
///
/// Disables the tmux pipe-pane, appends a timestamped disconnect marker to
/// the log file, then compresses it with gzip.
///
/// # Errors
/// Returns an error if the tmux command, file I/O, or gzip compression fails.
pub fn stop(runner: &dyn CommandRunner, log_path: &Path) -> Result<()> {
    debug!(path = %log_path.display(), "stopping tmux pipe-pane");

    // Disable tmux pipe-pane (no arguments stops the pipe).
    let output = runner
        .run("tmux", &["pipe-pane"], 5)
        .context("failed to run tmux pipe-pane (stop)")?;

    if output.exit_code != 0 {
        anyhow::bail!(
            "tmux pipe-pane (stop) exited with code {}: {}",
            output.exit_code,
            output.stderr.trim()
        );
    }

    // Unset pane logging state option.
    let unset_output = runner
        .run("tmux", &["set-option", "-p", "-u", "@fterm_logging"], 5)
        .context("failed to unset @fterm_logging pane option")?;
    if unset_output.exit_code != 0 {
        debug!(
            exit_code = unset_output.exit_code,
            "could not unset @fterm_logging (non-fatal)"
        );
    }

    // Pre-check: skip gracefully if log file does not exist.
    if !log_path.exists() {
        warn!(path = %log_path.display(), "log file does not exist; skipping disconnect marker");
        return Ok(());
    }

    // Append disconnect marker.
    let timestamp = Local::now().format("%Y-%m-%dT%H:%M:%S%z");
    let marker = format!("[{timestamp}] === Session Disconnected ===\n");

    debug!(path = %log_path.display(), "appending disconnect marker");

    let mut file = OpenOptions::new()
        .append(true)
        .open(log_path)
        .with_context(|| {
            format!(
                "failed to open log file for disconnect marker: {}",
                log_path.display()
            )
        })?;

    file.write_all(marker.as_bytes()).with_context(|| {
        format!(
            "failed to write disconnect marker to: {}",
            log_path.display()
        )
    })?;

    // Compress the log file.
    let log_path_str = log_path
        .to_str()
        .context("log path contains invalid UTF-8")?;

    debug!(path = %log_path_str, "compressing log file with gzip");

    let gzip_output = runner
        .run("gzip", &["--force", log_path_str], 30)
        .context("failed to run gzip on log file")?;

    if gzip_output.exit_code != 0 {
        anyhow::bail!(
            "gzip exited with code {}: {}",
            gzip_output.exit_code,
            gzip_output.stderr.trim()
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::indexing_slicing)]

    use std::collections::HashMap;
    use std::fs;
    use std::path::Path;
    use std::sync::Mutex;

    use anyhow::Result;
    use tempfile::TempDir;

    use crate::external::{AgentListResult, CommandOutput, CommandRunner};

    use super::*;

    /// Mock that records all commands and allows per-command responses.
    struct MockRunner {
        responses: HashMap<String, CommandOutput>,
        calls: Mutex<Vec<String>>,
    }

    impl MockRunner {
        fn new() -> Self {
            Self {
                responses: HashMap::new(),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn with_response(mut self, key: &str, output: CommandOutput) -> Self {
            self.responses.insert(String::from(key), output);
            self
        }

        fn all_success() -> Self {
            Self::new()
        }

        fn recorded_calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl CommandRunner for MockRunner {
        fn run(&self, cmd: &str, args: &[&str], _timeout_secs: u64) -> Result<CommandOutput> {
            let key = if args.is_empty() {
                String::from(cmd)
            } else {
                format!("{cmd} {}", args.join(" "))
            };
            self.calls.lock().unwrap().push(key.clone());
            Ok(self.responses.get(&key).cloned().unwrap_or(CommandOutput {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
            }))
        }

        fn ssh_resolve(&self, _host: &str, _config_args: &[String]) -> Result<String> {
            Ok(String::new())
        }

        fn ssh_agent_list(&self) -> Result<AgentListResult> {
            Ok(AgentListResult {
                available: false,
                keys: Vec::new(),
            })
        }

        fn ssh_keygen_fingerprint(&self, _path: &Path) -> Result<String> {
            Ok(String::new())
        }
    }

    #[test]
    fn stops_pipe_appends_marker_and_compresses() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("session.log");
        fs::write(&log_path, "existing content\n").unwrap();
        let runner = MockRunner::all_success();

        // Act
        stop(&runner, &log_path).unwrap();

        // Assert - verify disconnect marker was written
        // Note: gzip is mocked so file still exists as-is
        let content = fs::read_to_string(&log_path).unwrap();
        assert!(content.contains("existing content"));
        assert!(content.contains("=== Session Disconnected ==="));

        // Verify commands were called in order
        let calls = runner.recorded_calls();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0], "tmux pipe-pane");
        assert_eq!(calls[1], "tmux set-option -p -u @fterm_logging");
        assert!(calls[2].starts_with("gzip --force"));
    }

    #[test]
    fn returns_error_when_pipe_pane_stop_fails() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("session.log");
        fs::write(&log_path, "").unwrap();
        let runner = MockRunner::new().with_response(
            "tmux pipe-pane",
            CommandOutput {
                exit_code: 1,
                stdout: String::new(),
                stderr: String::from("server not found"),
            },
        );

        // Act
        let result = stop(&runner, &log_path);

        // Assert
        assert!(result.is_err());
        let err_msg = format!("{:#}", result.unwrap_err());
        assert!(err_msg.contains("tmux pipe-pane (stop) exited with code 1"));
    }

    #[test]
    fn returns_error_when_gzip_fails() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("session.log");
        fs::write(&log_path, "data\n").unwrap();
        let gzip_key = format!("gzip --force {}", log_path.display());
        let runner = MockRunner::new().with_response(
            &gzip_key,
            CommandOutput {
                exit_code: 1,
                stdout: String::new(),
                stderr: String::from("gzip: No such file or directory"),
            },
        );

        // Act
        let result = stop(&runner, &log_path);

        // Assert
        assert!(result.is_err());
        let err_msg = format!("{:#}", result.unwrap_err());
        assert!(err_msg.contains("gzip exited with code 1"));
    }

    #[test]
    fn missing_log_file_skips_gracefully() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("nonexistent.log");
        let runner = MockRunner::all_success();

        // Act
        let result = stop(&runner, &log_path);

        // Assert — should succeed without error
        assert!(result.is_ok());
        // tmux commands still run, but no file I/O or gzip
        let calls = runner.recorded_calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], "tmux pipe-pane");
        assert_eq!(calls[1], "tmux set-option -p -u @fterm_logging");
    }

    #[test]
    fn disconnect_marker_has_timestamp_format() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("session.log");
        fs::write(&log_path, "").unwrap();
        let runner = MockRunner::all_success();

        // Act
        stop(&runner, &log_path).unwrap();

        // Assert
        let content = fs::read_to_string(&log_path).unwrap();
        // Verify timestamp format: [YYYY-MM-DDThh:mm:ss+ZZZZ]
        assert!(content.starts_with('['));
        assert!(content.contains('T'));
        assert!(content.contains("] === Session Disconnected ==="));
    }
}
