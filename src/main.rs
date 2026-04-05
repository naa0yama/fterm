//! fterm — SSH/SCP connection management tool.

mod cli;
pub mod command;
pub mod config;
pub mod external;
pub mod logging;
pub mod tmux;
pub mod util;
pub mod validate;

use std::io;

use anyhow::Context;
use clap::Parser;
use tracing_subscriber::filter::EnvFilter;
#[cfg(not(feature = "otel"))]
use tracing_subscriber::fmt;
#[cfg(feature = "otel")]
use tracing_subscriber::layer::SubscriberExt;
#[cfg(feature = "otel")]
use tracing_subscriber::util::SubscriberInitExt;

use crate::cli::{Cli, Commands};

// NOTEST(unreachable): process entry point; global init and process::exit are not unit-testable
fn main() {
    // NOTEST(env): OTel feature gate; compiled only with --features otel
    #[cfg(not(feature = "otel"))]
    {
        fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
            )
            .init();
    }

    #[cfg(feature = "otel")]
    {
        let env_filter =
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
        let fmt_layer = tracing_subscriber::fmt::layer();

        let otel_layer = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
            .ok()
            .and_then(|_| {
                let exporter = opentelemetry_otlp::SpanExporter::builder()
                    .with_http()
                    .build()
                    .ok()?;

                let tracer_provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
                    .with_simple_exporter(exporter)
                    .build();

                let tracer = opentelemetry::trace::TracerProvider::tracer(
                    &tracer_provider,
                    env!("CARGO_PKG_NAME"),
                );
                opentelemetry::global::set_tracer_provider(tracer_provider);

                Some(tracing_opentelemetry::layer().with_tracer(tracer))
            });

        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt_layer)
            .with(otel_layer)
            .init();
    }

    let cli = Cli::parse();
    let runner = external::RealCommandRunner::new();

    let exit_code = match cli.command {
        Commands::Fssh => run_subcommand(|| command::fssh::run(&runner)),
        Commands::Ssh(args) => run_subcommand(|| command::ssh::run(&runner, &args.args)),
        Commands::Scp(args) => run_subcommand(|| command::scp::run(&runner, &args.args)),
        Commands::Flog => run_subcommand(command::flog::run),
        Commands::Fgen => run_subcommand(command::fgen::run),
        Commands::SshAdd(args) => run_subcommand(|| command::ssh_add::run(&args.args)),
        Commands::SshKeygen(args) => run_subcommand(|| command::ssh_keygen::run(&args.args)),
        Commands::Completion(args) => {
            run_subcommand(|| command::completion::run(&args.shell, args.list_hosts))
        }
        Commands::LogFilter => run_subcommand(|| {
            run_log_filter()?;
            Ok(0)
        }),
    };

    #[allow(clippy::exit)]
    std::process::exit(exit_code);
}

/// Execute a subcommand, logging errors and returning the exit code.
fn run_subcommand<F: FnOnce() -> anyhow::Result<i32>>(f: F) -> i32 {
    match f() {
        Ok(code) => code,
        Err(e) => {
            tracing::error!("{e:#}");
            1
        }
    }
}

/// Run the log-filter subcommand: read stdin, strip ANSI, prepend timestamps.
fn run_log_filter() -> anyhow::Result<()> {
    let stdin = io::stdin().lock();
    let stdout = io::stdout().lock();
    logging::filter::process_stream(stdin, stdout).context("log-filter stream processing failed")
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn run_log_filter_processes_empty_stream() {
        // Arrange — empty input; process_stream reads from a Cursor<&[u8]>
        let input: &[u8] = b"";
        let stdin = Cursor::new(input);
        let mut stdout_buf = Vec::new();

        // Act
        let result = logging::filter::process_stream(stdin, &mut stdout_buf);

        // Assert — empty input produces no output and no error
        assert!(result.is_ok());
        assert!(stdout_buf.is_empty());
    }

    #[test]
    fn run_subcommand_ok_returns_exit_code() {
        // Arrange / Act
        let result = run_subcommand(|| Ok(42));

        // Assert
        assert_eq!(result, 42);
    }

    #[test]
    fn run_subcommand_err_returns_1() {
        // Arrange / Act
        let result = run_subcommand(|| Err(anyhow::anyhow!("something went wrong")));

        // Assert
        assert_eq!(result, 1);
    }

    #[test]
    fn run_subcommand_ok_zero_returns_0() {
        // Arrange / Act
        let result = run_subcommand(|| Ok(0));

        // Assert
        assert_eq!(result, 0);
    }
}
