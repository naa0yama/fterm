//! Validation orchestrator — runs all checks and collects results.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tracing::{debug, warn};

use crate::external::CommandRunner;
use crate::validate::{
    basic, cm_dir, control_path, duplicate, host_prefix, identity, proxyjump, syntax,
};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Level of a validation message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckLevel {
    /// Hard failure that should block usage.
    Error,
    /// Soft issue that may indicate misconfiguration.
    Warn,
    /// Informational notice (not an error or warning).
    Info,
}

/// A single validation message.
#[derive(Debug, Clone)]
pub struct CheckMessage {
    /// Severity level.
    pub level: CheckLevel,
    /// Human-readable description.
    pub text: String,
}

/// Result of running all validation checks.
#[derive(Debug)]
pub struct ValidationResult {
    /// All collected messages.
    pub messages: Vec<CheckMessage>,
    /// Count of error-level messages.
    pub error_count: usize,
    /// Count of warning-level messages.
    pub warn_count: usize,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse a single value from `ssh -G` output by key (case-insensitive).
#[must_use]
pub fn parse_ssh_g_value(output: &str, key: &str) -> Option<String> {
    let key_lower = key.to_lowercase();
    for line in output.lines() {
        if line
            .split_once(' ')
            .is_some_and(|(k, _)| k.to_lowercase() == key_lower)
        {
            let (_, v) = line.split_once(' ')?;
            return Some(String::from(v));
        }
    }
    None
}

/// Parse all values for a given key from `ssh -G` output (case-insensitive).
#[must_use]
pub fn parse_ssh_g_values(output: &str, key: &str) -> Vec<String> {
    let key_lower = key.to_lowercase();
    output
        .lines()
        .filter_map(|line| {
            let (k, v) = line.split_once(' ')?;
            (k.to_lowercase() == key_lower).then(|| String::from(v))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Orchestration
// ---------------------------------------------------------------------------

/// Run all validation checks against the SSH configuration.
///
/// # Errors
/// Returns an error if any critical I/O operation fails.
#[tracing::instrument(skip(runner, config_files, hosts, config_args), err)]
#[allow(clippy::too_many_lines)]
pub fn run_all_checks(
    runner: &dyn CommandRunner,
    ssh_home: &Path,
    config_files: &[PathBuf],
    hosts: &[String],
    config_args: &[String],
) -> Result<ValidationResult> {
    let mut messages: Vec<CheckMessage> = Vec::new();

    // 1. ControlMaster directory
    debug!("running cm_dir check");
    messages.extend(cm_dir::check(ssh_home));

    // 2. Syntax check — if it fails, return immediately
    debug!("running syntax check");
    let syntax_msgs = syntax::check(runner, config_args).context("syntax check failed")?;
    let has_syntax_errors = syntax_msgs.iter().any(|m| m.level == CheckLevel::Error);
    messages.extend(syntax_msgs);
    if has_syntax_errors {
        let (error_count, warn_count) = count_levels(&messages);
        return Ok(ValidationResult {
            messages,
            error_count,
            warn_count,
        });
    }

    // 3. Duplicate host detection
    debug!("running duplicate host check");
    let dup_msgs = duplicate::check(config_files).context("duplicate host check failed")?;
    messages.extend(dup_msgs);

    // 4. Per-host checks (resolve ssh -G once per host)
    for host in hosts {
        debug!(host = %host, "running per-host checks");

        // Host prefix
        match host_prefix::check(host, config_files) {
            Ok(msgs) => messages.extend(msgs),
            Err(e) => {
                warn!(host = %host, error = %e, "host_prefix check failed");
            }
        }

        // Resolve ssh -G output once for this host
        let ssh_g_output = match runner
            .ssh_resolve(host, config_args)
            .with_context(|| format!("failed to resolve host: {host}"))
        {
            Ok(output) => output,
            Err(e) => {
                warn!(host = %host, error = %e, "ssh_resolve failed; skipping per-host checks");
                continue;
            }
        };

        // Basic (pure function)
        messages.extend(basic::check(&ssh_g_output, host));

        // Identity
        match identity::check(runner, &ssh_g_output, host) {
            Ok(msgs) => messages.extend(msgs),
            Err(e) => {
                warn!(host = %host, error = %e, "identity check failed");
            }
        }

        // ProxyJump
        let mut visited = vec![host.clone()];
        match proxyjump::check(
            runner,
            &ssh_g_output,
            host,
            config_args,
            hosts,
            &mut visited,
        ) {
            Ok(msgs) => messages.extend(msgs),
            Err(e) => {
                warn!(host = %host, error = %e, "proxyjump check failed");
            }
        }

        // ControlPath (pure function)
        messages.extend(control_path::check(&ssh_g_output, host));
    }

    let (error_count, warn_count) = count_levels(&messages);
    Ok(ValidationResult {
        messages,
        error_count,
        warn_count,
    })
}

/// Count errors and warnings in a slice of messages.
fn count_levels(messages: &[CheckMessage]) -> (usize, usize) {
    let mut errors = 0usize;
    let mut warns = 0usize;
    for m in messages {
        match m.level {
            CheckLevel::Error => {
                errors = errors.saturating_add(1);
            }
            CheckLevel::Warn => {
                warns = warns.saturating_add(1);
            }
            CheckLevel::Info => {}
        }
    }
    (errors, warns)
}

/// Format a coloured summary line for the validation result.
#[must_use]
pub fn format_summary(result: &ValidationResult) -> String {
    if result.error_count == 0 && result.warn_count == 0 {
        return String::from("\x1b[32m✓ All checks passed.\x1b[0m");
    }

    let mut parts: Vec<String> = Vec::new();
    if result.error_count > 0 {
        parts.push(format!("\x1b[31m{} error(s)\x1b[0m", result.error_count));
    }
    if result.warn_count > 0 {
        parts.push(format!("\x1b[33m{} warning(s)\x1b[0m", result.warn_count));
    }
    parts.join(", ")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::indexing_slicing)]

    use super::*;

    #[test]
    fn parse_ssh_g_value_finds_key() {
        // Arrange
        let output = "hostname example.com\nport 22\nuser admin\n";

        // Act
        let hostname = parse_ssh_g_value(output, "hostname");
        let port = parse_ssh_g_value(output, "port");

        // Assert
        assert_eq!(hostname, Some(String::from("example.com")));
        assert_eq!(port, Some(String::from("22")));
    }

    #[test]
    fn parse_ssh_g_value_case_insensitive() {
        // Arrange
        let output = "HostName example.com\n";

        // Act
        let result = parse_ssh_g_value(output, "hostname");

        // Assert
        assert_eq!(result, Some(String::from("example.com")));
    }

    #[test]
    fn parse_ssh_g_value_returns_none_for_missing_key() {
        // Arrange
        let output = "hostname example.com\n";

        // Act
        let result = parse_ssh_g_value(output, "user");

        // Assert
        assert!(result.is_none());
    }

    #[test]
    fn parse_ssh_g_values_collects_all() {
        // Arrange
        let output = "identityfile ~/.ssh/id_rsa\nidentityfile ~/.ssh/id_ed25519\n";

        // Act
        let result = parse_ssh_g_values(output, "identityfile");

        // Assert
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "~/.ssh/id_rsa");
        assert_eq!(result[1], "~/.ssh/id_ed25519");
    }

    #[test]
    fn count_levels_counts_correctly() {
        // Arrange
        let msgs = vec![
            CheckMessage {
                level: CheckLevel::Error,
                text: String::from("e1"),
            },
            CheckMessage {
                level: CheckLevel::Warn,
                text: String::from("w1"),
            },
            CheckMessage {
                level: CheckLevel::Error,
                text: String::from("e2"),
            },
        ];

        // Act
        let (errors, warns) = count_levels(&msgs);

        // Assert
        assert_eq!(errors, 2);
        assert_eq!(warns, 1);
    }

    #[test]
    fn format_summary_all_passed() {
        // Arrange
        let result = ValidationResult {
            messages: Vec::new(),
            error_count: 0,
            warn_count: 0,
        };

        // Act
        let summary = format_summary(&result);

        // Assert
        assert!(summary.contains("All checks passed"));
    }

    #[test]
    fn format_summary_with_errors_and_warnings() {
        // Arrange
        let result = ValidationResult {
            messages: Vec::new(),
            error_count: 2,
            warn_count: 3,
        };

        // Act
        let summary = format_summary(&result);

        // Assert
        assert!(summary.contains("2 error(s)"));
        assert!(summary.contains("3 warning(s)"));
    }

    // -----------------------------------------------------------------------
    // run_all_checks tests
    // -----------------------------------------------------------------------

    use tempfile::TempDir;

    use crate::external::MockCommandRunner;

    #[test]
    fn run_all_checks_pass_syntax_no_hosts_returns_empty_messages() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let ssh_home = tmp.path();
        // Create cm dir so cm_dir check passes
        std::fs::create_dir_all(ssh_home.join("conf.d").join("cm")).unwrap();

        let runner = MockCommandRunner::new().with_ssh_resolve(
            "syntax.check.dummy.host",
            "hostname syntax.check.dummy.host\n",
        );

        let config_files: Vec<PathBuf> = Vec::new();
        let hosts: Vec<String> = Vec::new();
        let config_args: Vec<String> = Vec::new();

        // Act
        let result =
            run_all_checks(&runner, ssh_home, &config_files, &hosts, &config_args).unwrap();

        // Assert — cm_dir passes (dir exists), syntax passes, duplicate passes (no files), no hosts
        assert_eq!(result.error_count, 0);
        assert_eq!(result.warn_count, 0);
        assert!(result.messages.is_empty());
    }

