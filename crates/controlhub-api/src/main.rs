//! controlhub-api: sessions, scoped telemetry queries, timeline and audit.
//! Has read-only ClickHouse credentials and no infrastructure-control access.

mod auth;
mod bootstrap;
mod error;
mod openapi;
mod permissions;
mod routes;
mod state;
mod test_fixtures;

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
    Serve(Box<ServeArgs>),
    /// Local development bootstrap (migrations, service users, seed data).
    Bootstrap {
        #[arg(long, default_value = ".controlhub")]
        out_dir: PathBuf,
    },
    /// Create (or with --remove delete) the accounts and scopes the integration/e2e tests use.
    TestFixtures {
        #[arg(long, default_value = ".controlhub")]
        out_dir: PathBuf,
        #[arg(long)]
        remove: bool,
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
    /// Self-service sign-up at `POST /api/v1/session/register`: `open` or `closed`.
    #[arg(long, env = "CONTROLHUB_SIGNUP", default_value = "closed", value_parser = ["open", "closed"])]
    signup: String,
    /// Sign-up attempts allowed per hour across all clients.
    #[arg(long, env = "CONTROLHUB_SIGNUP_PER_HOUR", default_value_t = 20)]
    signup_per_hour: u32,
}

fn router(state: state::AppState) -> Router {
    use routes::{account, invites, roles, session, telemetry};
    let scoped = "/api/v1/projects/{project_id}/environments/{environment_id}";
    Router::new()
        .route("/api/v1/health", get(routes::health))
        .route("/api/v1/ready", get(routes::ready))
        .route("/api/v1/session/login", post(session::login))
        .route("/api/v1/session/register", post(routes::signup::register))
        .route("/api/v1/session/logout", post(session::logout))
        .route("/api/v1/session", get(session::session))
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

fn api_config(args: &ServeArgs) -> state::ApiConfig {
    state::ApiConfig {
        allowed_origins: args.allowed_origins.split(',').map(|s| s.trim().to_owned()).collect(),
        secure_cookies: args.env != "dev",
        session_ttl: time::Duration::hours(12),
        signup: state::SignupPolicy { open: args.signup == "open", per_hour: args.signup_per_hour },
    }
}

async fn serve(args: ServeArgs) -> anyhow::Result<()> {
    let config = api_config(&args);
    let pg = PgStore::connect(&args.database_url, 16).await?;
    let ch = ChStore::new(&ChConfig {
        url: args.clickhouse_url,
        database: args.clickhouse_database,
        user: args.clickhouse_user,
        password: args.clickhouse_password,
    });
    if !config.secure_cookies {
        tracing::warn!("CONTROLHUB_ENV=dev: Secure cookie flag disabled for local http. Never use in production.");
    }
    let state = state::AppState {
        pg,
        ch,
        config,
        login_throttle: auth::LoginThrottle::default(),
        signup_throttle: routes::signup::SignupThrottle::default(),
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
        Some(Command::TestFixtures { out_dir, remove }) => {
            init_tracing("controlhub-api");
            if remove { test_fixtures::remove(&out_dir).await } else { test_fixtures::create(&out_dir).await }
        }
        Some(Command::Serve(args)) => {
            init_tracing("controlhub-api");
            serve(*args).await
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
            let (s, _, _) = send(state(), req(m.clone(), &uri, &[("cookie", "controlhub_session=chs_x")], body)).await;
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
            let (s, _, _) = send(state(), req(m.clone(), &uri, &[("cookie", "controlhub_session=chs_x")], body)).await;
            assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{m} {uri}: session lookup fails closed");
        }
        for uri in ["/api/v1/account/sessions", "/api/v1/projects/demo/invites"] {
            let (s, _, _) = send(state(), req(Method::GET, uri, &[], "")).await;
            assert_eq!(s, StatusCode::UNAUTHORIZED, "GET {uri}");
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
                &[("authorization", "Bearer cha_x"), ("cookie", "controlhub_session=chs_x")],
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
        for headers in [vec![("cookie", "controlhub_session=chs_x")], vec![("authorization", "Bearer cha_x")]] {
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
        let body = r#"{"email":"admin@controlhub.local","password":"pw"}"#;
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
        if std::env::var_os("CONTROLHUB_SIGNUP").is_none() {
            assert!(!super::api_config(&serve_args(&[])).signup.open, "default is closed");
        }
        let open = super::api_config(&serve_args(&["--signup", "open", "--signup-per-hour", "5"])).signup;
        assert!(open.open);
        assert_eq!(open.per_hour, 5);
        assert!(!super::api_config(&serve_args(&["--signup", "closed"])).signup.open);
        let mut argv =
            vec!["controlhub-api", "--database-url", "x", "--clickhouse-user", "u", "--clickhouse-password", "p"];
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
        assert_eq!(schemes["session"]["name"], "controlhub_session");
    }

    fn serve_args(extra: &[&str]) -> super::ServeArgs {
        let mut argv = vec![
            "controlhub-api",
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
    fn cookies_are_secure_unless_explicitly_in_dev_mode() {
        assert!(super::api_config(&serve_args(&["--env", "production"])).secure_cookies);
        assert!(super::api_config(&serve_args(&["--env", "staging"])).secure_cookies);
        assert!(super::api_config(&serve_args(&["--env", "Dev"])).secure_cookies, "exact match only");
        assert!(super::api_config(&serve_args(&["--env", "dev "])).secure_cookies);
        assert!(!super::api_config(&serve_args(&["--env", "dev"])).secure_cookies);
        if std::env::var_os("CONTROLHUB_ENV").is_none() {
            assert!(super::api_config(&serve_args(&[])).secure_cookies, "default is production");
        }
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
            super::Cli::try_parse_from(["controlhub-api", "openapi"]).unwrap().command,
            Some(super::Command::Openapi)
        ));
        assert!(super::Cli::try_parse_from(["controlhub-api"]).unwrap().command.is_none());
        assert!(super::Cli::try_parse_from(["controlhub-api", "drop-database"]).is_err());
        match super::Cli::try_parse_from(["controlhub-api", "bootstrap"]).unwrap().command {
            Some(super::Command::Bootstrap { out_dir }) => assert_eq!(out_dir, std::path::PathBuf::from(".controlhub")),
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
