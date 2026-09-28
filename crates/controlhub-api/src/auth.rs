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

#[derive(Debug, Clone)]
pub struct CurrentUser {
    pub session_id: Uuid,
    pub user_id: Uuid,
    pub email: String,
    pub display_name: String,
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

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let token = cookie_value(parts, SESSION_COOKIE).ok_or(ApiError::Unauthenticated)?;
        let session = state.pg.find_session(&hash_token(token)).await?.ok_or(ApiError::Unauthenticated)?;

        if is_mutation(&parts.method) {
            check_origin(state, &parts.headers)?;
            let header_token = parts
                .headers
                .get(CSRF_HEADER)
                .and_then(|v| v.to_str().ok())
                .ok_or(ApiError::Forbidden("missing CSRF token"))?;
            if !constant_time_eq(&hash_token(header_token), &session.csrf_token_hash) {
                return Err(ApiError::Forbidden("invalid CSRF token"));
            }
        }
        Ok(CurrentUser {
            session_id: session.session_id,
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
