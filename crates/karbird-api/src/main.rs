//! karbird-api: sessions, scoped telemetry queries, timeline and audit.
//! Has read-only ClickHouse credentials and no infrastructure-control access.

mod auth;
mod bootstrap;
mod error;
mod live_consumer;
mod live_hub;
mod live_metrics;
mod mail;
mod openapi;
mod permissions;
mod routes;
mod state;
mod system_permissions;
mod test_fixtures;
mod topology;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use axum::Router;
use axum::http::{HeaderValue, StatusCode, header};
use axum::routing::{get, post};
use clap::{Parser, Subcommand};
use karbird_domain::{EnvironmentId, ProjectId, Scope, TenantId};
use karbird_protocol::TELEMETRY_TOPIC;
use karbird_runtime::connect_broker;
use karbird_runtime::{init_tracing, load_local_env, shutdown_signal};
use karbird_storage::ch::{ChConfig, ChStore};
use karbird_storage::pg::PgStore;
use karbird_storage::retention::{self, Table};
use rdkafka::consumer::{Consumer, StreamConsumer};
use sqlx::postgres::{PgListener, PgNotification};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

#[derive(Debug, serde::Deserialize)]
struct ActionPlanNotice {
    tenant_id: String,
    project_id: String,
    environment_id: String,
    plan_id: String,
}

async fn forward_action_plan_notice(state: &state::AppState, notice: &PgNotification) {
    let parsed = serde_json::from_str::<ActionPlanNotice>(notice.payload());
    let Ok(notice) = parsed else {
        tracing::warn!("discarding malformed action-plan notification");
        return;
    };
    let scope = Scope {
        tenant_id: match TenantId::parse(notice.tenant_id) {
            Ok(id) => id,
            Err(_) => return,
        },
        project_id: match ProjectId::parse(notice.project_id) {
            Ok(id) => id,
            Err(_) => return,
        },
        environment_id: match EnvironmentId::parse(notice.environment_id) {
            Ok(id) => id,
            Err(_) => return,
        },
    };
    let id = match uuid::Uuid::parse_str(&notice.plan_id) {
        Ok(id) => id,
        Err(_) => return,
    };
    match state.pg.action_plan(&scope, id).await {
        Ok(Some(row)) => {
            let plan: routes::actions::ActionPlanDto = row.into();
            match serde_json::to_value(plan) {
                Ok(plan) => {
                    if let Err(error) = state.live_hub.publish_action(scope, plan) {
                        tracing::debug!(%error, "action-plan WebSocket fan-out skipped");
                    }
                }
                Err(error) => tracing::warn!(%error, "could not encode action-plan notification"),
            }
        }
        Ok(None) => {}
        Err(error) => tracing::warn!(%error, "could not load changed action plan"),
    }
}

