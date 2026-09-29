//! controlhub-executor: runs approved action plans.
//!
//! This is the only ControlHub process that changes the observed system, and the only one
//! allowed to use Docker (web, API and ingestion never get the Docker socket). It:
//! 1. claims an approved plan (PostgreSQL `FOR UPDATE SKIP LOCKED`, with a lease),
//! 2. re-validates the action and re-evaluates the policy,
//! 3. runs the allowlisted adapter step with fixed arguments (no shell, nothing from telemetry),
//! 4. verifies the outcome from the adapter and from telemetry,
//! 5. records the result; every state change is committed with its audit entry.
//!
//! A plan whose executor disappears is marked failed (outcome unknown) and never retried.

mod adapter;

use std::time::Duration;

use clap::Parser;
use controlhub_actions::{self as actions, Action, PolicyInput};
use controlhub_domain::{EnvironmentId, ProjectId, Scope, TenantId};
use controlhub_runtime::{init_tracing, load_local_env, shutdown_signal};
use controlhub_storage::actions::ActionPlanRow;
use controlhub_storage::ch::{ChConfig, ChStore};
use controlhub_storage::pg::PgStore;
use serde_json::{Value, json};

const LEASE_SECS: i64 = 60;
/// Telemetry needs a few seconds (SDK batching + processing) before it is queryable.
const INGEST_SETTLE: Duration = Duration::from_secs(8);

#[derive(Debug, Parser)]
#[command(name = "controlhub-executor")]
struct Args {
    #[arg(long, env = "CONTROLHUB_EXECUTOR_DATABASE_URL")]
    database_url: String,
    #[arg(long, env = "CONTROLHUB_CLICKHOUSE_URL", default_value = "http://127.0.0.1:18123")]
    clickhouse_url: String,
    #[arg(long, env = "CONTROLHUB_CLICKHOUSE_DATABASE", default_value = "controlhub")]
    clickhouse_database: String,
    #[arg(long, env = "CONTROLHUB_EXECUTOR_CLICKHOUSE_USER")]
    clickhouse_user: String,
    #[arg(long, env = "CONTROLHUB_EXECUTOR_CLICKHOUSE_PASSWORD")]
    clickhouse_password: String,
    #[arg(long, env = "CONTROLHUB_EXECUTOR_POLL_MS", default_value_t = 1000)]
    poll_ms: u64,
    #[command(flatten)]
    adapter: adapter::DemoConfig,
}

fn scope_of(row: &ActionPlanRow) -> anyhow::Result<Scope> {
    Ok(Scope {
        tenant_id: TenantId::parse(row.tenant_id.clone())?,
        project_id: ProjectId::parse(row.project_id.clone())?,
        environment_id: EnvironmentId::parse(row.environment_id.clone())?,
    })
}

fn now_ns() -> i64 {
    (time::OffsetDateTime::now_utc().unix_timestamp_nanos()) as i64
}

struct Executor {
    id: String,
    pg: PgStore,
    ch: ChStore,
    demo: adapter::Demo,
}

enum Outcome {
    Succeeded(Value, Value),
    VerificationFailed(Value, Value),
    Failed(Value),
}

impl Executor {
    async fn policy_allows(&self, scope: &Scope, action: &Action) -> anyhow::Result<Result<(), Value>> {
        let env = self.pg.environment(scope.project_id.as_str(), scope.environment_id.as_str()).await?;
        let is_demo = env.as_ref().is_some_and(|e| e.is_demo && e.tenant_id == scope.tenant_id.as_str());
        let (open, recent) = self.pg.action_counts(scope, action.target()).await?;
        let d = actions::evaluate(
            action,
            &PolicyInput { environment_is_demo: is_demo, open_plans_for_target: open, plans_last_hour: recent },
            true,
        );
        Ok(if d.allowed { Ok(()) } else { Err(serde_json::to_value(d)?) })
    }

    async fn verify(&self, scope: &Scope, action: &Action, from_ns: i64) -> anyhow::Result<(bool, Value)> {
        let v = action.verification();
        tokio::time::sleep(Duration::from_secs(v.observe_secs) + INGEST_SETTLE).await;
        let to_ns = from_ns + (v.observe_secs as i64) * 1_000_000_000;
        let rows = self.ch.service_summaries(scope, from_ns, to_ns).await?;
        let (requests, errors) = rows
            .iter()
            .filter(|r| r.service_name == v.service)
            .fold((0, 0), |(rq, er), r| (rq + r.requests, er + r.errors));
        let (passed, detail) = v.judge(requests, errors);
        Ok((
            passed,
            json!({
                "service": v.service, "window_from_ns": from_ns.to_string(), "window_to_ns": to_ns.to_string(),
                "requests": requests, "errors": errors, "max_error_ratio": v.max_error_ratio, "passed": passed, "detail": detail,
            }),
        ))
    }

