//! Browser sessions (HttpOnly cookie), CSRF double-submit protection and
//! project/environment authorization.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{FromRequestParts, OptionalFromRequestParts};
use axum::http::request::Parts;
use axum::http::{Method, header};
use controlhub_domain::{EnvironmentId, ProjectId, Scope, TenantId};
use controlhub_storage::secrets::{constant_time_eq, hash_token};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::error::ApiError;
use crate::state::AppState;

pub const SESSION_COOKIE: &str = "controlhub_session";
pub const CSRF_COOKIE: &str = "controlhub_csrf";
pub const CSRF_HEADER: &str = "x-csrf-token";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Viewer = 0,
    Operator = 1,
    Admin = 2,
    Owner = 3,
}

impl Role {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "viewer" => Some(Role::Viewer),
            "operator" => Some(Role::Operator),
            "admin" => Some(Role::Admin),
            "owner" => Some(Role::Owner),
            _ => None,
        }
    }
}

pub const API_TOKEN_PREFIX: &str = "cha_";

/// How the request was authenticated.
#[derive(Debug, Clone)]
pub enum Credential {
    /// Browser session (HttpOnly cookie + CSRF for mutations).
    Session(Uuid),
    /// Read-only bearer API token bound to exactly one environment.
    ApiToken { token_id: Uuid, scope: Scope },
}

#[derive(Debug, Clone)]
pub struct CurrentUser {
    pub credential: Credential,
    pub user_id: Uuid,
    pub email: String,
    pub display_name: String,
}

impl CurrentUser {
    pub fn session_id(&self) -> Result<Uuid, ApiError> {
        match &self.credential {
            Credential::Session(id) => Ok(*id),
            Credential::ApiToken { .. } => Err(ApiError::Forbidden("requires a browser session")),
        }
    }

    pub fn is_session(&self) -> bool {
        matches!(self.credential, Credential::Session(_))
    }

    /// Long-lived connections re-check this so logout, revocation or expiry closes them.
    pub async fn still_active(&self, state: &AppState) -> Result<bool, ApiError> {
        Ok(match &self.credential {
            Credential::Session(id) => state.pg.session_is_active(*id).await?,
            Credential::ApiToken { token_id, .. } => state.pg.api_token_is_active(*token_id).await?,
        })
    }
}

fn bearer(parts: &Parts) -> Option<&str> {
    let v = parts.headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = v.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then(|| token.trim())
}

pub fn cookie_value<'a>(parts: &'a Parts, name: &str) -> Option<&'a str> {
    parts
        .headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
}

/// Origin must match an allowed origin when present (browsers always send it
/// on cross-site and same-origin POST requests).
pub fn check_origin(state: &AppState, headers: &axum::http::HeaderMap) -> Result<(), ApiError> {
    match headers.get(header::ORIGIN).map(|v| v.to_str()) {
        None => Ok(()),
        Some(Ok(origin)) if state.config.allowed_origins.iter().any(|o| o == origin) => Ok(()),
        Some(_) => Err(ApiError::Forbidden("origin not allowed")),
    }
}

fn is_mutation(method: &Method) -> bool {
    !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

/// Cookie-authenticated mutations: allowed Origin (when sent) and the session's CSRF token.
fn check_csrf(state: &AppState, headers: &axum::http::HeaderMap, csrf_token_hash: &[u8]) -> Result<(), ApiError> {
    check_origin(state, headers)?;
    let header_token =
        headers.get(CSRF_HEADER).and_then(|v| v.to_str().ok()).ok_or(ApiError::Forbidden("missing CSRF token"))?;
    if !constant_time_eq(&hash_token(header_token), csrf_token_hash) {
        return Err(ApiError::Forbidden("invalid CSRF token"));
    }
    Ok(())
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        if let Some(token) = bearer(parts) {
            // API tokens are read-only: no mutation is ever authorized by one.
            if is_mutation(&parts.method) {
                return Err(ApiError::Forbidden("API tokens are read-only"));
            }
            if !token.starts_with(API_TOKEN_PREFIX) {
                return Err(ApiError::Unauthenticated);
            }
            let p = state.pg.find_api_token(&hash_token(token)).await?.ok_or(ApiError::Unauthenticated)?;
            return Ok(CurrentUser {
                credential: Credential::ApiToken { token_id: p.token_id, scope: p.scope },
                user_id: p.user_id,
                email: p.email,
                display_name: p.display_name,
            });
        }
        let token = cookie_value(parts, SESSION_COOKIE).ok_or(ApiError::Unauthenticated)?;
        let session = state.pg.find_session(&hash_token(token)).await?.ok_or(ApiError::Unauthenticated)?;

        if is_mutation(&parts.method) {
            check_csrf(state, &parts.headers, &session.csrf_token_hash)?;
        }
        Ok(CurrentUser {
            credential: Credential::Session(session.session_id),
            user_id: session.user_id,
            email: session.email,
            display_name: session.display_name,
        })
    }
}

