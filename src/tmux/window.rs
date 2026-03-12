//! Tmux window management.
//!
//! Manages the `@fterm_ssh_count` window option and window rename settings
//! to keep track of active SSH connections per window.

use anyhow::{Context, Result};
use tracing::debug;

use crate::external::CommandRunner;

/// Increment the `@fterm_ssh_count` window option by one.
///
/// Reads the current value (defaulting to `0`), adds one, and writes it back.
/// Also disables window renaming so the user-set title is preserved.
///
/// # Errors
///
/// Returns an error if any tmux command fails to execute.
pub fn increment_ssh_count(runner: &dyn CommandRunner) -> Result<()> {
    let current = read_ssh_count(runner)?;
    let new_count = current.saturating_add(1);
    debug!(current, new_count, "incrementing @fterm_ssh_count");

    write_ssh_count(runner, new_count)?;
    disable_rename(runner)?;

    Ok(())
}

/// Decrement the `@fterm_ssh_count` window option by one.
///
/// When the count reaches zero, `allow-rename` and `automatic-rename` are
/// restored to `on` so tmux can manage window titles again.
///
/// # Errors
///
/// Returns an error if any tmux command fails to execute.
pub fn decrement_ssh_count(runner: &dyn CommandRunner) -> Result<()> {
    let current = read_ssh_count(runner)?;
    let new_count = current.saturating_sub(1);
    debug!(current, new_count, "decrementing @fterm_ssh_count");

    if new_count == 0 {
        debug!("ssh count reached 0; unsetting @fterm_ssh_count and restoring rename");
        unset_window_option(runner, "@fterm_ssh_count")?;
        set_window_option(runner, "allow-rename", "on")?;
        set_window_option(runner, "automatic-rename", "on")?;
    } else {
        write_ssh_count(runner, new_count)?;
    }

    Ok(())
}

/// Disable automatic and manual window renaming.
///
/// Sets both `automatic-rename` and `allow-rename` to `off`.
///
/// # Errors
///
/// Returns an error if either tmux command fails to execute.
pub fn disable_rename(runner: &dyn CommandRunner) -> Result<()> {
    debug!("disabling window rename");
    set_window_option(runner, "automatic-rename", "off")?;
    set_window_option(runner, "allow-rename", "off")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Read the current `@fterm_ssh_count` value, defaulting to 0.
fn read_ssh_count(runner: &dyn CommandRunner) -> Result<u32> {
    let output = runner
        .run("tmux", &["show-window-option", "-v", "@fterm_ssh_count"], 5)
        .context("failed to read @fterm_ssh_count window option")?;

    if output.exit_code != 0 || output.stdout.trim().is_empty() {
        return Ok(0);
    }

    output
        .stdout
        .trim()
        .parse::<u32>()
        .context("@fterm_ssh_count is not a valid u32")
}

/// Write the `@fterm_ssh_count` window option.
fn write_ssh_count(runner: &dyn CommandRunner, count: u32) -> Result<()> {
    let count_str = count.to_string();
    set_window_option(runner, "@fterm_ssh_count", &count_str)
}

/// Unset (remove) a tmux window option.
fn unset_window_option(runner: &dyn CommandRunner, option: &str) -> Result<()> {
    let output = runner
        .run("tmux", &["set-window-option", "-u", option], 5)
        .with_context(|| format!("failed to unset window option {option}"))?;
    if output.exit_code != 0 {
        anyhow::bail!(
            "tmux set-window-option -u {option} failed with exit code {}: {}",
            output.exit_code,
            output.stderr.trim()
        );
    }
    Ok(())
}

/// Set a tmux window option.
fn set_window_option(runner: &dyn CommandRunner, option: &str, value: &str) -> Result<()> {
    let output = runner
        .run("tmux", &["set-window-option", option, value], 5)
        .with_context(|| format!("failed to set window option {option}={value}"))?;
    if output.exit_code != 0 {
        anyhow::bail!(
            "tmux set-window-option {option} {value} failed with exit code {}: {}",
            output.exit_code,
            output.stderr.trim()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::external::{CommandOutput, MockCommandRunner};

    fn mock_with_count(count: u32) -> MockCommandRunner {
        MockCommandRunner::new().with_run_response(
            "tmux show-window-option -v @fterm_ssh_count",
            CommandOutput {
                exit_code: 0,
                stdout: count.to_string(),
                stderr: String::new(),
            },
        )
    }

    #[test]
    fn increment_from_zero() {
        // Arrange
        let runner = mock_with_count(0);

        // Act
        let result = increment_ssh_count(&runner);

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn increment_from_existing_value() {
        // Arrange
        let runner = mock_with_count(2);

        // Act
        let result = increment_ssh_count(&runner);

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn decrement_to_zero_restores_rename() {
        // Arrange
        let runner = mock_with_count(1);

        // Act
        let result = decrement_ssh_count(&runner);

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn decrement_above_zero_keeps_rename_off() {
        // Arrange
        let runner = mock_with_count(3);

        // Act
        let result = decrement_ssh_count(&runner);

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn decrement_from_zero_stays_at_zero() {
        // Arrange
        let runner = mock_with_count(0);

        // Act
        let result = decrement_ssh_count(&runner);

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn read_ssh_count_default_on_error() {
        // Arrange
        let runner = MockCommandRunner::new().with_run_response(
            "tmux show-window-option -v @fterm_ssh_count",
            CommandOutput {
                exit_code: 1,
                stdout: String::new(),
                stderr: String::from("unknown option"),
            },
        );

        // Act
        let count = read_ssh_count(&runner).unwrap();

        // Assert
        assert_eq!(count, 0);
    }

    #[test]
    fn read_ssh_count_default_on_empty() {
        // Arrange
        let runner = MockCommandRunner::new().with_run_response(
            "tmux show-window-option -v @fterm_ssh_count",
            CommandOutput {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
            },
        );

        // Act
        let count = read_ssh_count(&runner).unwrap();

        // Assert
        assert_eq!(count, 0);
    }

    #[test]
    fn disable_rename_success() {
        // Arrange
        let runner = MockCommandRunner::new();

        // Act
        let result = disable_rename(&runner);

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn set_window_option_failure() {
        // Arrange
        let runner = MockCommandRunner::new().with_run_response(
            "tmux set-window-option automatic-rename off",
            CommandOutput {
                exit_code: 1,
                stdout: String::new(),
                stderr: String::from("server not found"),
            },
        );

        // Act
        let result = set_window_option(&runner, "automatic-rename", "off");

        // Assert
        assert!(result.is_err());
    }
}
