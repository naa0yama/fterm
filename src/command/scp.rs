//! SCP wrapper with validation and logging.
//!
//! Similar to the SSH wrapper but tailored for SCP file transfers:
//! validates each remote host, logs the session, and displays result banners.

use std::env;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use chrono::Local;
use tracing::{debug, warn};

use crate::config::details;
use crate::config::home::{build_config_args, get_dir};
use crate::config::include::resolve_included_files;
use crate::external::CommandRunner;
use crate::logging::start;
use crate::logging::stop;
use crate::tmux::pane;
use crate::tmux::session::{TmuxAction, ensure_tmux, get_tmux_identifiers};
use crate::tmux::window;
use crate::util::dry_run;
use crate::util::duration;
use crate::util::log_dir;
use crate::util::scp_args::extract_hosts;
use crate::util::splash;
use crate::util::ssh_env;
use crate::validate::orchestrator::{format_summary, run_all_checks};

/// Run the SCP wrapper command.
///
/// Validates remote hosts, sets up logging and tmux integration, executes
/// `scp`, and displays a result banner.
///
/// # Errors
///
/// Returns an error if any internal operation (tmux, logging, validation)
/// fails unexpectedly.
#[tracing::instrument(skip(runner, args), err)]
#[allow(clippy::too_many_lines)]
pub fn run(runner: &dyn CommandRunner, args: &[String]) -> Result<i32> {
    let start_time = Instant::now();

    // Extract remote hosts
    let remote_hosts = extract_hosts(args);
    if remote_hosts.is_empty() {
        debug!("no remote hosts found in args; exec scp directly");
        return Ok(exec_scp(args));
    }

    debug!(?remote_hosts, "extracted remote hosts from SCP args");

    if dry_run::is_scp(args) {
        debug!("dry-run flag detected; exec scp directly");
        return Ok(exec_scp(args));
    }

    // Pre-connect checks
    if let Some(code) =
        pre_connect_checks(runner, args, &remote_hosts).context("scp pre-connect checks failed")?
    {
        return Ok(code);
    }

    // Resolve first host once via ssh -G
    let config_args: Vec<String> = build_config_args()?;
    let first_host = remote_hosts.first().context("no remote hosts")?;
    let ssh_g_output = runner
        .ssh_resolve(first_host, &config_args)
        .with_context(|| format!("failed to resolve host: {first_host}"))?;

    let user = crate::config::connection::parse_connection_info(&ssh_g_output).map_or_else(
        || {
            warn!(host = %first_host, "could not parse connection info; defaulting to unknown user");
            String::from("unknown")
        },
        |info| info.user,
    );

    // Generate log path with hosts joined by underscore
    let hosts_joined = remote_hosts.join("_");
    let log_path = generate_scp_log_path(runner, &user, &hosts_joined);
    debug!(log_path = %log_path.display(), "generated SCP log path");

    // Get SSH details and agent keys from pre-resolved output
    let ssh_details = details::parse(&ssh_g_output);
    let agent_keys =
        crate::config::agent::get_matched_agent_keys_from_output(runner, &ssh_g_output)
            .unwrap_or_default();

    // Save original pane title for restore on teardown
    let original_pane_title = pane::get_title(runner).unwrap_or_default();

    // Setup (logging, banner, tmux)
    setup_scp_session(runner, &log_path, &hosts_joined, &ssh_details, &agent_keys)?;

    // Execute SCP (directly, not via runner)
    let scp_exit_code = exec_scp_status(args, &config_args);
    let success = scp_exit_code == 0;

    // Teardown
    let elapsed = start_time.elapsed().as_secs();
    let duration_str = duration::format(elapsed);
    debug!(duration = %duration_str, exit_code = scp_exit_code, "SCP completed");

    teardown_scp_session(
        runner,
        &log_path,
        &remote_hosts,
        success,
        &duration_str,
        &original_pane_title,
    );

    Ok(scp_exit_code)
}

