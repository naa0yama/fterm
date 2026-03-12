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

fn main() {
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