    #[test]
    fn run_all_checks_syntax_fail_returns_early_with_syntax_errors() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let ssh_home = tmp.path();
        std::fs::create_dir_all(ssh_home.join("conf.d").join("cm")).unwrap();

        let runner = MockCommandRunner::new().with_ssh_resolve_error(
            "syntax.check.dummy.host",
            "/home/user/.ssh/config: line 5: Bad configuration option: xyz",
        );

        let config_files: Vec<PathBuf> = Vec::new();
        let hosts: Vec<String> = vec![String::from("myhost")];
        let config_args: Vec<String> = Vec::new();

        // Act
        let result =
            run_all_checks(&runner, ssh_home, &config_files, &hosts, &config_args).unwrap();

        // Assert — should have syntax errors and return early (no per-host checks)
        assert!(result.error_count > 0);
        assert!(result.messages.iter().any(|m| m.text.contains("[syntax]")));
        // No per-host messages should appear since we returned early
        assert!(
            !result
                .messages
                .iter()
                .any(|m| m.text.contains("myhost") && !m.text.contains("[syntax]"))
        );
    }

    // -----------------------------------------------------------------------
    // count_levels tests
    // -----------------------------------------------------------------------

    #[test]
    fn count_levels_empty_messages_returns_zero() {
        // Arrange
        let msgs: Vec<CheckMessage> = Vec::new();

        // Act
        let (errors, warns) = count_levels(&msgs);

        // Assert
        assert_eq!(errors, 0);
        assert_eq!(warns, 0);
    }

    // -----------------------------------------------------------------------
    // format_summary tests
    // -----------------------------------------------------------------------

    #[test]
    fn format_summary_only_errors_no_warnings() {
        // Arrange
        let result = ValidationResult {
            messages: Vec::new(),
            error_count: 3,
            warn_count: 0,
        };

        // Act
        let summary = format_summary(&result);

        // Assert
        assert!(summary.contains("3 error(s)"));
        assert!(!summary.contains("warning(s)"));
    }

    #[test]
    fn format_summary_only_warnings_no_errors() {
        // Arrange
        let result = ValidationResult {
            messages: Vec::new(),
            error_count: 0,
            warn_count: 5,
        };

        // Act
        let summary = format_summary(&result);

        // Assert
        assert!(summary.contains("5 warning(s)"));
        assert!(!summary.contains("error(s)"));
    }

    // -----------------------------------------------------------------------
    // parse_ssh_g_value / parse_ssh_g_values edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn parse_ssh_g_value_empty_output_returns_none() {
        // Arrange
        let output = "";

        // Act
        let result = parse_ssh_g_value(output, "hostname");

        // Assert
        assert!(result.is_none());
    }

    #[test]
    fn parse_ssh_g_values_empty_output_returns_empty_vec() {
        // Arrange
        let output = "";

        // Act
        let result = parse_ssh_g_values(output, "identityfile");

        // Assert
        assert!(result.is_empty());
    }

    #[test]
    fn parse_ssh_g_values_no_matching_key_returns_empty_vec() {
        // Arrange
        let output = "hostname example.com\nport 22\nuser admin\n";

        // Act
        let result = parse_ssh_g_values(output, "identityfile");

        // Assert
        assert!(result.is_empty());
    }
}
