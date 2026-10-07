//! Browser sessions (HttpOnly cookie), CSRF double-submit protection and
//! project/environment authorization.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{FromRequestParts, OptionalFromRequestParts};
use axum::http::request::Parts;
use axum::http::{Method, header};
use karbird_domain::{EnvironmentId, ProjectId, Scope, TenantId};
use karbird_storage::secrets::{constant_time_eq, hash_token};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::error::ApiError;
use crate::permissions::{self, Actor, Permission, PermissionSet};
use crate::state::AppState;

fn has_organization_access(
    memberships: &[karbird_storage::pg::OrganizationMemberRow],
    user_id: Uuid,
    organization_id: &str,
) -> bool {
    memberships.iter().any(|m| m.user_id == user_id && m.organization_id == organization_id)
}

pub const SESSION_COOKIE: &str = "karbird_session";
pub const CSRF_COOKIE: &str = "karbird_csrf";
pub const CSRF_HEADER: &str = "x-csrf-token";

/// The most an API token can do, whatever its owner may do: read telemetry and plans.
pub const API_TOKEN_PERMISSIONS: [Permission; 2] = [Permission::TelemetryView, Permission::ActionsView];

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

/// Effective permissions of a credential: an API token never exceeds [`API_TOKEN_PERMISSIONS`].
pub fn effective_permissions(credential: &Credential, member: &[String]) -> PermissionSet {
    let set = permissions::from_stored(member);
    match credential {
        Credential::Session(_) => set,
        Credential::ApiToken { .. } => set.into_iter().filter(|p| API_TOKEN_PERMISSIONS.contains(p)).collect(),
    }
}

fn require(permissions: &PermissionSet, need: Permission, credential: &Credential) -> Result<(), ApiError> {
    if permissions.contains(&need) {
        return Ok(());
    }
    Err(match credential {
        Credential::ApiToken { .. } if !API_TOKEN_PERMISSIONS.contains(&need) => {
            ApiError::Forbidden("API tokens are read-only")
        }
        _ => ApiError::Forbidden("insufficient permissions"),
    })
}

/// A project the principal is a member of, with its permissions there.
#[derive(Debug, Clone)]
pub struct ProjectAccess {
    pub tenant_id: TenantId,
    pub project_id: ProjectId,
    pub actor: Actor,
}

/// An organisation member eligible to administer organisation-wide settings.
#[derive(Debug, Clone)]
pub struct OrganizationAccess {
    pub organization_id: TenantId,
}

/// Resolves organisation membership. Unknown and foreign organisations both
/// return 404; ordinary members cannot administer project creation.
pub async fn authorize_organization_admin(
    state: &AppState,
    user: &CurrentUser,
    organization_id: &str,
) -> Result<OrganizationAccess, ApiError> {
    user.session_id()?;
    let organization_id = TenantId::parse(organization_id).map_err(|_| ApiError::NotFound)?;
    let membership = state
        .pg
        .organisation_memberships(user.user_id)
        .await?
        .into_iter()
        .find(|m| m.organization_id == organization_id.as_str())
        .ok_or(ApiError::NotFound)?;
    if !matches!(membership.role.as_str(), "owner" | "admin") {
        return Err(ApiError::Forbidden("organisation admin access required"));
    }
    Ok(OrganizationAccess { organization_id })
}

/// Resolve a project for the user and require `need` there.
/// Unknown and foreign projects both return 404 so existence is not leaked.
pub async fn authorize_project(
    state: &AppState,
    user: &CurrentUser,
    project_id: &str,
    need: Permission,
) -> Result<ProjectAccess, ApiError> {
    let project = ProjectId::parse(project_id).map_err(|_| ApiError::NotFound)?;
    if let Credential::ApiToken { scope, .. } = &user.credential
        && scope.project_id != project
    {
        return Err(ApiError::NotFound);
    }
    let memberships = state.pg.memberships(user.user_id).await?;
    let m = memberships.iter().find(|m| m.project_id == project.as_str()).ok_or(ApiError::NotFound)?;
    let organization = state.pg.organisation_for_project(project.as_str()).await?.ok_or(ApiError::NotFound)?;
    if !has_organization_access(&state.pg.organisation_memberships(user.user_id).await?, user.user_id, &organization.id)
        || !state.pg.project_assignment_exists(user.user_id, project.as_str()).await?
    {
        return Err(ApiError::NotFound);
    }
    let permissions = effective_permissions(&user.credential, &m.permissions);
    require(&permissions, need, &user.credential)?;
    let tenant_id = TenantId::parse(m.tenant_id.clone()).map_err(|e| ApiError::Internal(e.to_string()))?;
    let actor = Actor { user_id: user.user_id, permissions, is_owner: m.is_owner() && user.is_session() };
    Ok(ProjectAccess { tenant_id, project_id: project, actor })
}

