//! controlhub-api: sessions, scoped telemetry queries, timeline and audit.
//! Has read-only ClickHouse credentials and no infrastructure-control access.

mod auth;
mod bootstrap;
mod error;
mod openapi;
mod routes;
mod state;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use axum::Router;
use axum::http::{HeaderValue, StatusCode, header};
use axum::routing::{get, post};
use clap::{Parser, Subcommand};
use controlhub_runtime::{init_tracing, load_local_env, shutdown_signal};
use controlhub_storage::ch::{ChConfig, ChStore};
use controlhub_storage::pg::PgStore;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

#[derive(Debug, Parser)]
#[command(name = "controlhub-api")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the HTTP API (default).
    Serve(ServeArgs),
    /// Local development bootstrap (migrations, service users, seed data).
    Bootstrap {
        #[arg(long, default_value = ".controlhub")]
        out_dir: PathBuf,
    },
    /// Print the OpenAPI document.
    Openapi,
}

#[derive(Debug, Parser)]
struct ServeArgs {
    #[arg(long, env = "CONTROLHUB_API_ADDR", default_value = "127.0.0.1:8080")]
    addr: SocketAddr,
    #[arg(long, env = "CONTROLHUB_API_DATABASE_URL")]
    database_url: String,
    #[arg(long, env = "CONTROLHUB_CLICKHOUSE_URL", default_value = "http://127.0.0.1:18123")]
    clickhouse_url: String,
    #[arg(long, env = "CONTROLHUB_CLICKHOUSE_DATABASE", default_value = "controlhub")]
    clickhouse_database: String,
    #[arg(long, env = "CONTROLHUB_API_CLICKHOUSE_USER")]
    clickhouse_user: String,
    #[arg(long, env = "CONTROLHUB_API_CLICKHOUSE_PASSWORD")]
    clickhouse_password: String,
    /// Comma-separated origins allowed for cookie-authenticated mutations.
    #[arg(long, env = "CONTROLHUB_ALLOWED_ORIGINS", default_value = "http://localhost:3000,http://127.0.0.1:3000")]
    allowed_origins: String,
    /// `dev` disables the Secure cookie flag for plain-http localhost. Anything else enforces it.
    #[arg(long, env = "CONTROLHUB_ENV", default_value = "production")]
    env: String,
    /// Maximum concurrent live WebSocket connections; further upgrades get 429.
    #[arg(long, env = "CONTROLHUB_MAX_LIVE_CONNECTIONS", default_value_t = 256)]
    max_live_connections: usize,
}

fn router(state: state::AppState) -> Router {
    use routes::{session, telemetry};
    let scoped = "/api/v1/projects/{project_id}/environments/{environment_id}";
    Router::new()
        .route("/api/v1/health", get(routes::health))
        .route("/api/v1/ready", get(routes::ready))
        .route("/api/v1/session/login", post(session::login))
        .route("/api/v1/session/logout", post(session::logout))
        .route("/api/v1/session", get(session::session))
        .route("/api/v1/projects", get(session::projects))
        .route(&format!("{scoped}/services"), get(telemetry::services))
        .route(&format!("{scoped}/topology"), get(telemetry::topology))
        .route(&format!("{scoped}/traces"), get(telemetry::traces))
        .route(&format!("{scoped}/traces/{{trace_id}}"), get(telemetry::trace_detail))
        .route(&format!("{scoped}/logs"), get(telemetry::logs))
        .route(&format!("{scoped}/events"), get(telemetry::events))
        .route(&format!("{scoped}/timeline"), get(telemetry::timeline))
        .route(&format!("{scoped}/coverage"), get(telemetry::coverage))
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
        .layer(RequestBodyLimitLayer::new(1024 * 1024))
        .layer(TimeoutLayer::with_status_code(StatusCode::SERVICE_UNAVAILABLE, Duration::from_secs(30)))
        .layer(SetResponseHeaderLayer::overriding(header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
        .layer(SetResponseHeaderLayer::overriding(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn serve(args: ServeArgs) -> anyhow::Result<()> {
    let pg = PgStore::connect(&args.database_url, 16).await?;
    let ch = ChStore::new(&ChConfig {
        url: args.clickhouse_url,
        database: args.clickhouse_database,
        user: args.clickhouse_user,
        password: args.clickhouse_password,
    });
    let dev = args.env == "dev";
    if dev {
        tracing::warn!("CONTROLHUB_ENV=dev: Secure cookie flag disabled for local http. Never use in production.");
    }
    let state = state::AppState {
        pg,
        ch,
        config: state::ApiConfig {
            allowed_origins: args.allowed_origins.split(',').map(|s| s.trim().to_owned()).collect(),
            secure_cookies: !dev,
            session_ttl: time::Duration::hours(12),
        },
        login_throttle: auth::LoginThrottle::default(),
        live_slots: std::sync::Arc::new(tokio::sync::Semaphore::new(args.max_live_connections)),
    };
    let listener = tokio::net::TcpListener::bind(args.addr).await?;
    tracing::info!(addr = %args.addr, "controlhub-api listening");
    axum::serve(listener, router(state)).with_graceful_shutdown(shutdown_signal()).await?;
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
        Some(Command::Bootstrap { out_dir }) => {
            init_tracing("controlhub-api");
            bootstrap::run(&out_dir).await
        }
        Some(Command::Serve(args)) => {
            init_tracing("controlhub-api");
            serve(args).await
        }
        None => {
            init_tracing("controlhub-api");
            serve(ServeArgs::parse_from(["controlhub-api"])).await
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
        !controlhub_storage::secrets::verify_password("x", s) && s.starts_with("$argon2id$")
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
        let mut e = controlhub_engine_core::Engine::new(Default::default());
        e.apply_batch_json(json).unwrap();
        assert_eq!(e.item_count(), 1);
    }
}
