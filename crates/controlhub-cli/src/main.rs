//! `controlhub` CLI.
//!
//! - `doctor`, `discover`, `plan`, `apply`: local and deterministic. `apply` only edits an
//!   allowlist of files, refuses when files changed since the plan, and needs `--yes`.
//! - `verify`, `context`, `mcp`: talk to a ControlHub instance. Tokens are read from environment
//!   variables only (never flags, which leak into shell history and process lists).
//!   `mcp` is a read-only MCP server on stdio.
//!
//! No command executes anything derived from telemetry or project files.

mod client;
mod context;
mod discover;
mod doctor;
mod mcp;
mod plan;
mod verify;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "controlhub", version, about = "Integrate a project with ControlHub and read context from it")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Clone, Args)]
pub struct Conn {
    /// ControlHub API base URL. Token: CONTROLHUB_API_TOKEN (read-only `cha_…`).
    #[arg(long, env = "CONTROLHUB_API_URL", default_value = "http://127.0.0.1:8080")]
    pub api_url: String,
    /// OTLP/HTTP ingestion endpoint. Token: CONTROLHUB_INGEST_TOKEN (`chi_…`).
    #[arg(long, env = "CONTROLHUB_OTLP_ENDPOINT", default_value = "http://127.0.0.1:4318")]
    pub ingest_url: String,
}

#[derive(Debug, Clone, Args)]
pub struct PlanArgs {
    /// Service name reported by the SDK (default: derived from package.json `name`).
    #[arg(long)]
    pub service_name: Option<String>,
    /// Deployment environment label written to `.env.example`.
    #[arg(long, default_value = "local")]
    pub environment: String,
    /// Version spec for `@controlhub/sdk-node` (it is not published to a registry yet; use a
    /// `file:` path to `packages/sdk-node` of a ControlHub checkout).
    #[arg(long, default_value = "^0.1.0")]
    pub sdk_spec: String,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Check configuration, connectivity and tokens.
    Doctor {
        #[arg(long, default_value = ".")]
        path: PathBuf,
        #[command(flatten)]
        conn: Conn,
    },
    /// Inspect a project (read-only; `.env` files are listed by name, never read).
    Discover {
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
    /// Print an integration plan as JSON (optionally write it to a file).
    Plan {
        #[arg(long, default_value = ".")]
        path: PathBuf,
        #[command(flatten)]
        plan: PlanArgs,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Apply a plan. Without `--yes` only shows the diff (exit code 2).
    Apply {
        #[arg(long, default_value = ".")]
        path: PathBuf,
        /// Plan file from `controlhub plan --out`. Without it a fresh plan is computed.
        #[arg(long)]
        plan: Option<PathBuf>,
        #[command(flatten)]
        plan_args: PlanArgs,
        #[arg(long)]
        yes: bool,
    },
    /// Send a verification event end to end and check that the app's telemetry arrives.
    Verify {
        #[arg(long, default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        service_name: Option<String>,
        #[arg(long, default_value_t = 60)]
        timeout_secs: u64,
        /// Fail when no telemetry from the application itself has been observed.
        #[arg(long)]
        require_app: bool,
        #[command(flatten)]
        conn: Conn,
    },
    /// Print a bounded, redacted JSON summary of the environment for assistants.
    Context {
        #[arg(long, default_value_t = 15, value_parser = clap::value_parser!(u32).range(1..=1440))]
        minutes: u32,
        #[command(flatten)]
        conn: Conn,
    },
    /// Run a read-only MCP server on stdio (token: CONTROLHUB_API_TOKEN).
    Mcp {
        #[command(flatten)]
        conn: Conn,
    },
}

/// Command outcome: printed JSON (stdout) plus exit code.
#[derive(Debug)]
pub struct Outcome {
    pub json: serde_json::Value,
    pub code: u8,
}

fn run(cli: Cli) -> anyhow::Result<Outcome> {
    match cli.command {
        Command::Doctor { path, conn } => doctor::run(&path, &conn),
        Command::Discover { path } => {
            let d = discover::discover(&path)?;
            Ok(Outcome { json: serde_json::to_value(d)?, code: 0 })
        }
        Command::Plan { path, plan, out } => {
            let p = plan::build(&path, &plan)?;
            if let Some(out) = out {
                std::fs::write(&out, serde_json::to_string_pretty(&p)? + "\n")?;
                eprintln!("plan written to {}", out.display());
            }
            Ok(Outcome { json: serde_json::to_value(p)?, code: 0 })
        }
        Command::Apply { path, plan, plan_args, yes } => {
            let p = match plan {
                Some(file) => plan::load(&file)?,
                None => plan::build(&path, &plan_args)?,
            };
            plan::apply(&path, &p, yes)
        }
        Command::Verify { path, service_name, timeout_secs, require_app, conn } => {
            verify::run(&path, service_name, timeout_secs, require_app, &conn)
        }
        Command::Context { minutes, conn } => context::run(minutes, &conn),
        Command::Mcp { conn } => mcp::run(&conn),
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(o) if o.json.is_null() => ExitCode::from(o.code),
        Ok(o) => {
            match serde_json::to_string_pretty(&o.json) {
                Ok(s) => println!("{s}"),
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            }
            ExitCode::from(o.code)
        }
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}