    async fn execute(&self, row: &ActionPlanRow) -> Outcome {
        let action = match actions::validate(&row.action_type, &row.target, &row.params) {
            Ok(a) => a,
            Err(e) => return Outcome::Failed(json!({ "error": "plan failed re-validation", "details": e })),
        };
        let scope = match scope_of(row) {
            Ok(s) => s,
            Err(e) => return Outcome::Failed(json!({ "error": format!("invalid scope: {e}") })),
        };
        match self.policy_allows(&scope, &action).await {
            Ok(Ok(())) => {}
            Ok(Err(policy)) => {
                return Outcome::Failed(json!({ "error": "denied by policy at execution time", "policy": policy }));
            }
            Err(e) => return Outcome::Failed(json!({ "error": format!("policy check failed: {e}") })),
        }
        tracing::info!(plan = %row.id, action = action.action_type(), target = action.target(), "executing");
        let result = match self.demo.run(&action).await {
            Ok(r) => r,
            Err(e) => return Outcome::Failed(json!({ "error": format!("{e:#}") })),
        };
        let done = now_ns();
        match self.verify(&scope, &action, done).await {
            Ok((true, v)) => Outcome::Succeeded(result, v),
            Ok((false, v)) => Outcome::VerificationFailed(result, v),
            Err(e) => Outcome::VerificationFailed(
                result,
                json!({ "passed": false, "detail": format!("verification query failed: {e}") }),
            ),
        }
    }

    async fn run_one(&self, row: ActionPlanRow) -> anyhow::Result<()> {
        // Keep the lease alive while the action and its verification run.
        let lease = {
            let (pg, id, exec) = (self.pg.clone(), row.id, self.id.clone());
            tokio::spawn(async move {
                let mut tick = tokio::time::interval(Duration::from_secs((LEASE_SECS / 3) as u64));
                tick.tick().await;
                loop {
                    tick.tick().await;
                    if !matches!(pg.extend_action_lease(id, &exec, LEASE_SECS).await, Ok(true)) {
                        break;
                    }
                }
            })
        };
        let outcome = self.execute(&row).await;
        lease.abort();
        let (state, result, verification) = match outcome {
            Outcome::Succeeded(r, v) => ("succeeded", r, Some(v)),
            Outcome::VerificationFailed(r, v) => ("verification_failed", r, Some(v)),
            Outcome::Failed(r) => ("failed", r, None),
        };
        tracing::info!(plan = %row.id, state, "finished");
        if !self.pg.finish_action_plan(row.id, &self.id, state, &result, verification.as_ref()).await? {
            tracing::error!(plan = %row.id, "could not record the result: plan no longer held by this executor");
        }
        Ok(())
    }

    async fn tick(&self) -> anyhow::Result<bool> {
        let expired = self.pg.expire_action_plans(&self.id).await?;
        let lost = self.pg.fail_lost_action_plans(&self.id).await?;
        if expired + lost > 0 {
            tracing::warn!(expired, lost, "housekeeping");
        }
        match self.pg.claim_action_plan(&self.id, LEASE_SECS).await? {
            Some(row) => {
                self.run_one(row).await?;
                Ok(true)
            }
            None => Ok(false),
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    load_local_env();
    init_tracing("controlhub-executor");
    let args = Args::parse();
    let host = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_else(|_| "local".into());
    let executor = Executor {
        id: format!("executor-{host}-{}-{}", std::process::id(), &uuid::Uuid::new_v4().simple().to_string()[..8]),
        pg: PgStore::connect(&args.database_url, 4).await?,
        ch: ChStore::new(&ChConfig {
            url: args.clickhouse_url,
            database: args.clickhouse_database,
            user: args.clickhouse_user,
            password: args.clickhouse_password,
        }),
        demo: adapter::Demo::new(args.adapter)?,
    };
    tracing::info!(id = %executor.id, "controlhub-executor running");
    let poll = Duration::from_millis(args.poll_ms.max(200));
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);
    loop {
        let worked = match executor.tick().await {
            Ok(w) => w,
            Err(e) => {
                tracing::error!(error = %e, "executor tick failed");
                false
            }
        };
        if worked {
            continue;
        }
        tokio::select! {
            _ = &mut shutdown => break,
            _ = tokio::time::sleep(poll) => {}
        }
    }
    tracing::info!("controlhub-executor stopped");
    Ok(())
}
