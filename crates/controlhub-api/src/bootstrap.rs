//! Local development bootstrap: migrations, least-privilege service
//! credentials, the demo tenant/project/environment and its ingestion token, and the one
//! account the developer configured in `.controlhub/owner.env`. Test accounts are not created
//! here (see `test_fixtures`). Writes generated secrets to the gitignored `.controlhub/` only.

use std::path::Path;

use anyhow::Context;
use controlhub_domain::{EnvironmentId, ProjectId, Scope, TenantId};
use controlhub_runtime::env_required;
use controlhub_storage::ch::{ChConfig, ChStore};
use controlhub_storage::pg::PgStore;
use controlhub_storage::retention::{self, Decision, Table};
use controlhub_storage::secrets::{generate_token, hash_password, hash_token, token_prefix};
use serde_json::json;

use crate::permissions::{self, Builtin};
use crate::routes::signup::{MAX_PASSWORD_BYTES, MIN_PASSWORD_CHARS, normalise_email};
use crate::system_permissions::SystemBuiltin;

const CH_MIGRATIONS: &[(&str, &str)] = &[
    ("0001_telemetry.sql", include_str!("../../../infra/migrations/clickhouse/0001_telemetry.sql")),
    (
        "0002_log_templates_and_codecs.sql",
        include_str!("../../../infra/migrations/clickhouse/0002_log_templates_and_codecs.sql"),
    ),
];

pub(crate) struct SeedScope {
    pub tenant: &'static str,
    pub project: &'static str,
    pub env: &'static str,
    pub tenant_name: &'static str,
    pub project_name: &'static str,
    pub env_name: &'static str,
    pub is_demo: bool,
}

impl SeedScope {
    pub fn scope(&self) -> anyhow::Result<Scope> {
        Ok(Scope {
            tenant_id: TenantId::parse(self.tenant)?,
            project_id: ProjectId::parse(self.project)?,
            environment_id: EnvironmentId::parse(self.env)?,
        })
    }
}

const DEMO_SCOPE: SeedScope = SeedScope {
    tenant: "local",
    project: "demo",
    env: "local-demo",
    tenant_name: "Local",
    project_name: "Demo system",
    env_name: "local-demo",
    is_demo: true,
};

/// Revokes the active ingestion tokens with this name in `scope` and issues a new one.
pub(crate) async fn rotate_ingest_token(pg: &PgStore, scope: &Scope, name: &str) -> anyhow::Result<String> {
    pg.revoke_ingest_tokens_named(scope, name).await?;
    let token = generate_token("chi_")?;
    pg.create_ingest_token(scope, name, &hash_token(&token), &token_prefix(&token), None, None).await?;
    Ok(token)
}

#[derive(Debug, PartialEq, Eq)]
struct Owner {
    email: String,
    password: String,
}

/// Email domains reserved for accounts that tests create; they are deleted after test runs and
/// by every bootstrap, so the owner may not use them.
pub(crate) const TEST_EMAIL_DOMAINS: &[&str] = &["controlhub.local", "example.test"];

/// The developer's own account from `CONTROLHUB_OWNER_EMAIL` / `CONTROLHUB_OWNER_PASSWORD`,
/// the only account a bootstrap creates. Both are required.
fn configured_owner(email: Option<&str>, password: Option<&str>) -> anyhow::Result<Owner> {
    let email = email.map(str::trim).filter(|s| !s.is_empty());
    let password = password.filter(|s| !s.is_empty());
    match (email, password) {
        (Some(e), Some(p)) => {
            let email =
                normalise_email(e).map_err(|_| anyhow::anyhow!("CONTROLHUB_OWNER_EMAIL is not a valid email"))?;
            let domain = email.rsplit('@').next().unwrap_or_default();
            anyhow::ensure!(
                !TEST_EMAIL_DOMAINS.contains(&domain),
                "CONTROLHUB_OWNER_EMAIL uses @{domain}, which is reserved for test accounts (deleted after test runs)"
            );
            anyhow::ensure!(
                p.len() <= MAX_PASSWORD_BYTES,
                "CONTROLHUB_OWNER_PASSWORD is longer than {MAX_PASSWORD_BYTES} bytes"
            );
            Ok(Owner { email, password: p.to_owned() })
        }
        _ => anyhow::bail!(
            "no owner account configured: put CONTROLHUB_OWNER_EMAIL and CONTROLHUB_OWNER_PASSWORD into .controlhub/owner.env (gitignored)"
        ),
    }
}