/// An authorized project/environment scope.
#[derive(Debug, Clone)]
pub struct Authorized {
    pub scope: Scope,
    pub is_demo: bool,
    pub actor: Actor,
}

/// Resolve a project/environment scope for the user and require `need` in that project.
/// Unknown and unauthorized scopes both return 404 so existence is not leaked.
pub async fn authorize_as(
    state: &AppState,
    user: &CurrentUser,
    project_id: &str,
    environment_id: &str,
    need: Permission,
) -> Result<Authorized, ApiError> {
    let project = ProjectId::parse(project_id).map_err(|_| ApiError::NotFound)?;
    let env = EnvironmentId::parse(environment_id).map_err(|_| ApiError::NotFound)?;
    if let Credential::ApiToken { scope, .. } = &user.credential
        && (scope.project_id != project || scope.environment_id != env)
    {
        return Err(ApiError::NotFound);
    }
    let memberships = state.pg.memberships(user.user_id).await?;
    let m = memberships.iter().find(|m| m.project_id == project.as_str()).ok_or(ApiError::NotFound)?;
    let organization = state.pg.organisation_for_project(project.as_str()).await?.ok_or(ApiError::NotFound)?;
    if !has_organization_access(&state.pg.organisation_memberships(user.user_id).await?, user.user_id, &organization.id)
        || !state.pg.project_assignment_exists(user.user_id, project.as_str()).await?
    {
        return Err(ApiError::NotFound);
    }
    let env_row = state.pg.environment(project.as_str(), env.as_str()).await?.ok_or(ApiError::NotFound)?;
    if env_row.tenant_id != m.tenant_id {
        return Err(ApiError::NotFound);
    }
    let permissions = effective_permissions(&user.credential, &m.permissions);
    require(&permissions, need, &user.credential)?;
    let tenant = TenantId::parse(m.tenant_id.clone()).map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Authorized {
        scope: Scope { tenant_id: tenant, project_id: project, environment_id: env },
        is_demo: env_row.is_demo,
        actor: Actor { user_id: user.user_id, permissions, is_owner: m.is_owner() && user.is_session() },
    })
}