/// Pre-connect checks: tmux, agent, and validation.
///
/// Returns `Some(exit_code)` for early exit, `None` to continue.
fn pre_connect_checks(
    runner: &dyn CommandRunner,
    args: &[String],
    remote_hosts: &[String],
) -> Result<Option<i32>> {
    // Tmux check
    if env::var("TMUX").is_err() {
        debug!("not inside tmux; delegating via ensure_tmux");
        let action =
            ensure_tmux(runner, "fterm", "scp", args).context("failed to ensure tmux session")?;
        if action == TmuxAction::DelegatedToTmux {
            return Ok(Some(0));
        }
    }

    // Load SSH_ENV file if set
    ssh_env::load();

    // SSH agent check
    let agent = runner
        .ssh_agent_list()
        .context("failed to check SSH agent")?;
    if !agent.available {
        #[allow(clippy::print_stderr)]
        {
            eprintln!("Error: SSH agent is not available. Start ssh-agent first.");
        }
        return Ok(Some(1));
    }

    // Validation
    let ssh_home = get_dir();
    let config_path = ssh_home.join("config");
    let config_files = if config_path.exists() {
        resolve_included_files(&config_path, &ssh_home)
            .context("failed to resolve SSH config includes")?
    } else {
        Vec::new()
    };
    let config_args: Vec<String> = build_config_args()?;

    let validation = run_all_checks(runner, &ssh_home, &config_files, remote_hosts, &config_args)
        .context("SSH config validation failed")?;

    if validation.error_count > 0 {
        let summary = format_summary(&validation);
        #[allow(clippy::print_stderr)]
        {
            eprintln!("{summary}");
            for msg in &validation.messages {
                eprintln!("  {}", msg.text);
            }
        }
        return Ok(Some(1));
    }

    if validation.warn_count > 0 {
        let summary = format_summary(&validation);
        #[allow(clippy::print_stderr)]
        {
            eprintln!("{summary}");
        }
    }

    Ok(None)
}

/// Setup SCP session: logging, banner, and tmux state.
fn setup_scp_session(
    runner: &dyn CommandRunner,
    log_path: &std::path::Path,
    hosts_joined: &str,
    ssh_details: &[String],
    agent_keys: &[String],
) -> Result<()> {
    // Start logging
    start::start(runner, log_path, hosts_joined, ssh_details, agent_keys)
        .context("failed to start logging")?;

    // Print connect banner
    let banner = splash::scp_connect_banner(
        hosts_joined,
        &splash::BannerParams {
            log_path: &log_path.to_string_lossy(),
            ssh_details,
            agent_keys,
        },
    );
    #[allow(clippy::print_stderr)]
    {
        eprint!("{banner}");
    }

    // Set tmux pane title
    let pane_title = format!("scp:{hosts_joined}");
    if let Err(e) = pane::set_title(runner, &pane_title) {
        warn!("failed to set pane title: {e:#}");
    }

    // Increment SSH count (also disables rename)
    if let Err(e) = window::increment_ssh_count(runner) {
        warn!("failed to increment ssh count: {e:#}");
    }

    // Set @fterm_ssh_host (format: "scp:host1 host2")
    let scp_host_value = format!("scp:{}", hosts_joined.replace('_', " "));
    if let Err(e) = pane::set_ssh_host(runner, &scp_host_value) {
        warn!("failed to set @fterm_ssh_host: {e:#}");
    }

    Ok(())
}

/// Teardown SCP session: banner, tmux cleanup, logging.
fn teardown_scp_session(
    runner: &dyn CommandRunner,
    log_path: &std::path::Path,
    remote_hosts: &[String],
    success: bool,
    duration_str: &str,
    original_pane_title: &str,
) {
    // Print result banner
    let banner = splash::scp_result_banner(
        remote_hosts,
        success,
        duration_str,
        &log_path.to_string_lossy(),
    );
    #[allow(clippy::print_stderr)]
    {
        eprint!("{banner}");
    }

    // Reset pane style
    if let Err(e) = pane::reset_style(runner) {
        warn!("failed to reset pane style: {e:#}");
    }

    // Restore pane title
    if let Err(e) = pane::set_title(runner, original_pane_title) {
        warn!("failed to restore pane title: {e:#}");
    }

    // Unset @fterm_ssh_host
    if let Err(e) = pane::unset_ssh_host(runner) {
        warn!("failed to unset @fterm_ssh_host: {e:#}");
    }

    // Decrement SSH count (restores rename when reaching 0)
    if let Err(e) = window::decrement_ssh_count(runner) {
        warn!("failed to decrement ssh count: {e:#}");
    }

    // Stop logging
    if let Err(e) = stop::stop(runner, log_path) {
        warn!("failed to stop logging: {e:#}");
    }

    // Reset terminal title
    #[allow(clippy::print_stderr)]
    {
        eprint!("\x1b]0;\x07");
    }
}