/// Admin connection to PostgreSQL (bootstrap and test fixtures only).
pub(crate) async fn admin_pg() -> anyhow::Result<PgStore> {
    let pg_port = std::env::var("CH_PG_PORT").unwrap_or_else(|_| "15432".into());
    let pg_admin_url = format!(
        "postgres://{}:{}@127.0.0.1:{pg_port}/controlhub",
        env_required("CH_PG_ADMIN_USER")?,
        env_required("CH_PG_ADMIN_PASSWORD")?
    );
    PgStore::connect(&pg_admin_url, 2).await.context("connect postgres as admin")
}

/// Sets each table's TTL to the configured retention. Lengthening applies at once; shortening
/// that would delete rows needs `allow_shrink` (`just bootstrap --shrink-retention`).
async fn apply_retention(ch: &ChStore, wanted: &[(Table, u32)], allow_shrink: bool) -> anyhow::Result<()> {
    for &(table, days) in wanted {
        let current = ch.ttl_days(table).await?.ok_or_else(|| {
            anyhow::anyhow!(
                "ClickHouse table `{}` has a TTL ControlHub does not recognise; set it to \
                 `TTL toDateTime({}) + INTERVAL N DAY` by hand, then run bootstrap again",
                table.name(),
                table.time_column()
            )
        })?;
        let rows_lost = if days < current { ch.rows_older_than(table, days).await? } else { 0 };
        match retention::decide(table, current, days, rows_lost, allow_shrink) {
            Decision::Keep => {}
            Decision::Apply => {
                ch.set_ttl_days(table, days).await?;
                tracing::info!(table = table.name(), from = current, to = days, rows_lost, "retention changed");
            }
            Decision::Refuse(msg) => anyhow::bail!(msg),
        }
    }
    Ok(())
}

