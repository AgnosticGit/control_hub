//! karbird-executor: runs approved action plans.
//!
//! This is the only karbird process that changes the observed system, and the only one
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
use karbird_actions::{self as actions, Action, Observed, PolicyInput};
use karbird_domain::{EnvironmentId, ProjectId, Scope, TenantId};
use karbird_executor::verification::verification_record;
use karbird_runtime::{init_tracing, load_local_env, shutdown_signal};
use karbird_storage::actions::ActionPlanRow;
use karbird_storage::ch::{ChConfig, ChStore};
use karbird_storage::pg::PgStore;
use serde_json::{Value, json};

const LEASE_SECS: i64 = 60;
/// Telemetry needs a few seconds (SDK batching + processing) before it is queryable.
const INGEST_SETTLE: Duration = Duration::from_secs(8);

#[derive(Debug, Parser)]
#[command(name = "karbird-executor")]
struct Args {
    #[arg(long, env = "KARBIRD_EXECUTOR_DATABASE_URL")]
    database_url: String,
    #[arg(long, env = "KARBIRD_CLICKHOUSE_URL", default_value = "http://127.0.0.1:18123")]
    clickhouse_url: String,
    #[arg(long, env = "KARBIRD_CLICKHOUSE_DATABASE", default_value = "karbird")]
    clickhouse_database: String,
    #[arg(long, env = "KARBIRD_EXECUTOR_CLICKHOUSE_USER")]
    clickhouse_user: String,
    #[arg(long, env = "KARBIRD_EXECUTOR_CLICKHOUSE_PASSWORD")]
    clickhouse_password: String,
    #[arg(long, env = "KARBIRD_EXECUTOR_POLL_MS", default_value_t = 1000)]
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

#[derive(Debug)]
enum Outcome {
    Succeeded(Value, Value),
    VerificationFailed(Value, Value),
    Failed(Value),
}

impl Outcome {
    /// Final plan state, result and verification as stored by `finish_action_plan`.
    fn into_record(self) -> (&'static str, Value, Option<Value>) {
        match self {
            Outcome::Succeeded(r, v) => ("succeeded", r, Some(v)),
            Outcome::VerificationFailed(r, v) => ("verification_failed", r, Some(v)),
            Outcome::Failed(r) => ("failed", r, None),
        }
    }
}

impl Executor {
    async fn policy_allows(&self, scope: &Scope, action: &Action) -> anyhow::Result<Result<(), Value>> {
        let env = self.pg.environment(scope.project_id.as_str(), scope.environment_id.as_str()).await?;
        let is_demo = env.as_ref().is_some_and(|e| e.is_demo && e.tenant_id == scope.tenant_id.as_str());
        let open = self.pg.open_action_plans(scope, action.target()).await?;
        let d = actions::evaluate(
            action,
            &PolicyInput { environment_is_demo: is_demo, open_plans_for_target: open, approved_by_proposer: false },
            true,
        );
        Ok(if d.allowed { Ok(()) } else { Err(serde_json::to_value(d)?) })
    }