impl OptionalFromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Option<Self>, Self::Rejection> {
        match <CurrentUser as FromRequestParts<AppState>>::from_request_parts(parts, state).await {
            Ok(u) => Ok(Some(u)),
            Err(ApiError::Unauthenticated) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/// Resolve and authorize a project/environment scope for the user.
/// Unknown and unauthorized scopes both return 404 so existence is not leaked.
pub async fn authorize(
    state: &AppState,
    user: &CurrentUser,
    project_id: &str,
    environment_id: &str,
    min: Role,
) -> Result<(Scope, bool), ApiError> {
    let project = ProjectId::parse(project_id).map_err(|_| ApiError::NotFound)?;
    let env = EnvironmentId::parse(environment_id).map_err(|_| ApiError::NotFound)?;
    if let Credential::ApiToken { scope, .. } = &user.credential {
        if scope.project_id != project || scope.environment_id != env {
            return Err(ApiError::NotFound);
        }
        if min > Role::Viewer {
            return Err(ApiError::Forbidden("API tokens are read-only"));
        }
    }
    let memberships = state.pg.memberships(user.user_id).await?;
    let m = memberships.iter().find(|m| m.project_id == project.as_str()).ok_or(ApiError::NotFound)?;
    let role = Role::parse(&m.role).ok_or(ApiError::Forbidden("unknown role"))?;
    let env_row = state.pg.environment(project.as_str(), env.as_str()).await?.ok_or(ApiError::NotFound)?;
    if env_row.tenant_id != m.tenant_id {
        return Err(ApiError::NotFound);
    }
    if role < min {
        return Err(ApiError::Forbidden("insufficient role"));
    }
    let tenant = TenantId::parse(m.tenant_id.clone()).map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok((Scope { tenant_id: tenant, project_id: project, environment_id: env }, env_row.is_demo))
}

/// Simple in-memory login throttle: max failures per key within a window.
#[derive(Debug, Clone, Default)]
pub struct LoginThrottle {
    inner: Arc<Mutex<HashMap<String, (u32, Instant)>>>,
}

impl LoginThrottle {
    const WINDOW: Duration = Duration::from_secs(15 * 60);
    const MAX_FAILURES: u32 = 10;

    pub async fn check(&self, key: &str) -> Result<(), ApiError> {
        let map = self.inner.lock().await;
        match map.get(key) {
            Some((n, since)) if since.elapsed() < Self::WINDOW && *n >= Self::MAX_FAILURES => {
                Err(ApiError::TooManyRequests)
            }
            _ => Ok(()),
        }
    }

    pub async fn failure(&self, key: &str) {
        let mut map = self.inner.lock().await;
        if map.len() > 50_000 {
            map.retain(|_, (_, since)| since.elapsed() < Self::WINDOW);
        }
        let e = map.entry(key.to_owned()).or_insert((0, Instant::now()));
        if e.1.elapsed() >= Self::WINDOW {
            *e = (0, Instant::now());
        }
        e.0 += 1;
    }

    pub async fn success(&self, key: &str) {
        self.inner.lock().await.remove(key);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::state::test_state;
    use axum::http::{HeaderMap, HeaderValue, Request};

    fn parts(headers: &[(&str, &[u8])]) -> Parts {
        let mut b = Request::builder().uri("/");
        for (k, v) in headers {
            b = b.header(*k, HeaderValue::from_bytes(v).unwrap());
        }
        b.body(()).unwrap().into_parts().0
    }

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        parts(&pairs.iter().map(|(k, v)| (*k, v.as_bytes())).collect::<Vec<_>>()).headers
    }

    fn forbidden(r: Result<(), ApiError>) -> &'static str {
        match r {
            Err(ApiError::Forbidden(m)) => m,
            other => panic!("expected 403, got {other:?}"),
        }
    }

    #[test]
    fn roles_parse_exactly_and_are_ordered() {
        assert_eq!(Role::parse("viewer"), Some(Role::Viewer));
        assert_eq!(Role::parse("operator"), Some(Role::Operator));
        assert_eq!(Role::parse("admin"), Some(Role::Admin));
        assert_eq!(Role::parse("owner"), Some(Role::Owner));
        for bad in ["", "Admin", "ADMIN", " admin", "superuser", "root"] {
            assert_eq!(Role::parse(bad), None, "{bad:?}");
        }
        assert!(Role::Viewer < Role::Operator && Role::Operator < Role::Admin && Role::Admin < Role::Owner);
    }

    #[test]
    fn bearer_tokens_are_parsed_case_insensitively_by_scheme() {
        assert_eq!(bearer(&parts(&[("authorization", b"Bearer cha_x")])), Some("cha_x"));
        assert_eq!(bearer(&parts(&[("authorization", b"bearer  cha_x ")])), Some("cha_x"));
        assert_eq!(bearer(&parts(&[("authorization", b"BEARER cha_x")])), Some("cha_x"));
        assert_eq!(bearer(&parts(&[("authorization", b"Basic dXNlcjpwYXNz")])), None);
        assert_eq!(bearer(&parts(&[("authorization", b"Bearer")])), None);
        assert_eq!(bearer(&parts(&[("authorization", b"cha_x")])), None);
        assert_eq!(bearer(&parts(&[("authorization", b"Bearer \xff\xfe")])), None, "non-ASCII header");
        assert_eq!(bearer(&parts(&[])), None);
    }

    #[test]
    fn cookie_values_are_matched_by_exact_name() {
        let p = parts(&[("cookie", b"a=1; controlhub_session=chs_abc; controlhub_csrf=chc_x=y")]);
        assert_eq!(cookie_value(&p, SESSION_COOKIE), Some("chs_abc"));
        assert_eq!(cookie_value(&p, CSRF_COOKIE), Some("chc_x=y"), "values may contain =");
        assert_eq!(cookie_value(&p, "controlhub"), None, "no prefix matching");
        let p = parts(&[("cookie", b"xcontrolhub_session=evil")]);
        assert_eq!(cookie_value(&p, SESSION_COOKIE), None);
        let p = parts(&[("cookie", b"a=1"), ("cookie", b"controlhub_session=second")]);
        assert_eq!(cookie_value(&p, SESSION_COOKIE), Some("second"), "multiple Cookie headers");
        let p = parts(&[("cookie", b"controlhub_session=first; controlhub_session=dup")]);
        assert_eq!(cookie_value(&p, SESSION_COOKIE), Some("first"));
        assert_eq!(cookie_value(&parts(&[("cookie", b"controlhub_session")]), SESSION_COOKIE), None);
        assert_eq!(cookie_value(&parts(&[]), SESSION_COOKIE), None);
    }

    #[test]
    fn only_safe_methods_are_not_mutations() {
        for m in [Method::GET, Method::HEAD, Method::OPTIONS] {
            assert!(!is_mutation(&m), "{m}");
        }
        for m in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE, Method::TRACE, Method::CONNECT] {
            assert!(is_mutation(&m), "{m}");
        }
        assert!(is_mutation(&Method::from_bytes(b"PROPFIND").unwrap()), "unknown methods count as mutations");
    }