/// Execute SCP directly using `std::process::Command::status()`.
///
/// Prepends `-F` config arguments so custom config dirs are honoured.
/// Returns the exit code of the SCP process.
fn exec_scp_status(args: &[String], config_args: &[String]) -> i32 {
    crate::external::exec_with_config("scp", args, config_args)
}

/// Execute SCP directly and return its exit code.
///
/// Used when no wrapping is needed (no remote hosts, dry-run).
/// Prepends `-F` config arguments when available.
fn exec_scp(args: &[String]) -> i32 {
    let config_args = build_config_args().unwrap_or_default();
    crate::external::exec_with_config("scp", args, &config_args)
}

/// Generate the log file path for SCP sessions.
///
/// Format: `{prefix}/{YYYY/MM/DD}/{YYYYMMDDTHHMMSS}_{tmux_ids}_scp_{user}@{hosts}.log`
fn generate_scp_log_path(runner: &dyn CommandRunner, user: &str, hosts: &str) -> PathBuf {
    let prefix = log_dir::get_prefix();
    let now = Local::now();
    let date_dir = now.format("%Y/%m/%d").to_string();
    let timestamp = now.format("%Y%m%dT%H%M%S").to_string();

    let tmux_ids = get_tmux_identifiers(runner);

    let filename = format!("{timestamp}_{tmux_ids}_scp_{user}@{hosts}.log");

    PathBuf::from(&prefix).join(&date_dir).join(&filename)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use serial_test::serial;
    use tempfile::TempDir;

    use super::*;
    use crate::external::AgentListResult;
    use crate::external::MockCommandRunner;

    #[test]
    fn generate_scp_log_path_contains_expected_parts() {
        // Arrange
        let runner = MockCommandRunner::new();

        // Act
        let path = generate_scp_log_path(&runner, "deploy", "host1_host2");
        let path_str = path.to_string_lossy();

        // Assert
        assert!(path_str.contains("scp_deploy@host1_host2.log"));
    }

    #[test]
    fn generate_scp_log_path_single_host() {
        // Arrange
        let runner = MockCommandRunner::new();

        // Act
        let path = generate_scp_log_path(&runner, "root", "web-server");
        let path_str = path.to_string_lossy();

        // Assert
        assert!(path_str.contains("scp_root@web-server.log"));
        assert!(path_str.ends_with(".log"));
    }

    #[test]
    fn generate_scp_log_path_contains_date_directory() {
        // Arrange
        let runner = MockCommandRunner::new();
        let now = Local::now();
        let expected_date = now.format("%Y/%m/%d").to_string();

        // Act
        let path = generate_scp_log_path(&runner, "user", "host");
        let path_str = path.to_string_lossy();

        // Assert
        assert!(
            path_str.contains(&expected_date),
            "path should contain date directory: {path_str}"
        );
    }

    #[test]
    fn generate_scp_log_path_contains_timestamp_prefix() {
        // Arrange
        let runner = MockCommandRunner::new();
        let now = Local::now();
        let expected_prefix = now.format("%Y%m%dT%H%M").to_string();

        // Act
        let path = generate_scp_log_path(&runner, "admin", "db-server");
        let path_str = path.to_string_lossy();

        // Assert
        assert!(
            path_str.contains(&expected_prefix),
            "path should contain timestamp prefix: {path_str}"
        );
    }

    #[test]
    fn generate_scp_log_path_special_characters_in_user() {
        // Arrange
        let runner = MockCommandRunner::new();

        // Act
        let path = generate_scp_log_path(&runner, "deploy-ci", "prod_staging");
        let path_str = path.to_string_lossy();

        // Assert
        assert!(path_str.contains("scp_deploy-ci@prod_staging.log"));
    }

    #[test]
    fn setup_scp_session_succeeds_with_mock_runner() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("scp_session.log");
        let runner = MockCommandRunner::new();
        let ssh_details = vec![String::from("hostname example.com")];
        let agent_keys = vec![String::from("SHA256:abc key@host (ED25519)")];

        // Act
        let result =
            setup_scp_session(&runner, &log_path, "host1_host2", &ssh_details, &agent_keys);

        // Assert
        assert!(
            result.is_ok(),
            "setup_scp_session should succeed: {result:?}"
        );
        assert!(log_path.exists(), "log file should be created");
        let content = std::fs::read_to_string(&log_path).unwrap();
        assert!(content.contains("=== SSH Config ==="));
        assert!(content.contains("hostname example.com"));
        assert!(content.contains("=== Matched Agent Keys ==="));
        assert!(content.contains("SHA256:abc key@host (ED25519)"));
    }

    #[test]
    fn setup_scp_session_creates_log_directory() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("nested").join("dir").join("scp.log");
        let runner = MockCommandRunner::new();

        // Act
        let result = setup_scp_session(&runner, &log_path, "myhost", &[], &[]);

        // Assert
        assert!(result.is_ok());
        assert!(log_path.exists());
    }

    #[test]
    fn setup_scp_session_with_empty_details() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("empty.log");
        let runner = MockCommandRunner::new();

        // Act
        let result = setup_scp_session(&runner, &log_path, "host", &[], &[]);

        // Assert
        assert!(result.is_ok());
        let content = std::fs::read_to_string(&log_path).unwrap();
        // No details/keys means no header at all
        assert_eq!(content, "");
    }

    #[test]
    fn teardown_scp_session_does_not_panic() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("teardown.log");
        let runner = MockCommandRunner::new();
        let remote_hosts = vec![String::from("host1"), String::from("host2")];

        // Act / Assert - should not panic regardless of success flag
        teardown_scp_session(&runner, &log_path, &remote_hosts, true, "0s", "");
        teardown_scp_session(&runner, &log_path, &remote_hosts, false, "0s", "");
    }

    #[test]
    fn teardown_scp_session_single_host_success() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("single.log");
        let runner = MockCommandRunner::new();
        let remote_hosts = vec![String::from("production")];

        // Act / Assert - should complete without panic
        teardown_scp_session(&runner, &log_path, &remote_hosts, true, "0s", "");
    }

    #[test]
    fn teardown_scp_session_failure_does_not_panic() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("fail.log");
        let runner = MockCommandRunner::new();
        let remote_hosts = vec![String::from("staging")];

        // Act / Assert - failure flag should not cause panic
        teardown_scp_session(&runner, &log_path, &remote_hosts, false, "0s", "");
    }

    #[test]
    #[serial(env)]
    fn pre_connect_checks_agent_unavailable_returns_1() {
        // Arrange
        // SAFETY: test runs single-threaded; env var is restored immediately.
        unsafe { env::set_var("TMUX", "/tmp/tmux-test/default,12345,0") };
        let runner = MockCommandRunner::new().with_agent_list(AgentListResult {
            available: false,
            keys: Vec::new(),
        });
        let args = vec![
            String::from("scp"),
            String::from("file.txt"),
            String::from("host:~/"),
        ];
        let remote_hosts = vec![String::from("host")];

        // Act
        let result = pre_connect_checks(&runner, &args, &remote_hosts);

        // Cleanup
        // SAFETY: test runs single-threaded; restoring env state.
        unsafe { env::remove_var("TMUX") };

        // Assert
        let code = result.unwrap();
        assert_eq!(
            code,
            Some(1),
            "should return Some(1) when agent is unavailable"
        );
    }

    #[test]
    #[serial(env)]
    fn pre_connect_checks_agent_available_no_config_returns_none() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        // Create cm dir so cm_dir check passes
        std::fs::create_dir_all(ssh_dir.join("conf.d").join("cm")).unwrap();
        // No config file inside .ssh

        // Provide full ssh_resolve output so per-host checks pass
        let host_resolve = "hostname 192.168.1.1\nuser deploy\nport 22\nidentitiesonly yes\nidentityfile /dev/null\ncontrolpath /tmp/cm/%r@%h:%p\n";

        let original_home = env::var("HOME").ok();
        // SAFETY: test runs single-threaded; env vars are restored immediately.
        unsafe {
            env::set_var("TMUX", "/tmp/tmux-test/default,12345,0");
            env::set_var("HOME", tmp.path().to_str().unwrap());
        };
        let runner = MockCommandRunner::new()
            .with_agent_list(AgentListResult {
                available: true,
                keys: vec![],
            })
            .with_ssh_resolve(
                "syntax.check.dummy.host",
                "hostname syntax.check.dummy.host\n",
            )
            .with_ssh_resolve("host", host_resolve);
        let args = vec![
            String::from("scp"),
            String::from("file.txt"),
            String::from("host:~/"),
        ];
        let remote_hosts = vec![String::from("host")];

        // Act
        let result = pre_connect_checks(&runner, &args, &remote_hosts);

        // Cleanup
        // SAFETY: test runs single-threaded; restoring env state.
        unsafe {
            env::remove_var("TMUX");
            match &original_home {
                Some(h) => env::set_var("HOME", h),
                None => env::remove_var("HOME"),
            }
        };

        // Assert
        let code = result.unwrap();
        assert_eq!(
            code, None,
            "should return None when agent is available and no config exists"
        );
    }

    #[test]
    #[serial(env)]
    fn pre_connect_checks_validation_errors_returns_1() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(ssh_dir.join("conf.d").join("cm")).unwrap();
        let config_path = ssh_dir.join("config");
        std::fs::write(&config_path, "Host errorhost\n  HostName 10.0.0.1\n").unwrap();

        let original_home = env::var("HOME").ok();
        // SAFETY: test runs single-threaded; env vars are restored immediately.
        unsafe {
            env::set_var("TMUX", "/tmp/tmux-test/default,12345,0");
            env::set_var("HOME", tmp.path().to_str().unwrap());
        };

        // Mock: agent available, syntax passes, but host resolve returns empty
        // output so basic checks produce errors (missing hostname, user, port).
        let runner = MockCommandRunner::new()
            .with_agent_list(AgentListResult {
                available: true,
                keys: vec![],
            })
            .with_ssh_resolve(
                "syntax.check.dummy.host",
                "hostname syntax.check.dummy.host\n",
            )
            .with_ssh_resolve("errorhost", "");
        let args = vec![
            String::from("scp"),
            String::from("file.txt"),
            String::from("errorhost:~/"),
        ];
        let remote_hosts = vec![String::from("errorhost")];

        // Act
        let result = pre_connect_checks(&runner, &args, &remote_hosts);

        // Cleanup
        // SAFETY: test runs single-threaded; restoring env state.
        unsafe {
            env::remove_var("TMUX");
            match &original_home {
                Some(h) => env::set_var("HOME", h),
                None => env::remove_var("HOME"),
            }
        };

        // Assert — validation errors should cause Some(1)
        let code = result.unwrap();
        assert_eq!(
            code,
            Some(1),
            "should return Some(1) when validation has errors"
        );
    }

    #[test]
    #[serial(env)]
    fn pre_connect_checks_validation_warnings_returns_none() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(ssh_dir.join("conf.d").join("cm")).unwrap();
        let config_path = ssh_dir.join("config");
        std::fs::write(&config_path, "Host warnhost\n  HostName 10.0.0.1\n").unwrap();

        let original_home = env::var("HOME").ok();
        // SAFETY: test runs single-threaded; env vars are restored immediately.
        unsafe {
            env::set_var("TMUX", "/tmp/tmux-test/default,12345,0");
            env::set_var("HOME", tmp.path().to_str().unwrap());
        };

        // Mock: all required fields present, but identitiesonly=no triggers warning.
        let host_resolve = "hostname 192.168.1.1\nuser deploy\nport 22\nidentitiesonly no\nidentityfile /dev/null\ncontrolpath /tmp/cm/%r@%h:%p\n";
        let runner = MockCommandRunner::new()
            .with_agent_list(AgentListResult {
                available: true,
                keys: vec![],
            })
            .with_ssh_resolve(
                "syntax.check.dummy.host",
                "hostname syntax.check.dummy.host\n",
            )
            .with_ssh_resolve("warnhost", host_resolve);
        let args = vec![
            String::from("scp"),
            String::from("file.txt"),
            String::from("warnhost:~/"),
        ];
        let remote_hosts = vec![String::from("warnhost")];

        // Act
        let result = pre_connect_checks(&runner, &args, &remote_hosts);

        // Cleanup
        // SAFETY: test runs single-threaded; restoring env state.
        unsafe {
            env::remove_var("TMUX");
            match &original_home {
                Some(h) => env::set_var("HOME", h),
                None => env::remove_var("HOME"),
            }
        };

        // Assert — warnings only, should return None (continue)
        let code = result.unwrap();
        assert_eq!(
            code, None,
            "should return None when validation has only warnings"
        );
    }

    #[test]
    fn teardown_scp_session_handles_runner_errors() {
        // Arrange — register failing responses for tmux commands used in teardown
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("teardown_err.log");
        let runner = MockCommandRunner::new()
            .with_run_response(
                "tmux select-pane -P default",
                crate::external::CommandOutput {
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: String::from("no server"),
                },
            )
            .with_run_response(
                "tmux select-pane -T ",
                crate::external::CommandOutput {
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: String::from("no server"),
                },
            )
            .with_run_response(
                "tmux set-option -p -u @fterm_ssh_host",
                crate::external::CommandOutput {
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: String::from("no server"),
                },
            );
        let remote_hosts = vec![String::from("host1")];

        // Act / Assert — should not panic despite all tmux commands failing
        teardown_scp_session(&runner, &log_path, &remote_hosts, true, "0s", "");
    }

    #[test]
    fn setup_scp_session_handles_pane_errors() {
        // Arrange — start::start needs a valid log path; pane/window cmds fail
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("setup_err.log");
        let runner = MockCommandRunner::new()
            .with_run_response(
                "tmux select-pane -T scp:errhost",
                crate::external::CommandOutput {
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: String::from("no pane"),
                },
            )
            .with_run_response(
                "tmux set-window-option automatic-rename off",
                crate::external::CommandOutput {
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: String::from("no window"),
                },
            )
            .with_run_response(
                "tmux set-option -p @fterm_ssh_host errhost",
                crate::external::CommandOutput {
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: String::from("no pane"),
                },
            );

        // Act — setup should succeed (pane/window failures are just warnings)
        let result = setup_scp_session(&runner, &log_path, "errhost", &[], &[]);

        // Assert
        assert!(
            result.is_ok(),
            "setup should succeed even with pane errors: {result:?}"
        );
    }

    #[test]
    #[serial(env)]
    fn pre_connect_checks_with_valid_config_passes() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        // Create cm dir so cm_dir check passes
        std::fs::create_dir_all(ssh_dir.join("conf.d").join("cm")).unwrap();
        let config_path = ssh_dir.join("config");
        std::fs::write(&config_path, "Host testhost\n  HostName 192.168.1.1\n").unwrap();

        // Provide full ssh_resolve output so per-host checks pass
        let host_resolve = "hostname 192.168.1.1\nuser deploy\nport 22\nidentitiesonly yes\nidentityfile /dev/null\ncontrolpath /tmp/cm/%r@%h:%p\n";

        let original_home = env::var("HOME").ok();
        // SAFETY: test runs single-threaded; env vars are restored immediately.
        unsafe {
            env::set_var("TMUX", "/tmp/tmux-test/default,12345,0");
            env::set_var("HOME", tmp.path().to_str().unwrap());
        };
        let runner = MockCommandRunner::new()
            .with_agent_list(AgentListResult {
                available: true,
                keys: vec![],
            })
            .with_ssh_resolve(
                "syntax.check.dummy.host",
                "hostname syntax.check.dummy.host\n",
            )
            .with_ssh_resolve("testhost", host_resolve);
        let args = vec![
            String::from("scp"),
            String::from("file.txt"),
            String::from("testhost:~/"),
        ];
        let remote_hosts = vec![String::from("testhost")];

        // Act
        let result = pre_connect_checks(&runner, &args, &remote_hosts);

        // Cleanup
        // SAFETY: test runs single-threaded; restoring env state.
        unsafe {
            env::remove_var("TMUX");
            match &original_home {
                Some(h) => env::set_var("HOME", h),
                None => env::remove_var("HOME"),
            }
        };

        // Assert
        let code = result.unwrap();
        assert_eq!(code, None, "should return None when config is valid");
    }
}
