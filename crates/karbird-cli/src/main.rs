//! `karbird` CLI.
//!
//! - `doctor`, `discover`, `plan`, `apply`: local and deterministic. `apply` only edits an
//!   allowlist of files, refuses when files changed since the plan, and needs `--yes`.
//! - `verify`, `context`, `mcp`: talk to a karbird instance. Tokens are read from environment
//!   variables only (never flags, which leak into shell history and process lists).
//!   `mcp` is a read-only MCP server on stdio.
//!
//! No command executes anything derived from telemetry or project files.

mod client;
mod context;
use karbird_cli::discover;
mod doctor;
mod mcp;
mod plan;
#[cfg(test)]
mod test_http;
mod verify;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "karbird", version, about = "Integrate a project with karbird and read context from it")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Clone, Args)]
pub struct Conn {
    /// karbird API base URL. Token: KARBIRD_API_TOKEN (read-only `cha_…`).
    #[arg(long, env = "KARBIRD_API_URL", default_value = "http://127.0.0.1:8080")]
    pub api_url: String,
    /// Optional public Karbird web URL used only for MCP links.
    #[arg(long, env = "KARBIRD_WEB_URL")]
    pub web_url: Option<String>,
    /// OTLP/HTTP ingestion endpoint. Token: KARBIRD_INGEST_TOKEN (`chi_…`).
    #[arg(long, env = "KARBIRD_OTLP_ENDPOINT", default_value = "http://127.0.0.1:4318")]
    pub ingest_url: String,
}

#[derive(Debug, Clone, Args)]
pub struct PlanArgs {
    /// Service name reported by the SDK (default: derived from package.json `name` or composer.json `name`).
    #[arg(long)]
    pub service_name: Option<String>,
    /// Deployment environment label written to `.env.example`.
    #[arg(long, default_value = "local")]
    pub environment: String,
    /// SDK version constraint. For a local PHP SDK checkout, use `path:../karbird/packages/sdk-php`.
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
        /// Plan file from `karbird plan --out`. Without it a fresh plan is computed.
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
    /// Run a read-only MCP server on stdio (token: KARBIRD_API_TOKEN).
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("karbird").chain(args.iter().copied()))
    }

    #[test]
    fn the_cli_definition_is_consistent() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }

    #[test]
    fn tokens_cannot_be_passed_as_flags() {
        for cmd in ["doctor", "verify", "context", "mcp"] {
            for flag in ["--token", "--api-token", "--ingest-token"] {
                assert!(parse(&[cmd, flag, "cha_x"]).is_err(), "{cmd} {flag}");
            }
        }
    }

    #[test]
    fn context_minutes_are_bounded() {
        for bad in ["0", "1441", "-1", "abc"] {
            assert!(parse(&["context", "--minutes", bad]).is_err(), "{bad}");
        }
        for (arg, want) in [("1", 1), ("1440", 1440)] {
            match parse(&["context", "--minutes", arg]).unwrap().command {
                Command::Context { minutes, .. } => assert_eq!(minutes, want),
                c => panic!("{c:?}"),
            }
        }
    }

    #[test]
    fn apply_needs_an_explicit_yes() {
        match parse(&["apply"]).unwrap().command {
            Command::Apply { yes, plan, path, .. } => {
                assert!(!yes);
                assert!(plan.is_none());
                assert_eq!(path, PathBuf::from("."));
            }
            c => panic!("{c:?}"),
        }
        match parse(&["apply", "--yes", "--plan", "p.json"]).unwrap().command {
            Command::Apply { yes, plan, .. } => assert!(yes && plan == Some(PathBuf::from("p.json"))),
            c => panic!("{c:?}"),
        }
        assert!(parse(&["apply", "--yes=false"]).is_err(), "--yes is a flag, not a value");
    }

    #[test]
    fn verify_and_plan_defaults() {
        match parse(&["verify", "--api-url", "https://hub.example.com"]).unwrap().command {
            Command::Verify { timeout_secs, require_app, service_name, conn, .. } => {
                assert_eq!((timeout_secs, require_app, service_name), (60, false, None));
                assert_eq!(conn.api_url, "https://hub.example.com");
            }
            c => panic!("{c:?}"),
        }
        match parse(&["plan"]).unwrap().command {
            Command::Plan { plan, out, .. } => {
                assert_eq!((plan.environment.as_str(), plan.sdk_spec.as_str()), ("local", "^0.1.0"));
                assert!(out.is_none() && plan.service_name.is_none());
            }
            c => panic!("{c:?}"),
        }
        assert!(parse(&["verify", "--timeout-secs", "-5"]).is_err());
        assert!(parse(&[]).is_err(), "a subcommand is required");
        assert!(parse(&["shell"]).is_err());
    }

    #[test]
    fn mcp_accepts_an_explicit_web_url() {
        assert!(parse(&["mcp", "--web-url", "https://karbird.example"]).is_ok());
    }

    #[test]
    fn discover_and_plan_run_without_the_network() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"name":"@acme/shop","scripts":{"start":"node server.js"}}"#,
        )
        .unwrap();
        let p = dir.path().to_str().unwrap();
        let o = run(parse(&["discover", "--path", p]).unwrap()).unwrap();
        assert_eq!((o.code, &o.json["package_name"]), (0, &serde_json::json!("@acme/shop")));

        let out = dir.path().join("plan.json");
        let o = run(parse(&["plan", "--path", p, "--out", out.to_str().unwrap()]).unwrap()).unwrap();
        assert_eq!(o.code, 0);
        let written: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
        assert_eq!(written, o.json, "--out writes exactly the printed plan");

        let before = std::fs::read_to_string(dir.path().join("package.json")).unwrap();
        let o = run(parse(&["apply", "--path", p, "--plan", out.to_str().unwrap()]).unwrap()).unwrap();
        assert_eq!(o.code, 2, "without --yes apply only previews");
        assert_eq!(std::fs::read_to_string(dir.path().join("package.json")).unwrap(), before);
    }

    #[test]
    fn discover_of_a_missing_directory_is_an_error() {
        assert!(run(parse(&["discover", "--path", "does/not/exist/anywhere"]).unwrap()).is_err());
    }
}