async fn run_action_notifications(mut listener: PgListener, state: state::AppState) {
    loop {
        match listener.recv().await {
            Ok(notification) => forward_action_plan_notice(&state, &notification).await,
            Err(error) => {
                tracing::warn!(%error, "PostgreSQL action notification listener failed");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

#[derive(Debug, Parser)]
#[command(name = "karbird-api")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the HTTP API (default).
    Serve(Box<ServeArgs>),
    /// Local development bootstrap (migrations, retention TTLs, service users, seed data).
    Bootstrap {
        #[arg(long, default_value = ".karbird")]
        out_dir: PathBuf,
        #[command(flatten)]
        retention: RetentionArgs,
        /// Confirms a retention decrease that permanently deletes older telemetry.
        #[arg(long)]
        shrink_retention: bool,
    },
    /// Production deploys: migrations, retention TTLs, service users and the owner account.
    /// No demo data, no token rotation, no files; safe to run on every deploy.
    Migrate {
        #[command(flatten)]
        retention: RetentionArgs,
        /// Confirms a retention decrease that permanently deletes older telemetry.
        #[arg(long)]
        shrink_retention: bool,
    },
    /// Create (or with --remove delete) the accounts and scopes the integration/e2e tests use.
    TestFixtures {
        #[arg(long, default_value = ".karbird")]
        out_dir: PathBuf,
        #[arg(long)]
        remove: bool,
    },
    /// Print the OpenAPI document.
    Openapi,
}

/// How many days ClickHouse keeps each signal; `bootstrap` applies these as table TTLs.
#[derive(Debug, Clone, clap::Args)]
struct RetentionArgs {
    #[arg(long, env = "KARBIRD_LOGS_RETENTION_DAYS", default_value_t = retention::DEFAULT_LOGS_DAYS, value_parser = retention_days())]
    logs_retention_days: u32,
    #[arg(long, env = "KARBIRD_SPANS_RETENTION_DAYS", default_value_t = retention::DEFAULT_SPANS_DAYS, value_parser = retention_days())]
    spans_retention_days: u32,
    #[arg(long, env = "KARBIRD_EVENTS_RETENTION_DAYS", default_value_t = retention::DEFAULT_EVENTS_DAYS, value_parser = retention_days())]
    events_retention_days: u32,
    #[arg(long, env = "KARBIRD_METRICS_RETENTION_DAYS", default_value_t = retention::DEFAULT_METRICS_DAYS, value_parser = retention_days())]
    metrics_retention_days: u32,
}

fn retention_days() -> clap::builder::RangedI64ValueParser<u32> {
    clap::value_parser!(u32).range(i64::from(retention::MIN_DAYS)..=i64::from(retention::MAX_DAYS))
}

impl RetentionArgs {
    fn tables(&self) -> [(Table, u32); 5] {
        [
            (Table::Logs, self.logs_retention_days),
            (Table::Spans, self.spans_retention_days),
            (Table::EdgeSpanFacts, self.spans_retention_days),
            (Table::Events, self.events_retention_days),
            (Table::Metrics, self.metrics_retention_days),
        ]
    }
}

#[derive(Debug, Parser)]
struct ServeArgs {
    #[arg(long, env = "KARBIRD_API_ADDR", default_value = "127.0.0.1:8080")]
    addr: SocketAddr,
    #[arg(long, env = "KARBIRD_API_DATABASE_URL")]
    database_url: String,
    /// Unique and stable per API replica; each replica group receives every telemetry partition.
    #[arg(long, env = "KARBIRD_LIVE_GROUP_ID")]
    live_group_id: Option<String>,
    #[arg(long, env = "KARBIRD_KAFKA_BROKERS", default_value = "127.0.0.1:19092")]
    brokers: String,
    #[arg(long, env = "KARBIRD_CLICKHOUSE_URL", default_value = "http://127.0.0.1:18123")]
    clickhouse_url: String,
    #[arg(long, env = "KARBIRD_CLICKHOUSE_DATABASE", default_value = "karbird")]
    clickhouse_database: String,
    #[arg(long, env = "KARBIRD_API_CLICKHOUSE_USER")]
    clickhouse_user: String,
    #[arg(long, env = "KARBIRD_API_CLICKHOUSE_PASSWORD")]
    clickhouse_password: String,
    /// Comma-separated origins allowed for cookie-authenticated mutations.
    #[arg(long, env = "KARBIRD_ALLOWED_ORIGINS", default_value = "http://localhost:3000,http://127.0.0.1:3000")]
    allowed_origins: String,
    /// `dev` disables the Secure cookie flag for plain-http localhost. Anything else enforces it.
    #[arg(long, env = "KARBIRD_ENV", default_value = "production")]
    env: String,
    /// Maximum concurrent live WebSocket connections; further upgrades get 429.
    #[arg(long, env = "KARBIRD_MAX_LIVE_CONNECTIONS", default_value_t = 256)]
    max_live_connections: usize,
    /// Self-service sign-up at `POST /api/v1/session/register`: `open` or `closed`.
    #[arg(long, env = "KARBIRD_SIGNUP", default_value = "closed", value_parser = ["open", "closed"])]
    signup: String,
    /// Sign-up attempts allowed per hour across all clients.
    #[arg(long, env = "KARBIRD_SIGNUP_PER_HOUR", default_value_t = 20)]
    signup_per_hour: u32,
    #[arg(long, env = "KARBIRD_PUBLIC_URL", default_value = "http://localhost:3000")]
    public_url: String,
    #[arg(long, env = "KARBIRD_MAIL_ENABLED", default_value_t = false)]
    mail_enabled: bool,
    #[arg(long, env = "KARBIRD_RESEND_API_KEY", requires = "mail_from")]
    resend_api_key: Option<String>,
    #[arg(long, env = "KARBIRD_MAIL_FROM", requires = "resend_api_key")]
    mail_from: Option<String>,
    #[arg(long, env = "KARBIRD_GOOGLE_CLIENT_ID", requires_all = ["google_client_secret", "google_redirect_url"])]
    google_client_id: Option<String>,
    #[arg(long, env = "KARBIRD_GOOGLE_CLIENT_SECRET", requires_all = ["google_client_id", "google_redirect_url"])]
    google_client_secret: Option<String>,
    #[arg(long, env = "KARBIRD_GOOGLE_REDIRECT_URL", requires_all = ["google_client_id", "google_client_secret"])]
    google_redirect_url: Option<String>,
    /// S3-compatible endpoint of the log archive. Without it logs are served from ClickHouse only.
    #[arg(long, env = "KARBIRD_ARCHIVE_S3_ENDPOINT")]
    archive_s3_endpoint: Option<String>,
    #[arg(long, env = "KARBIRD_ARCHIVE_S3_BUCKET", default_value = "karbird-archive")]
    archive_s3_bucket: String,
    #[arg(long, env = "KARBIRD_ARCHIVE_S3_REGION", default_value = "us-east-1")]
    archive_s3_region: String,
    #[arg(long, env = "KARBIRD_API_S3_ACCESS_KEY", requires = "archive_s3_endpoint")]
    archive_s3_access_key: Option<String>,
    #[arg(long, env = "KARBIRD_API_S3_SECRET_KEY", requires = "archive_s3_endpoint")]
    archive_s3_secret_key: Option<String>,
    /// Logs older than this are read from the archive by default. Default: one day less than
    /// the log retention; must stay below it.
    #[arg(long, env = "KARBIRD_ARCHIVE_HOT_WINDOW_HOURS", value_parser = clap::value_parser!(u32).range(1..=24 * i64::from(retention::MAX_DAYS)))]
    archive_hot_window_hours: Option<u32>,
    #[command(flatten)]
    retention: RetentionArgs,
}

/// Hours of logs read from ClickHouse by default: the configured value, or one day less than
/// the log retention, so a query never asks ClickHouse for rows its TTL may already have deleted.
fn hot_window_hours(args: &ServeArgs) -> anyhow::Result<u32> {
    let retention_hours = args.retention.logs_retention_days * 24;
    match args.archive_hot_window_hours {
        Some(h) if h >= retention_hours => anyhow::bail!(
            "KARBIRD_ARCHIVE_HOT_WINDOW_HOURS={h} must be below the log retention \
             (KARBIRD_LOGS_RETENTION_DAYS={} = {retention_hours} h): ClickHouse no longer has older logs",
            args.retention.logs_retention_days
        ),
        Some(h) => Ok(h),
        None => Ok(retention_hours - 24),
    }
}

/// Refuses to serve when the configured retention is not what ClickHouse does. Waits for
/// ClickHouse to come up, but a mismatch fails at once.
async fn verify_retention(ch: &ChStore, retention: &RetentionArgs) -> anyhow::Result<()> {
    for (table, days) in retention.tables() {
        let mut failures = 0u32;
        let actual = loop {
            match ch.ttl_days(table).await {
                Ok(actual) => break actual,
                Err(e) if failures < 20 => {
                    failures += 1;
                    tracing::warn!(error = %e, failures, "ClickHouse not ready for the retention check; retrying");
                    tokio::time::sleep(karbird_runtime::backoff(
                        failures,
                        Duration::from_millis(250),
                        Duration::from_secs(5),
                    ))
                    .await;
                }
                Err(e) => return Err(anyhow::Error::new(e).context("read ClickHouse retention")),
            }
        };
        retention::check(table, days, actual).map_err(anyhow::Error::msg)?;
    }
    Ok(())
}

fn archive_config(args: &ServeArgs) -> anyhow::Result<Option<state::ArchiveConfig>> {
    let hot_window_hours = hot_window_hours(args)?;
    let Some(endpoint) = &args.archive_s3_endpoint else { return Ok(None) };
    let (Some(access_key), Some(secret_key)) = (&args.archive_s3_access_key, &args.archive_s3_secret_key) else {
        anyhow::bail!("KARBIRD_ARCHIVE_S3_ENDPOINT needs KARBIRD_API_S3_ACCESS_KEY and KARBIRD_API_S3_SECRET_KEY");
    };
    let objects = karbird_storage::archive::ArchiveObjects::s3(&karbird_storage::archive::ObjectConfig {
        endpoint: endpoint.clone(),
        bucket: args.archive_s3_bucket.clone(),
        region: args.archive_s3_region.clone(),
        access_key: access_key.clone(),
        secret_key: secret_key.clone(),
    })?;
    Ok(Some(state::ArchiveConfig {
        reader: karbird_storage::archive::ArchiveReader::new(objects),
        hot_window_ns: i64::from(hot_window_hours) * 3_600_000_000_000,
    }))
}

fn router(state: state::AppState) -> Router {
    use axum::routing::{delete, patch};
    use routes::{account, admin, invites, organisations, roles, session, telemetry};
    let scoped = "/api/v1/projects/{project_id}/environments/{environment_id}";
    Router::new()
        .route("/api/v1/health", get(routes::health))
        .route("/api/v1/ready", get(routes::ready))
        // Scraped over the private API network; the public Caddy site does not proxy this path.
        .route("/metrics", get(routes::metrics::metrics))
        .route("/api/v1/session/login", post(session::login))
        .route("/api/v1/session/register", post(routes::signup::register))
        .route("/api/v1/session/logout", post(session::logout))
        .route("/api/v1/session", get(session::session))
        .route("/api/v1/auth/google/start", get(routes::oauth::start))
        .route("/api/v1/auth/google/callback", get(routes::oauth::callback))
        .route("/api/v1/onboarding/first-project", post(account::create_first_project))
        .route("/api/v1/projects", get(session::projects).post(account::create_project))
        .route("/api/v1/projects/{project_id}", axum::routing::patch(account::rename_project))
        .route("/api/v1/projects/{project_id}/leave", post(account::leave_project))
        .route("/api/v1/account", axum::routing::patch(account::update))
        .route("/api/v1/account/password", post(account::change_password))
        .route("/api/v1/account/sessions", get(account::sessions))
        .route("/api/v1/account/sessions/revoke-others", post(account::revoke_other_sessions))
        .route("/api/v1/account/sessions/{session_id}", axum::routing::delete(account::revoke_session))
        .route("/api/v1/projects/{project_id}/invites", get(invites::list).post(invites::create))
        .route("/api/v1/projects/{project_id}/invites/{invite_id}", axum::routing::delete(invites::revoke))
        .route("/api/v1/invites/preview", post(invites::preview))
        .route("/api/v1/invites/accept", post(invites::accept))
        .route(
            "/api/v1/organizations/{organization_id}/members",
            get(organisations::list_members).post(organisations::add_member),
        )
        .route(
            "/api/v1/organizations/{organization_id}/projects/{project_id}/members/{user_id}",
            axum::routing::put(organisations::put_project_assignment).delete(organisations::delete_project_assignment),
        )
        .route(
            "/api/v1/organizations/{organization_id}/invites",
            get(organisations::list_invites).post(organisations::create_invite),
        )
        .route(
            "/api/v1/organizations/{organization_id}/invites/{invite_id}",
            axum::routing::delete(organisations::revoke_invite),
        )
        .route("/api/v1/organization-invites/accept", post(organisations::accept_invite))
        .route("/api/v1/permissions", get(roles::catalog))
        .route("/api/v1/projects/{project_id}/roles", get(roles::list_roles).post(roles::create_role))
        .route(
            "/api/v1/projects/{project_id}/roles/{role_id}",
            axum::routing::patch(roles::update_role).delete(roles::delete_role),
        )
        .route("/api/v1/projects/{project_id}/members", get(roles::list_members))
        .route(
            "/api/v1/projects/{project_id}/members/{user_id}",
            axum::routing::patch(roles::update_member).delete(roles::remove_member),
        )
        .route(&format!("{scoped}/services"), get(telemetry::services))
        .route(&format!("{scoped}/topology"), get(telemetry::topology))
        .route(&format!("{scoped}/traces"), get(telemetry::traces))
        .route(&format!("{scoped}/traces/{{trace_id}}"), get(telemetry::trace_detail))
        .route(&format!("{scoped}/logs"), get(telemetry::logs))
        .route(&format!("{scoped}/events"), get(telemetry::events))
        .route(&format!("{scoped}/timeline"), get(telemetry::timeline))
        .route(&format!("{scoped}/coverage"), get(telemetry::coverage))
        .route(&format!("{scoped}/edges/traffic"), get(routes::edge_traffic::edge_traffic))
        .route(&format!("{scoped}/infrastructure"), get(routes::infrastructure::infrastructure))
        .route(&format!("{scoped}/infrastructure/timeline"), get(routes::infra_timeline::infra_timeline))
        .route(&format!("{scoped}/live"), get(routes::live::live))
        .route(&format!("{scoped}/audit"), get(routes::audit))
        .route(&format!("{scoped}/tokens"), get(routes::tokens::list).post(routes::tokens::create))
        .route(&format!("{scoped}/tokens/{{token_id}}/revoke"), post(routes::tokens::revoke))
        .route(&format!("{scoped}/tokens/{{token_id}}/rotate"), post(routes::tokens::rotate))
        .route(&format!("{scoped}/actions"), get(routes::actions::list).post(routes::actions::propose))
        .route(&format!("{scoped}/actions/catalog"), get(routes::actions::catalog))
        .route(&format!("{scoped}/actions/{{plan_id}}"), get(routes::actions::detail))
        .route(&format!("{scoped}/actions/{{plan_id}}/approve"), post(routes::actions::approve))
        .route(&format!("{scoped}/actions/{{plan_id}}/reject"), post(routes::actions::reject))
        .route(&format!("{scoped}/actions/{{plan_id}}/cancel"), post(routes::actions::cancel))
        .route("/api/v1/admin/overview", get(admin::overview))
        .route("/api/v1/admin/permissions", get(admin::catalog))
        .route("/api/v1/admin/projects", get(admin::list_projects).post(admin::create_project))
        .route("/api/v1/admin/projects/{project_id}", patch(admin::rename_project).delete(admin::delete_project))
        .route("/api/v1/admin/projects/{project_id}/environments", post(admin::create_environment))
        .route(
            "/api/v1/admin/projects/{project_id}/members",
            get(admin::list_project_members).put(admin::put_project_member),
        )
        .route("/api/v1/admin/projects/{project_id}/members/{user_id}", delete(admin::remove_project_member))
        .route("/api/v1/admin/users", get(admin::list_users))
        .route("/api/v1/admin/users/{user_id}/disable", post(admin::disable_user))
        .route("/api/v1/admin/users/{user_id}/enable", post(admin::enable_user))
        .route("/api/v1/admin/users/{user_id}/sessions/revoke", post(admin::revoke_user_sessions))
        .route("/api/v1/admin/members", get(admin::list_members).post(admin::add_member))
        .route("/api/v1/admin/members/{user_id}", patch(admin::update_member).delete(admin::remove_member))
        .route("/api/v1/admin/roles", get(admin::list_roles).post(admin::create_role))
        .route("/api/v1/admin/roles/{role_id}", patch(admin::update_role).delete(admin::delete_role))
        .route("/api/v1/admin/audit", get(admin::audit_log))
        .layer(RequestBodyLimitLayer::new(1024 * 1024))
        .layer(TimeoutLayer::with_status_code(StatusCode::SERVICE_UNAVAILABLE, Duration::from_secs(30)))
        .layer(SetResponseHeaderLayer::overriding(header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
        .layer(SetResponseHeaderLayer::overriding(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

fn api_config(args: &ServeArgs) -> state::ApiConfig {
    state::ApiConfig {
        allowed_origins: args.allowed_origins.split(',').map(|s| s.trim().to_owned()).collect(),
        secure_cookies: args.env != "dev",
        session_ttl: time::Duration::hours(12),
        signup: state::SignupPolicy { open: args.signup == "open", per_hour: args.signup_per_hour },
    }
}

fn advance_live_cursor(previous: &mut i64, now: i64) -> i64 {
    *previous = now.max(previous.saturating_add(1));
    *previous
}

fn resolve_live_group_id(configured: Option<&str>, hostname: Option<&str>) -> anyhow::Result<String> {
    if let Some(group) = configured {
        anyhow::ensure!(!group.trim().is_empty(), "KARBIRD_LIVE_GROUP_ID must not be empty");
        return Ok(group.to_owned());
    }
    Ok(hostname
        .filter(|host| !host.trim().is_empty())
        .map(|host| format!("karbird-live-{host}"))
        .unwrap_or_else(|| "karbird-live-local".to_owned()))
}

async fn serve(args: ServeArgs) -> anyhow::Result<()> {
    let config = api_config(&args);
    let archive = archive_config(&args)?;
    if archive.is_none() {
        tracing::info!("log archive not configured: logs are served from ClickHouse only");
    }
    let pg = PgStore::connect(&args.database_url, 16).await?;
    let ch = ChStore::new(&ChConfig {
        url: args.clickhouse_url,
        database: args.clickhouse_database,
        user: args.clickhouse_user,
        password: args.clickhouse_password,
    });
    verify_retention(&ch, &args.retention).await?;
    let kafka_config = connect_broker(&args.brokers, Duration::from_secs(60)).await?;
    let hostname = std::env::var("HOSTNAME").ok();
    let live_group_id = resolve_live_group_id(args.live_group_id.as_deref(), hostname.as_deref())?;
    if !config.secure_cookies {
        tracing::warn!("KARBIRD_ENV=dev: Secure cookie flag disabled for local http. Never use in production.");
    }
    let resend_api_key = args.resend_api_key.as_deref().filter(|value| !value.trim().is_empty());
    let mail_from = args.mail_from.as_deref().filter(|value| !value.trim().is_empty());
    let mailer: std::sync::Arc<dyn mail::Mailer> = match (resend_api_key, mail_from) {
        (Some(key), Some(from)) => std::sync::Arc::new(mail::ResendMailer::new(key.to_owned(), from.to_owned())?),
        (None, None) if !args.mail_enabled => std::sync::Arc::new(mail::DisabledMailer),
        _ => anyhow::bail!("KARBIRD_MAIL_ENABLED needs KARBIRD_RESEND_API_KEY and KARBIRD_MAIL_FROM"),
    };
    let google_client_id = args.google_client_id.as_deref().filter(|value| !value.trim().is_empty());
    let google_client_secret = args.google_client_secret.as_deref().filter(|value| !value.trim().is_empty());
    let google_redirect_url = args.google_redirect_url.as_deref().filter(|value| !value.trim().is_empty());
    let google_oauth = match (google_client_id, google_client_secret, google_redirect_url) {
        (Some(client_id), Some(client_secret), Some(redirect_url)) => Some(routes::oauth::GoogleOAuthConfig {
            client_id: client_id.to_owned(),
            client_secret: client_secret.to_owned(),
            redirect_url: redirect_url.to_owned(),
        }),
        (None, None, None) => None,
        _ => anyhow::bail!(
            "Google OAuth needs KARBIRD_GOOGLE_CLIENT_ID, KARBIRD_GOOGLE_CLIENT_SECRET and KARBIRD_GOOGLE_REDIRECT_URL"
        ),
    };
    let state = state::AppState {
        pg,
        ch,
        archive,
        config,
        login_throttle: auth::LoginThrottle::default(),
        signup_throttle: routes::signup::SignupThrottle::default(),
        live_slots: std::sync::Arc::new(tokio::sync::Semaphore::new(args.max_live_connections)),
        live_hub: live_hub::LiveHub::default(),
        live_metrics: std::sync::Arc::new(live_metrics::LiveMetrics::default()),
        mailer,
        public_url: args.public_url.trim_end_matches('/').to_owned(),
        google_oauth,
        oauth_states: routes::oauth::OAuthStateStore::default(),
        oauth_client: reqwest::Client::builder().timeout(Duration::from_secs(10)).build()?,
    };
    let mut action_listener = PgListener::connect_with(state.pg.pool()).await?;
    action_listener.listen("karbird_action_plan_changed").await?;
    let action_task = tokio::spawn(run_action_notifications(action_listener, state.clone()));
    let live_consumer: StreamConsumer = kafka_config
        .clone()
        .set("group.id", &live_group_id)
        .set("enable.auto.commit", "false")
        .set("enable.auto.offset.store", "false")
        .set("auto.offset.reset", "latest")
        .set("isolation.level", "read_committed")
        .create()?;
    live_consumer.subscribe(&[TELEMETRY_TOPIC])?;
    let live_task = tokio::spawn(live_consumer::run(live_consumer, state.live_hub.clone(), state.live_metrics.clone()));
    let listener = tokio::net::TcpListener::bind(args.addr).await?;
    tracing::info!(addr = %args.addr, "karbird-api listening");
    axum::serve(listener, router(state)).with_graceful_shutdown(shutdown_signal()).await?;
    live_task.abort();
    action_task.abort();
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    load_local_env();
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Openapi) => {
            println!("{}", openapi::spec_json()?);
            Ok(())
        }
        Some(Command::Bootstrap { out_dir, retention, shrink_retention }) => {
            init_tracing("karbird-api");
            bootstrap::run(&out_dir, &retention.tables(), shrink_retention).await
        }
        Some(Command::Migrate { retention, shrink_retention }) => {
            init_tracing("karbird-api");
            bootstrap::migrate(&retention.tables(), shrink_retention).await
        }
        Some(Command::TestFixtures { out_dir, remove }) => {
            init_tracing("karbird-api");
            if remove { test_fixtures::remove(&out_dir).await } else { test_fixtures::create(&out_dir).await }
        }
        Some(Command::Serve(args)) => {
            init_tracing("karbird-api");
            serve(*args).await
        }
        None => {
            init_tracing("karbird-api");
            serve(ServeArgs::parse_from(["karbird-api"])).await
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    #[test]
    fn dummy_hash_is_valid_phc() {
        assert!(argon2_parse(crate::routes::session::DUMMY_HASH_FOR_TEST));
    }

    fn argon2_parse(s: &str) -> bool {
        !karbird_storage::secrets::verify_password("x", s) && s.starts_with("$argon2id$")
    }

    #[test]
    fn openapi_spec_builds() {
        let spec = crate::openapi::spec_json().unwrap();
        assert!(spec.contains("/api/v1/projects/{project_id}/environments/{environment_id}/timeline"));
    }

    #[test]
    fn timeline_contract_is_engine_compatible() {
        // The Worker hands the REST response text straight to the WASM engine.
        let json = r#"{"items":[{"type":"span","trace_id":"t","span_id":"s","parent_span_id":null,"service_id":"svc_a","service_name":"a","target_entity_id":null,"name":"GET /","kind":2,"start_ns":"1790000000000000000","end_ns":"1790000000100000000","is_error":false}],"from_ns":"1","to_ns":"2","spans_truncated":false,"logs_truncated":false,"events_truncated":false,"completeness":{}}"#;
        let mut e = karbird_engine_core::Engine::new(Default::default());
        e.apply_batch_json(json).unwrap();
        assert_eq!(e.item_count(), 1);
    }

    use axum::body::Body;
    use axum::http::{HeaderMap, Method, Request, StatusCode};
    use clap::Parser as _;
    use serde_json::Value;
    use tower::ServiceExt as _;

    const SCOPED: &str = "/api/v1/projects/demo/environments/local-demo";
    const PLAN: &str = "00000000-0000-0000-0000-000000000000";

    fn get_routes() -> Vec<String> {
        [
            "/services",
            "/topology",
            "/traces",
            "/traces/0123456789abcdef0123456789abcdef",
            "/logs",
            "/events",
            "/timeline",
            "/coverage",
            "/edges/traffic",
            "/infrastructure",
            "/infrastructure/timeline",
            "/live",
            "/audit",
            "/tokens",
            "/actions",
            "/actions/catalog",
        ]
        .iter()
        .map(|p| format!("{SCOPED}{p}"))
        .chain(
            ["/api/v1/session", "/api/v1/projects", "/api/v1/permissions", PROJECT_ROLES, PROJECT_MEMBERS]
                .map(str::to_owned),
        )
        .collect()
    }

    const PROJECT_ROLES: &str = "/api/v1/projects/demo/roles";
    const PROJECT_MEMBERS: &str = "/api/v1/projects/demo/members";

    /// Role and member mutations (method, uri).
    fn member_mutations() -> Vec<(Method, String)> {
        vec![
            (Method::POST, PROJECT_ROLES.to_owned()),
            (Method::PATCH, format!("{PROJECT_ROLES}/{PLAN}")),
            (Method::DELETE, format!("{PROJECT_ROLES}/{PLAN}")),
            (Method::PATCH, format!("{PROJECT_MEMBERS}/{PLAN}")),
            (Method::DELETE, format!("{PROJECT_MEMBERS}/{PLAN}")),
        ]
    }

    #[tokio::test]
    async fn member_and_role_mutations_need_a_session_and_never_accept_api_tokens() {
        let body = r#"{"name":"Manager","permissions":["telemetry.view"],"role_id":null}"#;
        for (m, uri) in member_mutations() {
            let (s, _, _) = send(state(), req(m.clone(), &uri, &[], body)).await;
            assert_eq!(s, StatusCode::UNAUTHORIZED, "{m} {uri}");
            let (s, _, b) = send(state(), req(m.clone(), &uri, &[("authorization", "Bearer cha_x")], body)).await;
            assert_eq!(s, StatusCode::FORBIDDEN, "{m} {uri}");
            assert_eq!(b["message"], "forbidden: API tokens are read-only", "{m} {uri}");
            // With a session cookie the session lookup hits storage (down here): fail closed.
            let (s, _, _) = send(state(), req(m.clone(), &uri, &[("cookie", "karbird_session=chs_x")], body)).await;
            assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{m} {uri}");
        }
    }

    /// Account, project and invite mutations (method, uri).
    fn account_mutations() -> Vec<(Method, String)> {
        vec![
            (Method::PATCH, "/api/v1/account".to_owned()),
            (Method::POST, "/api/v1/account/password".to_owned()),
            (Method::POST, "/api/v1/account/sessions/revoke-others".to_owned()),
            (Method::DELETE, format!("/api/v1/account/sessions/{PLAN}")),
            (Method::POST, "/api/v1/onboarding/first-project".to_owned()),
            (Method::POST, "/api/v1/projects".to_owned()),
            (Method::PATCH, "/api/v1/projects/demo".to_owned()),
            (Method::POST, "/api/v1/projects/demo/leave".to_owned()),
            (Method::POST, "/api/v1/projects/demo/invites".to_owned()),
            (Method::DELETE, format!("/api/v1/projects/demo/invites/{PLAN}")),
            (Method::POST, "/api/v1/invites/accept".to_owned()),
        ]
    }

    #[tokio::test]
    async fn account_project_and_invite_mutations_need_a_session_and_never_accept_api_tokens() {
        let body = r#"{"name":"P","display_name":"D","current_password":"x","new_password":"y","role_id":"r","token":"chv_x"}"#;
        for (m, uri) in account_mutations() {
            let (s, _, _) = send(state(), req(m.clone(), &uri, &[], body)).await;
            assert_eq!(s, StatusCode::UNAUTHORIZED, "{m} {uri}");
            let (s, _, b) = send(state(), req(m.clone(), &uri, &[("authorization", "Bearer cha_x")], body)).await;
            assert_eq!(s, StatusCode::FORBIDDEN, "{m} {uri}");
            assert_eq!(b["message"], "forbidden: API tokens are read-only", "{m} {uri}");
            let (s, _, _) = send(state(), req(m.clone(), &uri, &[("cookie", "karbird_session=chs_x")], body)).await;
            assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{m} {uri}: session lookup fails closed");
        }
        for uri in ["/api/v1/account/sessions", "/api/v1/projects/demo/invites"] {
            let (s, _, _) = send(state(), req(Method::GET, uri, &[], "")).await;
            assert_eq!(s, StatusCode::UNAUTHORIZED, "GET {uri}");
        }
    }

    /// karbird administration mutations (method, uri).
    fn admin_mutations() -> Vec<(Method, String)> {
        vec![
            (Method::POST, "/api/v1/admin/projects".to_owned()),
            (Method::PATCH, "/api/v1/admin/projects/p-1".to_owned()),
            (Method::DELETE, "/api/v1/admin/projects/p-1".to_owned()),
            (Method::POST, "/api/v1/admin/projects/p-1/environments".to_owned()),
            (Method::PUT, "/api/v1/admin/projects/p-1/members".to_owned()),
            (Method::DELETE, format!("/api/v1/admin/projects/p-1/members/{PLAN}")),
            (Method::POST, format!("/api/v1/admin/users/{PLAN}/disable")),
            (Method::POST, format!("/api/v1/admin/users/{PLAN}/enable")),
            (Method::POST, format!("/api/v1/admin/users/{PLAN}/sessions/revoke")),
            (Method::POST, "/api/v1/admin/members".to_owned()),
            (Method::PATCH, format!("/api/v1/admin/members/{PLAN}")),
            (Method::DELETE, format!("/api/v1/admin/members/{PLAN}")),
            (Method::POST, "/api/v1/admin/roles".to_owned()),
            (Method::PATCH, format!("/api/v1/admin/roles/{PLAN}")),
            (Method::DELETE, format!("/api/v1/admin/roles/{PLAN}")),
        ]
    }

    #[tokio::test]
    async fn admin_routes_need_a_session_and_never_accept_api_tokens() {
        let body = r#"{"name":"P","email":"a@example.test","role":"viewer","role_id":"r","permissions":[]}"#;
        for (m, uri) in admin_mutations() {
            let (s, _, _) = send(state(), req(m.clone(), &uri, &[], body)).await;
            assert_eq!(s, StatusCode::UNAUTHORIZED, "{m} {uri}");
            let (s, _, b) = send(state(), req(m.clone(), &uri, &[("authorization", "Bearer cha_x")], body)).await;
            assert_eq!(s, StatusCode::FORBIDDEN, "{m} {uri}");
            assert_eq!(b["message"], "forbidden: API tokens are read-only", "{m} {uri}");
            let (s, _, _) = send(state(), req(m.clone(), &uri, &[("cookie", "karbird_session=chs_x")], body)).await;
            assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{m} {uri}: session lookup fails closed");
        }
        for uri in [
            "/api/v1/admin/overview",
            "/api/v1/admin/permissions",
            "/api/v1/admin/projects",
            "/api/v1/admin/projects/p-1/members",
            "/api/v1/admin/users",
            "/api/v1/admin/members",
            "/api/v1/admin/roles",
            "/api/v1/admin/audit",
        ] {
            let (s, _, b) = send(state(), req(Method::GET, uri, &[], "")).await;
            assert_eq!((s, b["code"].as_str()), (StatusCode::UNAUTHORIZED, Some("unauthenticated")), "GET {uri}");
        }
    }

    #[tokio::test]
    async fn invite_preview_checks_origin_and_the_secret_format_before_storage() {
        let preview = "/api/v1/invites/preview";
        let (s, _, b) =
            send(state(), req(Method::POST, preview, &[("origin", "http://evil.example")], r#"{"token":"chv_x"}"#))
                .await;
        assert_eq!((s, b["message"].as_str()), (StatusCode::FORBIDDEN, Some("forbidden: origin not allowed")));
        for token in ["", "cha_apitoken", "chs_session", "chv_<script>", "chv_a b"] {
            let body = serde_json::json!({ "token": token }).to_string();
            let (s, _, _) = send(state(), req(Method::POST, preview, &[], &body)).await;
            assert_eq!(s, StatusCode::NOT_FOUND, "{token:?} is never looked up");
        }
        let (s, _, _) = send(state(), req(Method::POST, preview, &[], r#"{"token":"chv_wellformed"}"#)).await;
        assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "well-formed secrets reach storage (down here)");
    }

    fn post_routes() -> Vec<String> {
        [
            "/tokens".to_owned(),
            format!("/tokens/{PLAN}/revoke"),
            format!("/tokens/{PLAN}/rotate"),
            "/actions".to_owned(),
            format!("/actions/{PLAN}/approve"),
            format!("/actions/{PLAN}/reject"),
            format!("/actions/{PLAN}/cancel"),
        ]
        .iter()
        .map(|p| format!("{SCOPED}{p}"))
        .chain(["/api/v1/session/logout".to_owned()])
        .collect()
    }

    async fn send(state: crate::state::AppState, req: Request<Body>) -> (StatusCode, HeaderMap, Value) {
        let res = crate::router(state).oneshot(req).await.unwrap();
        let (status, headers) = (res.status(), res.headers().clone());
        let body = axum::body::to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        (status, headers, serde_json::from_slice(&body).unwrap_or(Value::Null))
    }

    fn req(method: Method, uri: &str, headers: &[(&str, &str)], body: &str) -> Request<Body> {
        let mut b = Request::builder().method(method).uri(uri).header("content-type", "application/json");
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        b.body(Body::from(body.to_owned())).unwrap()
    }

    fn state() -> crate::state::AppState {
        crate::state::test_state(true)
    }

    #[tokio::test]
    async fn health_is_public_and_responses_carry_security_headers() {
        let (s, h, body) = send(state(), req(Method::GET, "/api/v1/health", &[], "")).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(body["status"], "ok");
        assert_eq!(h.get("cache-control").unwrap(), "no-store");
        assert_eq!(h.get("x-content-type-options").unwrap(), "nosniff");
    }

    #[tokio::test]
    async fn error_responses_carry_security_headers_too() {
        let (s, h, _) = send(state(), req(Method::GET, &format!("{SCOPED}/services"), &[], "")).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
        assert_eq!(h.get("cache-control").unwrap(), "no-store");
        assert_eq!(h.get("x-content-type-options").unwrap(), "nosniff");
    }

    #[tokio::test]
    async fn every_protected_route_requires_a_credential() {
        for uri in get_routes() {
            let (s, _, body) = send(state(), req(Method::GET, &uri, &[], "")).await;
            assert_eq!(s, StatusCode::UNAUTHORIZED, "GET {uri}");
            assert_eq!(body["code"], "unauthenticated", "GET {uri}");
        }
        for uri in post_routes() {
            let (s, _, _) = send(state(), req(Method::POST, &uri, &[], r#"{"comment":"x"}"#)).await;
            assert_eq!(s, StatusCode::UNAUTHORIZED, "POST {uri}");
        }
    }

    #[tokio::test]
    async fn api_tokens_can_never_mutate() {
        for uri in post_routes() {
            let (s, _, body) =
                send(state(), req(Method::POST, &uri, &[("authorization", "Bearer cha_anything")], "{}")).await;
            assert_eq!(s, StatusCode::FORBIDDEN, "POST {uri}");
            assert_eq!(body["message"], "forbidden: API tokens are read-only", "POST {uri}");
        }
        // Rejected before any lookup, even with a cookie present as well.
        let (s, _, _) = send(
            state(),
            req(
                Method::POST,
                &format!("{SCOPED}/tokens"),
                &[("authorization", "Bearer cha_x"), ("cookie", "karbird_session=chs_x")],
                "{}",
            ),
        )
        .await;
        assert_eq!(s, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn bearer_values_that_are_not_api_tokens_are_401() {
        for token in ["chs_session_token", "chi_ingest_token", "chc_csrf", "", "cha"] {
            let auth = format!("Bearer {token}");
            let (s, _, _) =
                send(state(), req(Method::GET, &format!("{SCOPED}/services"), &[("authorization", &auth)], "")).await;
            assert_eq!(s, StatusCode::UNAUTHORIZED, "{token:?}");
        }
    }

    #[tokio::test]
    async fn storage_outage_fails_closed_without_leaking_details() {
        for headers in [vec![("cookie", "karbird_session=chs_x")], vec![("authorization", "Bearer cha_x")]] {
            let (s, _, body) = send(state(), req(Method::GET, &format!("{SCOPED}/services"), &headers, "")).await;
            assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{headers:?}");
            assert_eq!(body, serde_json::json!({ "code": "storage_unavailable", "message": "storage unavailable" }));
        }
        let (s, _, _) = send(state(), req(Method::GET, "/api/v1/ready", &[], "")).await;
        assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "ready reports storage state");
    }

    #[tokio::test]
    async fn login_checks_origin_and_input_before_touching_storage() {
        let login = "/api/v1/session/login";
        let body = r#"{"email":"admin@karbird.local","password":"pw"}"#;
        let (s, _, b) = send(state(), req(Method::POST, login, &[("origin", "http://evil.example")], body)).await;
        assert_eq!((s, b["message"].as_str()), (StatusCode::FORBIDDEN, Some("forbidden: origin not allowed")));

        let long_pw = format!(r#"{{"email":"a@b","password":"{}"}}"#, "x".repeat(1025));
        let long_email = format!(r#"{{"email":"{}@b","password":"x"}}"#, "a".repeat(260));
        for body in [r#"{"email":"   ","password":"x"}"#.to_owned(), long_pw, long_email] {
            let (s, _, _) = send(state(), req(Method::POST, login, &[], &body)).await;
            assert_eq!(s, StatusCode::UNAUTHORIZED, "invalid input is a plain 401");
        }

        // Plausible credentials reach storage, which is down: an error, never a session.
        let (s, h, _) = send(state(), req(Method::POST, login, &[("origin", "http://localhost:3000")], body)).await;
        assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
        assert!(h.get("set-cookie").is_none());
    }

    #[tokio::test]
    async fn login_throttle_applies_to_the_normalised_email() {
        let st = state();
        for _ in 0..10 {
            st.login_throttle.failure("victim@example.test").await;
        }
        let body = r#"{"email":"  Victim@Example.TEST ","password":"pw"}"#;
        let (s, _, b) = send(st, req(Method::POST, "/api/v1/session/login", &[], body)).await;
        assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(b["code"], "too_many_requests");
    }

    const REGISTER: &str = "/api/v1/session/register";
    const SIGNUP: &str = r#"{"email":"new@example.test","password":"correct horse battery"}"#;

    #[tokio::test]
    async fn register_checks_origin_policy_and_input_before_touching_storage() {
        let (s, _, b) = send(state(), req(Method::POST, REGISTER, &[("origin", "http://evil.example")], SIGNUP)).await;
        assert_eq!((s, b["message"].as_str()), (StatusCode::FORBIDDEN, Some("forbidden: origin not allowed")));

        let mut closed = state();
        closed.config.signup.open = false;
        let (s, h, b) = send(closed, req(Method::POST, REGISTER, &[("origin", "http://localhost:3000")], SIGNUP)).await;
        assert_eq!(
            (s, b["message"].as_str()),
            (StatusCode::FORBIDDEN, Some("forbidden: sign-up is disabled on this server"))
        );
        assert!(h.get("set-cookie").is_none());

        for body in [
            r#"{"email":"not-an-email","password":"correct horse battery"}"#,
            r#"{"email":"new@example.test","password":"short"}"#,
            r#"{"email":"new@example.test","password":"New@Example.test"}"#,
        ] {
            let (s, h, b) = send(state(), req(Method::POST, REGISTER, &[], body)).await;
            assert_eq!((s, b["code"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_request")), "{body}");
            assert!(h.get("set-cookie").is_none());
        }
        let (s, _, _) = send(state(), req(Method::POST, REGISTER, &[], r#"{"email":"a@example.test"}"#)).await;
        assert!(s.is_client_error(), "{s}");

        // Valid input reaches storage, which is down: an error, never a session.
        let (s, h, b) =
            send(state(), req(Method::POST, REGISTER, &[("origin", "http://localhost:3000")], SIGNUP)).await;
        assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(b["message"], "storage unavailable");
        assert!(h.get("set-cookie").is_none());
    }

    #[tokio::test]
    async fn register_is_rate_limited_across_all_clients() {
        let mut st = state();
        st.config.signup.per_hour = 2;
        for _ in 0..2 {
            let (s, _, _) = send(st.clone(), req(Method::POST, REGISTER, &[], SIGNUP)).await;
            assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "within the limit it reaches storage");
        }
        let other = r#"{"email":"someone.else@example.test","password":"correct horse battery"}"#;
        let (s, _, b) = send(st.clone(), req(Method::POST, REGISTER, &[], other)).await;
        assert_eq!((s, b["code"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("too_many_requests")));
        // Invalid input is rejected without spending the budget.
        let (s, _, _) = send(st, req(Method::POST, REGISTER, &[], r#"{"email":"x","password":"y"}"#)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn signup_is_closed_unless_explicitly_opened() {
        if std::env::var_os("KARBIRD_SIGNUP").is_none() {
            assert!(!super::api_config(&serve_args(&[])).signup.open, "default is closed");
        }
        let open = super::api_config(&serve_args(&["--signup", "open", "--signup-per-hour", "5"])).signup;
        assert!(open.open);
        assert_eq!(open.per_hour, 5);
        assert!(!super::api_config(&serve_args(&["--signup", "closed"])).signup.open);
        let mut argv =
            vec!["karbird-api", "--database-url", "x", "--clickhouse-user", "u", "--clickhouse-password", "p"];
        for bad in ["Open", "yes", "true", ""] {
            argv.extend_from_slice(&["--signup", bad]);
            assert!(super::ServeArgs::try_parse_from(&argv).is_err(), "{bad:?}");
            argv.truncate(7);
        }
    }

    #[tokio::test]
    async fn malformed_and_oversized_bodies_are_rejected() {
        let login = "/api/v1/session/login";
        let (s, _, _) = send(state(), req(Method::POST, login, &[], "{not json")).await;
        assert!(s.is_client_error(), "{s}");
        let (s, _, _) = send(state(), req(Method::POST, login, &[], r#"{"email":"a@b"}"#)).await;
        assert!(s.is_client_error(), "{s}");
        let big = format!(r#"{{"email":"a@b","password":"{}"}}"#, "x".repeat(1024 * 1024 + 1));
        let (s, _, _) = send(state(), req(Method::POST, login, &[], &big)).await;
        assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn unknown_routes_and_methods() {
        let (s, _, _) = send(state(), req(Method::GET, "/api/v1/admin", &[], "")).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        let (s, _, _) = send(state(), req(Method::DELETE, &format!("{SCOPED}/tokens"), &[], "")).await;
        assert_eq!(s, StatusCode::METHOD_NOT_ALLOWED);
    }

    #[test]
    fn openapi_documents_every_route_and_both_credentials() {
        let spec: Value = serde_json::from_str(&crate::openapi::spec_json().unwrap()).unwrap();
        let tpl = "/api/v1/projects/{project_id}/environments/{environment_id}";
        let mut expected: Vec<(String, &str)> = vec![
            ("/api/v1/health".into(), "get"),
            ("/api/v1/ready".into(), "get"),
            ("/api/v1/session/login".into(), "post"),
            ("/api/v1/session/register".into(), "post"),
            ("/api/v1/session/logout".into(), "post"),
            ("/api/v1/session".into(), "get"),
            ("/api/v1/auth/google/start".into(), "get"),
            ("/api/v1/auth/google/callback".into(), "get"),
            ("/api/v1/onboarding/first-project".into(), "post"),
            ("/api/v1/projects".into(), "get"),
            ("/api/v1/permissions".into(), "get"),
            ("/api/v1/projects/{project_id}/roles".into(), "get"),
            ("/api/v1/projects/{project_id}/roles".into(), "post"),
            ("/api/v1/projects/{project_id}/roles/{role_id}".into(), "patch"),
            ("/api/v1/projects/{project_id}/roles/{role_id}".into(), "delete"),
            ("/api/v1/projects/{project_id}/members".into(), "get"),
            ("/api/v1/projects/{project_id}/members/{user_id}".into(), "patch"),
            ("/api/v1/projects/{project_id}/members/{user_id}".into(), "delete"),
            ("/api/v1/projects".into(), "post"),
            ("/api/v1/projects/{project_id}".into(), "patch"),
            ("/api/v1/projects/{project_id}/leave".into(), "post"),
            ("/api/v1/account".into(), "patch"),
            ("/api/v1/account/password".into(), "post"),
            ("/api/v1/account/sessions".into(), "get"),
            ("/api/v1/account/sessions/revoke-others".into(), "post"),
            ("/api/v1/account/sessions/{session_id}".into(), "delete"),
            ("/api/v1/projects/{project_id}/invites".into(), "get"),
            ("/api/v1/projects/{project_id}/invites".into(), "post"),
            ("/api/v1/projects/{project_id}/invites/{invite_id}".into(), "delete"),
            ("/api/v1/invites/preview".into(), "post"),
            ("/api/v1/invites/accept".into(), "post"),
            ("/api/v1/organizations/{organization_id}/members".into(), "get"),
            ("/api/v1/organizations/{organization_id}/members".into(), "post"),
            ("/api/v1/organizations/{organization_id}/projects/{project_id}/members/{user_id}".into(), "put"),
            ("/api/v1/organizations/{organization_id}/projects/{project_id}/members/{user_id}".into(), "delete"),
            ("/api/v1/organizations/{organization_id}/invites".into(), "get"),
            ("/api/v1/organizations/{organization_id}/invites".into(), "post"),
            ("/api/v1/organizations/{organization_id}/invites/{invite_id}".into(), "delete"),
            ("/api/v1/organization-invites/accept".into(), "post"),
            ("/api/v1/admin/overview".into(), "get"),
            ("/api/v1/admin/permissions".into(), "get"),
            ("/api/v1/admin/projects".into(), "get"),
            ("/api/v1/admin/projects".into(), "post"),
            ("/api/v1/admin/projects/{project_id}".into(), "patch"),
            ("/api/v1/admin/projects/{project_id}".into(), "delete"),
            ("/api/v1/admin/projects/{project_id}/environments".into(), "post"),
            ("/api/v1/admin/projects/{project_id}/members".into(), "get"),
            ("/api/v1/admin/projects/{project_id}/members".into(), "put"),
            ("/api/v1/admin/projects/{project_id}/members/{user_id}".into(), "delete"),
            ("/api/v1/admin/users".into(), "get"),
            ("/api/v1/admin/users/{user_id}/disable".into(), "post"),
            ("/api/v1/admin/users/{user_id}/enable".into(), "post"),
            ("/api/v1/admin/users/{user_id}/sessions/revoke".into(), "post"),
            ("/api/v1/admin/members".into(), "get"),
            ("/api/v1/admin/members".into(), "post"),
            ("/api/v1/admin/members/{user_id}".into(), "patch"),
            ("/api/v1/admin/members/{user_id}".into(), "delete"),
            ("/api/v1/admin/roles".into(), "get"),
            ("/api/v1/admin/roles".into(), "post"),
            ("/api/v1/admin/roles/{role_id}".into(), "patch"),
            ("/api/v1/admin/roles/{role_id}".into(), "delete"),
            ("/api/v1/admin/audit".into(), "get"),
        ];
        for (p, m) in [
            ("/services", "get"),
            ("/topology", "get"),
            ("/traces", "get"),
            ("/traces/{trace_id}", "get"),
            ("/logs", "get"),
            ("/events", "get"),
            ("/timeline", "get"),
            ("/coverage", "get"),
            ("/edges/traffic", "get"),
            ("/infrastructure", "get"),
            ("/infrastructure/timeline", "get"),
            ("/live", "get"),
            ("/audit", "get"),
            ("/tokens", "get"),
            ("/tokens", "post"),
            ("/tokens/{token_id}/revoke", "post"),
            ("/tokens/{token_id}/rotate", "post"),
            ("/actions", "get"),
            ("/actions", "post"),
            ("/actions/catalog", "get"),
            ("/actions/{plan_id}", "get"),
            ("/actions/{plan_id}/approve", "post"),
            ("/actions/{plan_id}/reject", "post"),
            ("/actions/{plan_id}/cancel", "post"),
        ] {
            expected.push((format!("{tpl}{p}"), m));
        }
        let paths = spec["paths"].as_object().unwrap();
        for (p, m) in &expected {
            assert!(paths.get(p).and_then(|x| x.get(*m)).is_some(), "{m} {p} missing from OpenAPI");
        }
        let documented: usize = paths.values().map(|v| v.as_object().map_or(0, |o| o.len())).sum();
        assert_eq!(documented, expected.len(), "every documented operation is routed");
        let schemes = &spec["components"]["securitySchemes"];
        assert_eq!(schemes["api_token"]["scheme"], "bearer");
        assert_eq!(schemes["session"]["in"], "cookie");
        assert_eq!(schemes["session"]["name"], "karbird_session");
    }

    fn serve_args(extra: &[&str]) -> super::ServeArgs {
        let mut argv = vec![
            "karbird-api",
            "--database-url",
            "postgres://x@127.0.0.1/db",
            "--clickhouse-user",
            "u",
            "--clickhouse-password",
            "p",
        ];
        argv.extend_from_slice(extra);
        super::ServeArgs::try_parse_from(argv).unwrap()
    }

    #[test]
    fn live_cursor_remains_monotonic_for_same_timestamp_batches() {
        let mut cursor = 100;
        assert_eq!(super::advance_live_cursor(&mut cursor, 100), 101);
        assert_eq!(super::advance_live_cursor(&mut cursor, 100), 102);
        assert_eq!(super::advance_live_cursor(&mut cursor, 200), 200);
        assert_eq!(super::advance_live_cursor(&mut cursor, 199), 201);
    }

    #[test]
    fn live_group_id_is_stable_and_rejects_an_empty_override() {
        assert_eq!(super::resolve_live_group_id(Some("api-1"), Some("host-a")).unwrap(), "api-1");
        assert_eq!(super::resolve_live_group_id(None, Some("host-a")).unwrap(), "karbird-live-host-a");
        assert_eq!(super::resolve_live_group_id(None, None).unwrap(), "karbird-live-local");
        assert!(super::resolve_live_group_id(Some("  "), Some("host-a")).is_err());
    }

    #[test]
    fn cookies_are_secure_unless_explicitly_in_dev_mode() {
        assert!(super::api_config(&serve_args(&["--env", "production"])).secure_cookies);
        assert!(super::api_config(&serve_args(&["--env", "staging"])).secure_cookies);
        assert!(super::api_config(&serve_args(&["--env", "Dev"])).secure_cookies, "exact match only");
        assert!(super::api_config(&serve_args(&["--env", "dev "])).secure_cookies);
        assert!(!super::api_config(&serve_args(&["--env", "dev"])).secure_cookies);
        if std::env::var_os("KARBIRD_ENV").is_none() {
            assert!(super::api_config(&serve_args(&[])).secure_cookies, "default is production");
        }
    }

    #[test]
    fn retention_defaults_and_bounds() {
        let unset = [
            "KARBIRD_LOGS_RETENTION_DAYS",
            "KARBIRD_SPANS_RETENTION_DAYS",
            "KARBIRD_EVENTS_RETENTION_DAYS",
            "KARBIRD_METRICS_RETENTION_DAYS",
        ]
        .iter()
        .all(|v| std::env::var_os(v).is_none());
        if unset {
            assert_eq!(
                serve_args(&[]).retention.tables(),
                [
                    (super::Table::Logs, 14),
                    (super::Table::Spans, 14),
                    (super::Table::EdgeSpanFacts, 14),
                    (super::Table::Events, 30),
                    (super::Table::Metrics, 14)
                ]
            );
        }
        let mut argv =
            vec!["karbird-api", "--database-url", "x", "--clickhouse-user", "u", "--clickhouse-password", "p"];
        for flag in
            ["--logs-retention-days", "--spans-retention-days", "--events-retention-days", "--metrics-retention-days"]
        {
            for bad in ["0", "1", "3651", "-5", "two", ""] {
                argv.extend_from_slice(&[flag, bad]);
                assert!(super::ServeArgs::try_parse_from(&argv).is_err(), "{flag} {bad:?}");
                argv.truncate(7);
            }
            for ok in ["2", "3650"] {
                argv.extend_from_slice(&[flag, ok]);
                assert!(super::ServeArgs::try_parse_from(&argv).is_ok(), "{flag} {ok}");
                argv.truncate(7);
            }
        }
    }

    #[test]
    fn the_hot_window_follows_log_retention_and_stays_below_it() {
        let hours = |extra: &[&str]| super::hot_window_hours(&serve_args(extra));
        if std::env::var_os("KARBIRD_ARCHIVE_HOT_WINDOW_HOURS").is_none() {
            assert_eq!(hours(&["--logs-retention-days", "14"]).unwrap(), 13 * 24, "one day less by default");
            assert_eq!(hours(&["--logs-retention-days", "2"]).unwrap(), 24);
            assert_eq!(hours(&["--logs-retention-days", "90"]).unwrap(), 89 * 24);
        }
        assert_eq!(hours(&["--logs-retention-days", "14", "--archive-hot-window-hours", "48"]).unwrap(), 48);
        assert_eq!(hours(&["--logs-retention-days", "14", "--archive-hot-window-hours", "335"]).unwrap(), 335);
        for too_long in ["336", "1000"] {
            let e = hours(&["--logs-retention-days", "14", "--archive-hot-window-hours", too_long]).unwrap_err();
            let msg = e.to_string();
            assert!(msg.contains("KARBIRD_ARCHIVE_HOT_WINDOW_HOURS") && msg.contains("336 h"), "{msg}");
        }
        let args = serve_args(&[
            "--logs-retention-days",
            "7",
            "--archive-hot-window-hours",
            "200",
            "--archive-s3-endpoint",
            "http://127.0.0.1:1",
            "--archive-s3-access-key",
            "a",
            "--archive-s3-secret-key",
            "s",
        ]);
        assert!(super::archive_config(&args).is_err(), "the API refuses to start with such a window");
    }

    #[test]
    fn allowed_origins_are_split_and_trimmed() {
        let c = super::api_config(&serve_args(&["--allowed-origins", " https://a.example , https://b.example"]));
        assert_eq!(c.allowed_origins, ["https://a.example", "https://b.example"]);
        assert_eq!(c.session_ttl, time::Duration::hours(12));
    }

    #[test]
    fn cli_subcommands_parse() {
        assert!(matches!(
            super::Cli::try_parse_from(["karbird-api", "openapi"]).unwrap().command,
            Some(super::Command::Openapi)
        ));
        assert!(super::Cli::try_parse_from(["karbird-api"]).unwrap().command.is_none());
        assert!(super::Cli::try_parse_from(["karbird-api", "drop-database"]).is_err());
        match super::Cli::try_parse_from(["karbird-api", "migrate", "--logs-retention-days", "7"]).unwrap().command {
            Some(super::Command::Migrate { retention, shrink_retention }) => {
                assert!(!shrink_retention, "deleting data is never the default");
                assert_eq!(retention.tables()[0], (super::Table::Logs, 7));
            }
            other => panic!("{other:?}"),
        }
        assert!(
            super::Cli::try_parse_from(["karbird-api", "migrate", "--out-dir", "x"]).is_err(),
            "migrate writes no files"
        );
        match super::Cli::try_parse_from(["karbird-api", "bootstrap"]).unwrap().command {
            Some(super::Command::Bootstrap { out_dir, shrink_retention, .. }) => {
                assert_eq!(out_dir, std::path::PathBuf::from(".karbird"));
                assert!(!shrink_retention, "deleting data is never the default");
            }
            other => panic!("{other:?}"),
        }
        match super::Cli::try_parse_from([
            "karbird-api",
            "bootstrap",
            "--shrink-retention",
            "--logs-retention-days",
            "7",
            "--spans-retention-days",
            "3",
            "--events-retention-days",
            "90",
            "--metrics-retention-days",
            "5",
        ])
        .unwrap()
        .command
        {
            Some(super::Command::Bootstrap { retention, shrink_retention, .. }) => {
                assert!(shrink_retention);
                assert_eq!(
                    retention.tables(),
                    [
                        (super::Table::Logs, 7),
                        (super::Table::Spans, 3),
                        (super::Table::EdgeSpanFacts, 3),
                        (super::Table::Events, 90),
                        (super::Table::Metrics, 5)
                    ]
                );
            }
            other => panic!("{other:?}"),
        }
        assert!(
            super::ServeArgs::try_parse_from([
                "x",
                "--addr",
                "not-an-addr",
                "--database-url",
                "d",
                "--clickhouse-user",
                "u",
                "--clickhouse-password",
                "p"
            ])
            .is_err()
        );
    }
}
