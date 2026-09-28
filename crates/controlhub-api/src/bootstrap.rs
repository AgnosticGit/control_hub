//! Local development bootstrap: migrations, least-privilege service
//! credentials, seed tenant/project/environment, users and ingestion tokens.
//! Writes generated secrets to the gitignored `.controlhub/` directory only.

use std::path::Path;

use anyhow::Context;
use controlhub_domain::{EnvironmentId, ProjectId, Scope, TenantId};
use controlhub_runtime::env_required;
use controlhub_storage::ch::{ChConfig, ChStore};
use controlhub_storage::pg::PgStore;
use controlhub_storage::secrets::{generate_password, generate_token, hash_password, hash_token, token_prefix};
use serde_json::json;

const CH_MIGRATIONS: &[(&str, &str)] =
    &[("0001_telemetry.sql", include_str!("../../../infra/migrations/clickhouse/0001_telemetry.sql"))];

struct SeedScope {
    scope: Scope,
    tenant_name: &'static str,
    project_name: &'static str,
    env_name: &'static str,
    is_demo: bool,
}

fn scope(t: &str, p: &str, e: &str) -> anyhow::Result<Scope> {
    Ok(Scope {
        tenant_id: TenantId::parse(t)?,
        project_id: ProjectId::parse(p)?,
        environment_id: EnvironmentId::parse(e)?,
    })
}

pub async fn run(out_dir: &Path) -> anyhow::Result<()> {
    let pg_port = std::env::var("CH_PG_PORT").unwrap_or_else(|_| "15432".into());
    let pg_admin_url = format!(
        "postgres://{}:{}@127.0.0.1:{pg_port}/controlhub",
        env_required("CH_PG_ADMIN_USER")?,
        env_required("CH_PG_ADMIN_PASSWORD")?
    );
    let pg = PgStore::connect(&pg_admin_url, 2).await.context("connect postgres as admin")?;
    pg.migrate().await.context("postgres migrations")?;
    tracing::info!("postgres migrations applied");

    pg.ensure_service_role(
        "controlhub_api",
        &env_required("CH_PG_API_PASSWORD")?,
        &[
            "SELECT ON tenants, projects, environments, users, memberships, consumer_offsets, processing_errors",
            "SELECT, INSERT, UPDATE ON sessions",
            "SELECT, INSERT ON audit_log",
        ],
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
    ch.ensure_user("controlhub_api", &env_required("CH_CH_API_PASSWORD")?, &["SELECT ON controlhub.*"]).await?;
    ch.ensure_user(
        "controlhub_processor",
        &env_required("CH_CH_PROCESSOR_PASSWORD")?,
        &["INSERT, SELECT ON controlhub.*"],
    )
    .await?;

    // Seed scopes: a demo project and a second tenant used by isolation tests.
    let seeds = [
        SeedScope {
            scope: scope("local", "demo", "local-demo")?,
            tenant_name: "Local",
            project_name: "Demo system",
            env_name: "local-demo",
            is_demo: true,
        },
        SeedScope {
            scope: scope("isolation-tenant", "isolation", "isolation-test")?,
            tenant_name: "Isolation test tenant",
            project_name: "Isolation",
            env_name: "isolation-test",
            is_demo: true,
        },
    ];
    for s in &seeds {
        pg.upsert_scope(&s.scope, s.tenant_name, s.project_name, s.env_name, s.is_demo).await?;
    }

    let mut users = Vec::new();
    for (email, name, project, role) in [
        ("admin@controlhub.local", "Local admin", "demo", "owner"),
        ("viewer@controlhub.local", "Local viewer", "demo", "viewer"),
        ("isolation@controlhub.local", "Isolation tenant user", "isolation", "owner"),
    ] {
        let password = generate_password()?;
        let pw = password.clone();
        let hash = tokio::task::spawn_blocking(move || hash_password(&pw)).await??;
        let id = pg.upsert_user(email, name, &hash).await?;
        pg.upsert_membership(id, project, role).await?;
        users.push(json!({ "email": email, "password": password, "project_id": project, "role": role }));
    }

    let mut tokens = serde_json::Map::new();
    for (name, s) in [("demo-local", &seeds[0].scope), ("isolation-test", &seeds[1].scope)] {
        pg.revoke_ingest_tokens_named(s, name).await?;
        let token = generate_token("chi_")?;
        pg.create_ingest_token(s, name, &hash_token(&token), &token_prefix(&token), None, None).await?;
        tokens.insert(name.to_owned(), json!(token));
    }

    std::fs::create_dir_all(out_dir)?;
    let creds = json!({
        "warning": "Local development credentials. Generated by `just bootstrap`; rotated on every run. Never commit.",
        "web_url": "http://localhost:3000/app",
        "api_url": "http://127.0.0.1:8080",
        "otlp_http_endpoint": "http://127.0.0.1:4318",
        "project_id": "demo",
        "environment_id": "local-demo",
        "users": users,
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
