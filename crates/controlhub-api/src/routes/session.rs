use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use controlhub_storage::secrets::{generate_token, hash_token, verify_password};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::auth::{CSRF_COOKIE, Credential, CurrentUser, SESSION_COOKIE, check_origin};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// Precomputed Argon2 hash used when the user does not exist, so both paths
/// cost the same (no account enumeration by timing).
const DUMMY_HASH: &str =
    "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHRzb21lc2FsdA$Z5c3oQx0Ctq8Vn8b2c0m0jX8zY4Q6fB8Qe3Qm3b5x0A";
#[cfg(test)]
pub const DUMMY_HASH_FOR_TEST: &str = DUMMY_HASH;

#[derive(Debug, Deserialize, ToSchema)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserDto {
    pub id: String,
    pub email: String,
    pub display_name: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EnvironmentDto {
    pub id: String,
    pub name: String,
    /// Demo environments must always be labelled in the UI.
    pub is_demo: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProjectDto {
    pub id: String,
    pub tenant_id: String,
    pub name: String,
    pub role: String,
    pub environments: Vec<EnvironmentDto>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SessionResponse {
    pub user: UserDto,
    pub projects: Vec<ProjectDto>,
}

fn cookie(name: &str, value: &str, http_only: bool, secure: bool, max_age: i64) -> Result<HeaderValue, ApiError> {
    let mut c = format!("{name}={value}; Path=/; SameSite=Strict; Max-Age={max_age}");
    if http_only {
        c.push_str("; HttpOnly");
    }
    if secure {
        c.push_str("; Secure");
    }
    HeaderValue::from_str(&c).map_err(|e| ApiError::Internal(e.to_string()))
}

pub async fn projects_for(state: &AppState, user: &CurrentUser) -> ApiResult<Vec<ProjectDto>> {
    let token_scope = match &user.credential {
        Credential::ApiToken { scope, .. } => Some(scope),
        Credential::Session(_) => None,
    };
    let mut memberships = state.pg.memberships(user.user_id).await?;
    if let Some(s) = token_scope {
        memberships.retain(|m| m.project_id == s.project_id.as_str() && m.tenant_id == s.tenant_id.as_str());
        for m in &mut memberships {
            m.role = "viewer".to_owned();
        }
    }
    let ids: Vec<String> = memberships.iter().map(|m| m.project_id.clone()).collect();
    let envs = state.pg.environments_for_projects(&ids).await?;
    Ok(memberships
        .into_iter()
        .map(|m| ProjectDto {
            environments: envs
                .iter()
                .filter(|e| e.project_id == m.project_id && e.tenant_id == m.tenant_id)
                .filter(|e| token_scope.is_none_or(|s| e.id == s.environment_id.as_str()))
                .map(|e| EnvironmentDto { id: e.id.clone(), name: e.name.clone(), is_demo: e.is_demo })
                .collect(),
            id: m.project_id,
            tenant_id: m.tenant_id,
            name: m.project_name,
            role: m.role,
        })
        .collect())
}

#[utoipa::path(post, path = "/api/v1/session/login", tag = "session", request_body = LoginRequest,
    responses((status = 200, body = SessionResponse), (status = 401, body = crate::error::ErrorBody), (status = 429, body = crate::error::ErrorBody)))]
pub async fn login(
    State(state): State<AppState>,
    request_headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> ApiResult<Response> {
    check_origin(&state, &request_headers)?;
    let email = req.email.trim().to_lowercase();
    if email.is_empty() || email.len() > 254 || req.password.len() > 1024 {
        return Err(ApiError::Unauthenticated);
    }
    state.login_throttle.check(&email).await?;
    let user = state.pg.find_user_by_email(&email).await?;
    let password = req.password.clone();
    let hash = user.as_ref().map_or(DUMMY_HASH.to_owned(), |u| u.password_hash.clone());
    let ok = tokio::task::spawn_blocking(move || verify_password(&password, &hash))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let Some(user) = user.filter(|_| ok) else {
        state.login_throttle.failure(&email).await;
        return Err(ApiError::Unauthenticated);
    };
    state.login_throttle.success(&email).await;

    let session_token = generate_token("chs_")?;
    let csrf_token = generate_token("chc_")?;
    let session_id = state
        .pg
        .create_session(user.id, &hash_token(&session_token), &hash_token(&csrf_token), state.config.session_ttl)
        .await?;
    let _ = state
        .pg
        .audit(controlhub_storage::pg::NewAuditEntry {
            tenant_id: None,
            project_id: None,
            actor_id: &user.id.to_string(),
            actor_kind: "user",
            action: "session.login",
            target: Some(&session_id.to_string()),
            reason: None,
            before: None,
            after: None,
            outcome: "succeeded",
            request_id: None,
        })
        .await;

    let current = CurrentUser {
        credential: crate::auth::Credential::Session(session_id),
        user_id: user.id,
        email: user.email.clone(),
        display_name: user.display_name.clone(),
    };
    let body = SessionResponse {
        user: UserDto { id: user.id.to_string(), email: user.email, display_name: user.display_name },
        projects: projects_for(&state, &current).await?,
    };
    let max_age = state.config.session_ttl.whole_seconds();
    let mut headers = HeaderMap::new();
    headers.append(
        header::SET_COOKIE,
        cookie(SESSION_COOKIE, &session_token, true, state.config.secure_cookies, max_age)?,
    );
    // Readable by JS on purpose (double-submit); useless without the HttpOnly session cookie.
    headers.append(header::SET_COOKIE, cookie(CSRF_COOKIE, &csrf_token, false, state.config.secure_cookies, max_age)?);
    Ok((headers, Json(body)).into_response())
}

#[utoipa::path(get, path = "/api/v1/session", tag = "session",
    responses((status = 200, body = SessionResponse), (status = 401, body = crate::error::ErrorBody)))]
pub async fn session(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<SessionResponse>> {
    Ok(Json(SessionResponse {
        projects: projects_for(&state, &user).await?,
        user: UserDto { id: user.user_id.to_string(), email: user.email, display_name: user.display_name },
    }))
}

#[utoipa::path(post, path = "/api/v1/session/logout", tag = "session",
    responses((status = 204), (status = 401, body = crate::error::ErrorBody), (status = 403, body = crate::error::ErrorBody)))]
pub async fn logout(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Response> {
    state.pg.revoke_session(user.session_id()?).await?;
    let mut headers = HeaderMap::new();
    headers.append(header::SET_COOKIE, cookie(SESSION_COOKIE, "", true, state.config.secure_cookies, 0)?);
    headers.append(header::SET_COOKIE, cookie(CSRF_COOKIE, "", false, state.config.secure_cookies, 0)?);
    Ok((axum::http::StatusCode::NO_CONTENT, headers).into_response())
}

#[utoipa::path(get, path = "/api/v1/projects", tag = "projects",
    responses((status = 200, body = Vec<ProjectDto>), (status = 401, body = crate::error::ErrorBody)))]
pub async fn projects(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Vec<ProjectDto>>> {
    Ok(Json(projects_for(&state, &user).await?))
}