pub async fn run(out_dir: &Path, retention: &[(Table, u32)], allow_shrink: bool) -> anyhow::Result<()> {
    // Checked first, so a missing owner.env fails before anything is changed.
    let owner = configured_owner(
        std::env::var("CONTROLHUB_OWNER_EMAIL").ok().as_deref(),
        std::env::var("CONTROLHUB_OWNER_PASSWORD").ok().as_deref(),
    )?;
    let pg = admin_pg().await?;
    pg.migrate().await.context("postgres migrations")?;
    tracing::info!("postgres migrations applied");
    for b in Builtin::ALL {
        let updated = pg.sync_builtin_role(b.key(), &permissions::to_strings(&b.permissions())).await?;
        tracing::debug!(role = b.key(), updated, "built-in role permissions synced");
    }
    for b in SystemBuiltin::ALL {
        let updated = pg.sync_system_builtin_role(b.key(), &permissions::to_strings(&b.permissions())).await?;
        tracing::debug!(role = b.key(), updated, "built-in system role permissions synced");
    }

    pg.ensure_service_role(
        "controlhub_api",
        &env_required("CH_PG_API_PASSWORD")?,
        &[
            "SELECT ON consumer_offsets, processing_errors",
            // INSERT only (no UPDATE/DELETE): self-service sign-up creates a user with a new tenant.
            "SELECT, INSERT ON tenants, projects, environments, users",
            // Users edit their own profile, password and onboarding state; project admins rename projects.
            "UPDATE (display_name, password_hash, onboarding_completed_at) ON users",
            "UPDATE (name) ON projects",
            // ControlHub administrators (permission checks in the API): their grants and role
            // templates, disabling accounts, and deleting a project only through the definer function.
            "SELECT, INSERT, UPDATE, DELETE ON system_memberships, system_roles",
            "UPDATE (disabled_at) ON users",
            "EXECUTE ON FUNCTION controlhub_delete_project(text)",
            // Members, role templates and invite links are managed from the web app (permission checks in the API).
            "SELECT, INSERT, UPDATE, DELETE ON memberships, project_roles",
            "SELECT, INSERT, UPDATE ON project_invites",
            "SELECT, INSERT, UPDATE ON sessions",
            "SELECT, INSERT ON audit_log",
            "SELECT, INSERT, UPDATE ON ingest_tokens, api_tokens",
            "SELECT, INSERT, UPDATE ON action_plans",
            // Reads archived logs (ADR-0015); only the archiver writes the catalog.
            "SELECT ON archive_segments, archive_dictionaries",
        ],
    )
    .await?;
    // The archiver writes the archive catalog and reads projects to find deleted ones.
    pg.ensure_service_role(
        "controlhub_archiver",
        &env_required("CH_PG_ARCHIVER_PASSWORD")?,
        &[
            "SELECT, INSERT, UPDATE, DELETE ON archive_segments, archive_dictionaries, archive_windows",
            "SELECT ON projects",
        ],
    )
    .await?;
    // The executor is the only component that runs actions (and the only one with Docker access).
    pg.ensure_service_role(
        "controlhub_executor",
        &env_required("CH_PG_EXECUTOR_PASSWORD")?,
        &["SELECT, UPDATE ON action_plans", "SELECT ON environments, users", "INSERT ON audit_log"],
    )
    .await?;
    pg.ensure_service_role("controlhub_ingest", &env_required("CH_PG_INGEST_PASSWORD")?, &["SELECT ON ingest_tokens"])
        .await?;
    pg.ensure_service_role(
        "controlhub_processor",
        &env_required("CH_PG_PROCESSOR_PASSWORD")?,
        &["SELECT, INSERT, UPDATE ON consumer_offsets", "SELECT, INSERT ON processing_errors"],
    )
    .await?;

    let ch_port = std::env::var("CH_CLICKHOUSE_HTTP_PORT").unwrap_or_else(|_| "18123".into());
    let ch = ChStore::new(&ChConfig {
        url: format!("http://127.0.0.1:{ch_port}"),
        database: "controlhub".into(),
        user: env_required("CH_CH_ADMIN_USER")?,
        password: env_required("CH_CH_ADMIN_PASSWORD")?,
    });
    let applied = ch.migrate(CH_MIGRATIONS).await.context("clickhouse migrations")?;
    tracing::info!(?applied, "clickhouse migrations applied");
    apply_retention(&ch, retention, allow_shrink).await?;
    ch.ensure_user("controlhub_api", &env_required("CH_CH_API_PASSWORD")?, &["SELECT ON controlhub.*"]).await?;
    ch.ensure_user(
        "controlhub_processor",
        &env_required("CH_CH_PROCESSOR_PASSWORD")?,
        &["INSERT, SELECT ON controlhub.*"],
    )
    .await?;
    // Read-only: the archiver copies logs out; ClickHouse's TTL deletes them.
    ch.ensure_user("controlhub_archiver", &env_required("CH_CH_ARCHIVER_PASSWORD")?, &["SELECT ON controlhub.logs"])
        .await?;
    // Read-only: the executor verifies outcomes from telemetry.
    ch.ensure_user("controlhub_executor", &env_required("CH_CH_EXECUTOR_PASSWORD")?, &["SELECT ON controlhub.*"])
        .await?;

    let demo = DEMO_SCOPE.scope()?;
    pg.upsert_scope(&demo, DEMO_SCOPE.tenant_name, DEMO_SCOPE.project_name, DEMO_SCOPE.env_name, DEMO_SCOPE.is_demo)
        .await?;

    // The only account: the developer's own, password from the gitignored owner.env, so it
    // survives `just reset`. Accounts left by tests or by older bootstraps are removed.
    let removed = pg.delete_users_in_domains(TEST_EMAIL_DOMAINS).await?;
    if removed > 0 {
        tracing::info!(removed, "test accounts removed");
    }
    if owner.password.chars().count() < MIN_PASSWORD_CHARS {
        tracing::warn!(
            "the owner password is shorter than {MIN_PASSWORD_CHARS} characters (the sign-up minimum); fine for local dev only"
        );
    }
    let pw = owner.password.clone();
    let hash = tokio::task::spawn_blocking(move || hash_password(&pw)).await??;
    let name = owner.email.split('@').next().unwrap_or_default().to_owned();
    let id = pg.upsert_user(&owner.email, &name, &hash).await?;
    pg.upsert_membership(id, "demo", Builtin::Owner.key()).await?;
    pg.upsert_system_builtin(id, SystemBuiltin::SuperAdmin.key()).await?;
    tracing::info!(email = %owner.email, "owner account ensured (owner of the demo project, ControlHub Super Admin)");

    let mut tokens = serde_json::Map::new();
    tokens.insert("demo-local".into(), json!(rotate_ingest_token(&pg, &demo, "demo-local").await?));

    std::fs::create_dir_all(out_dir)?;
    let creds = json!({
        "warning": "Local development credentials. Generated by `just bootstrap`; rotated on every run. Never commit.",
        "web_url": "http://localhost:3000/app",
        "api_url": "http://127.0.0.1:8080",
        "otlp_http_endpoint": "http://127.0.0.1:4318",
        "project_id": "demo",
        "environment_id": "local-demo",
        "owner": {
            "email": owner.email,
            "project_id": "demo",
            "role": "owner",
            "password": "set in .controlhub/owner.env",
        },
        "ingest_tokens": tokens,
    });
    std::fs::write(out_dir.join("local-credentials.json"), serde_json::to_string_pretty(&creds)?)?;
    let demo_token = tokens.get("demo-local").and_then(|v| v.as_str()).unwrap_or_default();
    std::fs::write(
        out_dir.join("demo.env"),
        format!(
            "# Generated by just bootstrap. Local demo ingestion credentials.\nCONTROLHUB_INGEST_TOKEN={demo_token}\nCONTROLHUB_OTLP_ENDPOINT=http://127.0.0.1:4318\nCONTROLHUB_ENVIRONMENT=local-demo\n"
        ),
    )?;
    tracing::info!(path = %out_dir.join("local-credentials.json").display(), "local credentials written");
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn bootstrap_refuses_to_run_without_an_owner_account() {
        let msg = |e: anyhow::Error| e.to_string();
        assert!(msg(configured_owner(None, None).unwrap_err()).contains(".controlhub/owner.env"));
        assert!(
            msg(configured_owner(Some("  "), Some("")).unwrap_err()).contains("no owner account"),
            "empty lines in owner.env count as unset"
        );
    }

    #[test]
    fn owner_email_is_normalised_and_the_password_kept_verbatim() {
        let o = configured_owner(Some(" Me@Example.COM "), Some(" pass word ")).unwrap();
        assert_eq!(o, Owner { email: "me@example.com".into(), password: " pass word ".into() });
    }

    #[test]
    fn the_owner_cannot_use_a_test_domain_that_test_runs_delete() {
        for email in ["me@controlhub.local", "Me@Example.Test"] {
            let err = configured_owner(Some(email), Some("long enough password")).unwrap_err().to_string();
            assert!(err.contains("reserved for test accounts"), "{err}");
        }
        assert!(configured_owner(Some("me@controlhub.local.example.com"), Some("pw")).is_ok());
    }

    #[test]
    fn half_configured_or_invalid_owner_fails_the_bootstrap() {
        let msg = |e: anyhow::Error| e.to_string();
        assert!(msg(configured_owner(Some("me@example.com"), None).unwrap_err()).contains("no owner account"));
        assert!(msg(configured_owner(None, Some("secret")).unwrap_err()).contains("no owner account"));
        assert!(msg(configured_owner(Some("not-an-email"), Some("secret")).unwrap_err()).contains("valid email"));
        let long = "x".repeat(MAX_PASSWORD_BYTES + 1);
        assert!(msg(configured_owner(Some("me@example.com"), Some(&long)).unwrap_err()).contains("longer"));
    }

    #[test]
    fn errors_never_contain_the_password() {
        for (e, p) in [(Some("bad"), Some("hunter2-secret")), (Some("me@example.com"), Some(&*"s".repeat(2000)))] {
            let err = configured_owner(e, p).unwrap_err().to_string();
            assert!(!err.contains("hunter2") && !err.contains("sss"), "{err}");
        }
    }
}