    #[tokio::test]
    async fn origin_must_be_allowlisted_when_present() {
        let s = test_state(true);
        assert!(check_origin(&s, &headers(&[])).is_ok(), "same-origin GET/non-browser clients send none");
        assert!(check_origin(&s, &headers(&[("origin", "http://localhost:3000")])).is_ok());
        for o in [
            "http://localhost:3000/",
            "http://localhost:3001",
            "https://localhost:3000",
            "http://evil.example",
            "http://localhost:3000.evil.example",
            "null",
            "",
        ] {
            assert_eq!(forbidden(check_origin(&s, &headers(&[("origin", o)]))), "origin not allowed", "{o:?}");
        }
        let p = parts(&[("origin", b"http://localhost:3000\xff")]);
        assert_eq!(forbidden(check_origin(&s, &p.headers)), "origin not allowed", "non-UTF-8 Origin");
    }

    #[tokio::test]
    async fn csrf_token_must_match_the_session() {
        let s = test_state(true);
        let good = "chc_good";
        let hash = hash_token(good);
        assert!(check_csrf(&s, &headers(&[(CSRF_HEADER, good)]), &hash).is_ok());
        assert!(check_csrf(&s, &headers(&[(CSRF_HEADER, good), ("origin", "http://127.0.0.1:3000")]), &hash).is_ok());
        assert_eq!(forbidden(check_csrf(&s, &headers(&[]), &hash)), "missing CSRF token");
        assert_eq!(forbidden(check_csrf(&s, &headers(&[(CSRF_HEADER, "chc_bad")]), &hash)), "invalid CSRF token");
        assert_eq!(forbidden(check_csrf(&s, &headers(&[(CSRF_HEADER, "")]), &hash)), "invalid CSRF token");
        assert_eq!(
            forbidden(check_csrf(&s, &headers(&[(CSRF_HEADER, good.to_uppercase().as_str())]), &hash)),
            "invalid CSRF token"
        );
        // The raw hash is not accepted in place of the token.
        let hex_hash: String = hash.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            forbidden(check_csrf(&s, &headers(&[(CSRF_HEADER, hex_hash.as_str())]), &hash)),
            "invalid CSRF token"
        );
        // A foreign Origin fails even with a valid token (checked first).
        let h = headers(&[(CSRF_HEADER, good), ("origin", "http://evil.example")]);
        assert_eq!(forbidden(check_csrf(&s, &h, &hash)), "origin not allowed");
        // The session cookie's own value is not a CSRF token.
        assert_eq!(forbidden(check_csrf(&s, &headers(&[(CSRF_HEADER, "chs_session")]), &hash)), "invalid CSRF token");
    }

    fn scope(p: &str, e: &str) -> Scope {
        Scope {
            tenant_id: TenantId::parse("local").unwrap(),
            project_id: ProjectId::parse(p).unwrap(),
            environment_id: EnvironmentId::parse(e).unwrap(),
        }
    }

    fn token_user(p: &str, e: &str) -> CurrentUser {
        CurrentUser {
            credential: Credential::ApiToken { token_id: Uuid::nil(), scope: scope(p, e) },
            user_id: Uuid::nil(),
            email: "a@example.test".into(),
            display_name: "A".into(),
        }
    }

    #[tokio::test]
    async fn api_tokens_are_bound_to_one_environment_without_revealing_others() {
        let s = test_state(true);
        let u = token_user("demo", "local-demo");
        for (p, e) in [("demo", "other-env"), ("other", "local-demo"), ("isolation", "isolation-test")] {
            assert!(
                matches!(authorize(&s, &u, p, e, Role::Viewer).await, Err(ApiError::NotFound)),
                "{p}/{e} must be 404 (not 403) for a token of another scope"
            );
        }
    }

    #[tokio::test]
    async fn api_tokens_never_get_more_than_viewer() {
        let s = test_state(true);
        let u = token_user("demo", "local-demo");
        for min in [Role::Operator, Role::Admin, Role::Owner] {
            match authorize(&s, &u, "demo", "local-demo", min).await {
                Err(ApiError::Forbidden(m)) => assert_eq!(m, "API tokens are read-only"),
                other => panic!("{min:?}: {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn malformed_scope_ids_are_404_before_any_lookup() {
        let s = test_state(true);
        let session = CurrentUser {
            credential: Credential::Session(Uuid::nil()),
            user_id: Uuid::nil(),
            email: String::new(),
            display_name: String::new(),
        };
        for (p, e) in [("", "env"), ("demo", ""), ("de mo", "env"), ("demo", "env/../x"), ("demo", "e'; --")] {
            assert!(
                matches!(authorize(&s, &session, p, e, Role::Viewer).await, Err(ApiError::NotFound)),
                "{p:?}/{e:?}"
            );
        }
        // Well-formed IDs reach storage, which is down here: that must be an error, never access.
        assert!(matches!(authorize(&s, &session, "demo", "local-demo", Role::Viewer).await, Err(ApiError::Storage(_))));
    }

    #[test]
    fn session_id_requires_a_browser_session() {
        let t = token_user("demo", "local-demo");
        assert!(!t.is_session());
        assert!(matches!(t.session_id(), Err(ApiError::Forbidden("requires a browser session"))));
        let id = Uuid::now_v7();
        let s = CurrentUser { credential: Credential::Session(id), ..t };
        assert!(s.is_session());
        assert_eq!(s.session_id().unwrap(), id);
    }

    #[tokio::test]
    async fn login_throttle_blocks_after_max_failures_per_key() {
        let t = LoginThrottle::default();
        for _ in 0..LoginThrottle::MAX_FAILURES - 1 {
            t.failure("a@x").await;
        }
        assert!(t.check("a@x").await.is_ok(), "below the limit");
        t.failure("a@x").await;
        assert!(matches!(t.check("a@x").await, Err(ApiError::TooManyRequests)));
        assert!(t.check("b@x").await.is_ok(), "other keys are unaffected");
        t.success("a@x").await;
        assert!(t.check("a@x").await.is_ok(), "success resets");
    }
}