    async fn verify(
        &self,
        scope: &Scope,
        action: &Action,
        result: &Value,
        from_ns: i64,
    ) -> anyhow::Result<(bool, Value)> {
        let v = match action.verification_after(result) {
            Ok(v) => v,
            Err(e) => return Ok((false, json!({ "passed": false, "detail": e }))),
        };
        tokio::time::sleep(Duration::from_secs(v.observe_secs) + INGEST_SETTLE).await;
        let to_ns = from_ns + (v.observe_secs as i64) * 1_000_000_000;
        let row = self.ch.verification_window(scope, v.service, v.instance, v.expect_version, from_ns, to_ns).await?;
        let observed = Observed {
            requests: row.requests,
            errors: row.errors,
            instances: row.instances,
            other_version_requests: row.other_version_requests,
        };
        Ok(verification_record(&v, &observed, from_ns, to_ns))
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
        match self.verify(&scope, &action, &result, done).await {
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
        let (state, result, verification) = outcome.into_record();
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
    init_tracing("karbird-executor");
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
    tracing::info!(id = %executor.id, "karbird-executor running");
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
    tracing::info!("karbird-executor stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn row(action_type: &str, target: &str, params: Value) -> ActionPlanRow {
        let t = time::OffsetDateTime::UNIX_EPOCH;
        ActionPlanRow {
            id: uuid::Uuid::nil(),
            tenant_id: "local".into(),
            project_id: "demo".into(),
            environment_id: "local-demo".into(),
            action_type: action_type.into(),
            target: target.into(),
            params,
            reason: "reason".into(),
            state: "running".into(),
            policy: json!({}),
            created_by: uuid::Uuid::nil(),
            created_by_email: "op@example.test".into(),
            created_at: t,
            expires_at: t,
            decided_by: None,
            decided_by_email: None,
            decided_at: None,
            decision_comment: None,
            executor_id: Some("executor-test".into()),
            lease_until: None,
            started_at: None,
            finished_at: None,
            result: None,
            verification: None,
        }
    }

    fn executor() -> Executor {
        Executor {
            id: "executor-test".into(),
            // Closed local ports: any storage access fails instead of touching a real system.
            pg: PgStore::connect_lazy("postgres://u:p@127.0.0.1:1/karbird", 1).unwrap(),
            ch: ChStore::new(&ChConfig {
                url: "http://127.0.0.1:1".into(),
                database: "karbird".into(),
                user: "u".into(),
                password: "p".into(),
            }),
            demo: adapter::Demo::new(adapter::DemoConfig {
                gateway_url: "http://127.0.0.1:1".into(),
                orders_url: "http://127.0.0.1:1".into(),
                control_url: "http://127.0.0.1:1".into(),
                db_container: "karbird-demo-postgres-1".into(),
                docker_bin: "karbird-test-no-such-docker".into(),
            })
            .unwrap(),
        }
    }

    #[test]
    fn outcomes_map_to_final_states() {
        let (s, r, v) = Outcome::Succeeded(json!({ "a": 1 }), json!({ "passed": true })).into_record();
        assert_eq!((s, r, v), ("succeeded", json!({ "a": 1 }), Some(json!({ "passed": true }))));
        let (s, _, v) = Outcome::VerificationFailed(json!({}), json!({ "passed": false })).into_record();
        assert_eq!((s, v), ("verification_failed", Some(json!({ "passed": false }))));
        let (s, r, v) = Outcome::Failed(json!({ "error": "x" })).into_record();
        assert_eq!((s, r, v), ("failed", json!({ "error": "x" }), None));
        // Every final state is a terminal state of the plan lifecycle.
        for s in ["succeeded", "verification_failed", "failed"] {
            assert!(actions::State::parse(s).is_some_and(actions::State::is_terminal), "{s}");
        }
    }

    #[test]
    fn scope_is_taken_from_the_plan_row_and_validated() {
        let s = scope_of(&row("demo.faults.clear", "orders", json!({}))).unwrap();
        assert_eq!(
            (s.tenant_id.as_str(), s.project_id.as_str(), s.environment_id.as_str()),
            ("local", "demo", "local-demo")
        );
        for (t, p, e) in [("", "demo", "env"), ("local", "de mo", "env"), ("local", "demo", "e;x")] {
            let mut r = row("demo.faults.clear", "orders", json!({}));
            (r.tenant_id, r.project_id, r.environment_id) = (t.into(), p.into(), e.into());
            assert!(scope_of(&r).is_err(), "{t:?}/{p:?}/{e:?}");
        }
    }

    #[tokio::test]
    async fn tampered_plans_fail_revalidation_before_any_effect() {
        let ex = executor();
        for r in [
            row("shell.exec", "orders", json!({ "cmd": "rm -rf /" })),
            row("demo.faults.clear", "postgres", json!({})),
            row("demo.faults.inject", "orders", json!({ "latency_ms": 1e9, "error_rate": 0.1 })),
            row("demo.restart_database", "--privileged", json!({})),
            row("demo.container.restart", "demo-postgres", json!({})),
            row("demo.container.restart", "catalog-1; docker rm -f x", json!({})),
            row("demo.replicas.scale", "catalog", json!({ "replicas": 50 })),
            row("demo.feature_flag.set", "kill-switch", json!({ "enabled": 1 })),
            row("demo.release.rollback", "orders", json!({ "version": "../../etc" })),
        ] {
            match ex.execute(&r).await {
                Outcome::Failed(v) => assert_eq!(v["error"], "plan failed re-validation", "{v}"),
                other => panic!("{} must fail re-validation, got {other:?}", r.action_type),
            }
        }
    }

    #[tokio::test]
    async fn invalid_scope_fails_before_policy_or_effect() {
        let mut r = row("demo.faults.clear", "orders", json!({}));
        r.tenant_id = "bad tenant".into();
        match executor().execute(&r).await {
            Outcome::Failed(v) => assert!(v["error"].as_str().unwrap().starts_with("invalid scope"), "{v}"),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn unreachable_policy_store_fails_closed() {
        // Valid plan, but the policy cannot be evaluated: it must not run.
        match executor().execute(&row("demo.faults.clear", "orders", json!({}))).await {
            Outcome::Failed(v) => assert!(v["error"].as_str().unwrap().starts_with("policy check failed"), "{v}"),
            other => panic!("{other:?}"),
        }
    }

    fn demo_scope() -> Scope {
        scope_of(&row("demo.faults.clear", "orders", json!({}))).unwrap()
    }

    #[tokio::test]
    async fn rollback_without_an_allowlisted_release_fails_verification_without_querying() {
        // ClickHouse is unreachable here: an answer proves no query was attempted.
        let rollback = Action::DemoReleaseRollback { service: actions::DemoReleaseService::Orders };
        for result in [json!({}), json!({ "to_version": "7.7.7" })] {
            let (passed, v) = executor().verify(&demo_scope(), &rollback, &result, 0).await.unwrap();
            assert!(!passed);
            assert_eq!(v["passed"], false);
            assert!(v["detail"].as_str().unwrap().contains("allowlisted release"), "{v}");
        }
    }

    #[test]
    fn verification_records_carry_what_was_checked() {
        let scale = Action::DemoReplicasScale { service: actions::DemoScalable::Catalog, replicas: 2 }.verification();
        let o = Observed { requests: 30, errors: 0, instances: 2, other_version_requests: 0 };
        let (passed, v) = verification_record(&scale, &o, 1, 15_000_000_001);
        assert!(passed);
        assert_eq!(
            (v["service"].as_str(), v["expected_instances"].as_u64(), v["instances"].as_u64()),
            (Some("catalog"), Some(2), Some(2))
        );
        assert_eq!((v["window_from_ns"].as_str(), v["window_to_ns"].as_str()), (Some("1"), Some("15000000001")));
        assert_eq!(v["instance"], Value::Null);

        let restart = Action::DemoContainerRestart { container: actions::DemoContainer::Catalog1 }.verification();
        let (passed, v) = verification_record(&restart, &Observed::default(), 0, 1);
        assert!(!passed);
        assert_eq!(v["instance"], "catalog-1");
        assert!(v["detail"].as_str().unwrap().contains("received 0 requests"));

        let rollback = Action::DemoReleaseRollback { service: actions::DemoReleaseService::Orders }
            .verification_after(&json!({ "to_version": "0.1.0" }))
            .unwrap();
        let (passed, v) = verification_record(&rollback, &Observed { other_version_requests: 1, ..o }, 0, 1);
        assert!(!passed);
        assert_eq!((v["expected_version"].as_str(), v["other_version_requests"].as_u64()), (Some("0.1.0"), Some(1)));
    }

    #[test]
    fn args_require_database_url_and_validate_numbers() {
        let base = ["karbird-executor", "--clickhouse-user", "u", "--clickhouse-password", "p"];
        if std::env::var_os("KARBIRD_EXECUTOR_DATABASE_URL").is_none() {
            assert!(Args::try_parse_from(base).is_err(), "database URL is required");
        }
        let mut argv = base.to_vec();
        argv.extend(["--database-url", "postgres://x@127.0.0.1/db", "--poll-ms", "250"]);
        let a = Args::try_parse_from(&argv).unwrap();
        assert_eq!(a.poll_ms, 250);
        argv.extend(["--poll-ms", "-1"]);
        assert!(Args::try_parse_from(&argv).is_err());
    }
}