/// [`authorize_as`] for routes that only need the scope and whether it is a demo environment.
pub async fn authorize(
    state: &AppState,
    user: &CurrentUser,
    project_id: &str,
    environment_id: &str,
    need: Permission,
) -> Result<(Scope, bool), ApiError> {
    let a = authorize_as(state, user, project_id, environment_id, need).await?;
    Ok((a.scope, a.is_demo))
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

    fn all_stored() -> Vec<String> {
        Permission::ALL.iter().map(|p| p.as_str().to_owned()).collect()
    }

    #[test]
    fn api_tokens_are_capped_to_read_permissions_of_their_owner() {
        let token = Credential::ApiToken { token_id: Uuid::nil(), scope: scope("demo", "local-demo") };
        let owner = effective_permissions(&token, &all_stored());
        assert_eq!(owner, API_TOKEN_PERMISSIONS.into_iter().collect::<PermissionSet>());
        let telemetry_only = effective_permissions(&token, &["telemetry.view".to_owned()]);
        assert_eq!(telemetry_only, [Permission::TelemetryView].into(), "never more than the owner has");
        let session = Credential::Session(Uuid::nil());
        assert_eq!(effective_permissions(&session, &all_stored()).len(), Permission::ALL.len());
        assert!(effective_permissions(&session, &["root".to_owned(), "".to_owned()]).is_empty(), "unknown stored ids");
    }

    #[test]
    fn missing_permissions_are_forbidden_with_a_reason() {
        let token = Credential::ApiToken { token_id: Uuid::nil(), scope: scope("demo", "local-demo") };
        let session = Credential::Session(Uuid::nil());
        let viewer: PermissionSet = [Permission::TelemetryView, Permission::ActionsView].into();
        assert!(require(&viewer, Permission::TelemetryView, &session).is_ok());
        assert_eq!(forbidden(require(&viewer, Permission::TokensManage, &session)), "insufficient permissions");
        assert_eq!(forbidden(require(&viewer, Permission::TokensManage, &token)), "API tokens are read-only");
        let none = PermissionSet::new();
        assert_eq!(forbidden(require(&none, Permission::TelemetryView, &token)), "insufficient permissions");
    }

    #[test]
    fn organization_access_requires_membership_in_the_projects_organisation() {
        let user_id = Uuid::from_u128(42);
        let memberships = vec![karbird_storage::pg::OrganizationMemberRow {
            user_id,
            organization_id: "org-a".into(),
            role: "member".into(),
            created_at: time::OffsetDateTime::UNIX_EPOCH,
        }];

        assert!(has_organization_access(&memberships, user_id, "org-a"));
        assert!(!has_organization_access(&memberships, user_id, "org-b"));
        assert!(!has_organization_access(&memberships, Uuid::from_u128(7), "org-a"));
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
        let p = parts(&[("cookie", b"a=1; karbird_session=chs_abc; karbird_csrf=chc_x=y")]);
        assert_eq!(cookie_value(&p, SESSION_COOKIE), Some("chs_abc"));
        assert_eq!(cookie_value(&p, CSRF_COOKIE), Some("chc_x=y"), "values may contain =");
        assert_eq!(cookie_value(&p, "karbird"), None, "no prefix matching");
        let p = parts(&[("cookie", b"xkarbird_session=evil")]);
        assert_eq!(cookie_value(&p, SESSION_COOKIE), None);
        let p = parts(&[("cookie", b"a=1"), ("cookie", b"karbird_session=second")]);
        assert_eq!(cookie_value(&p, SESSION_COOKIE), Some("second"), "multiple Cookie headers");
        let p = parts(&[("cookie", b"karbird_session=first; karbird_session=dup")]);
        assert_eq!(cookie_value(&p, SESSION_COOKIE), Some("first"));
        assert_eq!(cookie_value(&parts(&[("cookie", b"karbird_session")]), SESSION_COOKIE), None);
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
        }
    }

    #[tokio::test]
    async fn api_tokens_are_bound_to_one_environment_without_revealing_others() {
        let s = test_state(true);
        let u = token_user("demo", "local-demo");
        for (p, e) in [("demo", "other-env"), ("other", "local-demo"), ("isolation", "isolation-test")] {
            assert!(
                matches!(authorize(&s, &u, p, e, Permission::TelemetryView).await, Err(ApiError::NotFound)),
                "{p}/{e} must be 404 (not 403) for a token of another scope"
            );
            assert!(
                matches!(authorize_project(&s, &u, p, Permission::MembersView).await, Err(ApiError::NotFound))
                    || p == "demo",
                "{p}: another project is 404 for a token"
            );
        }
    }

    #[tokio::test]
    async fn malformed_scope_ids_are_404_before_any_lookup() {
        let s = test_state(true);
        let session =
            CurrentUser { credential: Credential::Session(Uuid::nil()), user_id: Uuid::nil(), email: String::new() };
        for (p, e) in [("", "env"), ("demo", ""), ("de mo", "env"), ("demo", "env/../x"), ("demo", "e'; --")] {
            assert!(
                matches!(authorize(&s, &session, p, e, Permission::TelemetryView).await, Err(ApiError::NotFound)),
                "{p:?}/{e:?}"
            );
        }
        for p in ["", "de mo", "a/../b"] {
            assert!(matches!(
                authorize_project(&s, &session, p, Permission::MembersView).await,
                Err(ApiError::NotFound)
            ));
        }
        // Well-formed IDs reach storage, which is down here: that must be an error, never access.
        assert!(matches!(
            authorize(&s, &session, "demo", "local-demo", Permission::TelemetryView).await,
            Err(ApiError::Storage(_))
        ));
        assert!(matches!(
            authorize_project(&s, &session, "demo", Permission::MembersView).await,
            Err(ApiError::Storage(_))
        ));
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
