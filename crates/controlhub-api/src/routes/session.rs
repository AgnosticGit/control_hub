use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use controlhub_storage::pg::AccountRow;
use controlhub_storage::secrets::{generate_token, hash_token, verify_password};
use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;
use utoipa::ToSchema;

use crate::auth::{CSRF_COOKIE, Credential, CurrentUser, SESSION_COOKIE, check_origin, effective_permissions};
use crate::error::{ApiError, ApiResult};
use crate::permissions;
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
    /// RFC 3339.
    pub created_at: String,
    /// Onboarding finished or skipped; it is offered, never required.
    pub onboarding_done: bool,
}

impl From<AccountRow> for UserDto {
    fn from(a: AccountRow) -> Self {
        Self {
            id: a.id.to_string(),
            email: a.email,
            display_name: a.display_name,
            created_at: a.created_at.format(&Rfc3339).unwrap_or_default(),
            onboarding_done: a.onboarding_completed_at.is_some(),
        }
    }
}

pub(crate) async fn user_dto(state: &AppState, user_id: uuid::Uuid) -> ApiResult<UserDto> {
    Ok(state.pg.account(user_id).await?.ok_or(ApiError::Unauthenticated)?.into())
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
    /// Built-in role key (`owner`, `admin`, `operator`, `viewer`), `custom` for a role template or
    /// `individual` for an individual permission set. API tokens always report `viewer`.
    pub role: String,
    /// Display name of the role (template name, or `Individual`).
    pub role_name: String,
    /// Effective permission ids in this project (see `GET /api/v1/permissions`).
    pub permissions: Vec<String>,
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
    }
    let ids: Vec<String> = memberships.iter().map(|m| m.project_id.clone()).collect();
    let envs = state.pg.environments_for_projects(&ids).await?;
    Ok(memberships
        .into_iter()
        .map(|m| ProjectDto {
            permissions: permissions::to_strings(&effective_permissions(&user.credential, &m.permissions)),
            role: match (token_scope, &m.role_key, m.role_id) {
                (Some(_), _, _) => "viewer".to_owned(),
                (None, Some(key), _) => key.clone(),
                (None, None, Some(_)) => "custom".to_owned(),
                (None, None, None) => "individual".to_owned(),
            },
            role_name: if token_scope.is_some() { "API token".to_owned() } else { m.role_name },
            environments: envs
                .iter()
                .filter(|e| e.project_id == m.project_id && e.tenant_id == m.tenant_id)
                .filter(|e| token_scope.is_none_or(|s| e.id == s.environment_id.as_str()))
                .map(|e| EnvironmentDto { id: e.id.clone(), name: e.name.clone(), is_demo: e.is_demo })
                .collect(),
            id: m.project_id,
            tenant_id: m.tenant_id,
            name: m.project_name,
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
    let (headers, body) = start_session(&state, user.id, user.email).await?;
    Ok((headers, Json(body)).into_response())
}

/// Create a browser session for an authenticated user: stores only token hashes, audits
/// `session.login`, and returns the session + CSRF cookies with the session payload.
pub(crate) async fn start_session(
    state: &AppState,
    user_id: uuid::Uuid,
    email: String,
) -> ApiResult<(HeaderMap, SessionResponse)> {
    let session_token = generate_token("chs_")?;
    let csrf_token = generate_token("chc_")?;
    let session_id = state
        .pg
        .create_session(user_id, &hash_token(&session_token), &hash_token(&csrf_token), state.config.session_ttl)
        .await?;
    let _ = state
        .pg
        .audit(controlhub_storage::pg::NewAuditEntry {
            tenant_id: None,
            project_id: None,
            actor_id: &user_id.to_string(),
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

    let current = CurrentUser { credential: crate::auth::Credential::Session(session_id), user_id, email };
    let body =
        SessionResponse { projects: projects_for(state, &current).await?, user: user_dto(state, user_id).await? };
    let max_age = state.config.session_ttl.whole_seconds();
    let mut headers = HeaderMap::new();
    headers.append(
        header::SET_COOKIE,
        cookie(SESSION_COOKIE, &session_token, true, state.config.secure_cookies, max_age)?,
    );
    // Readable by JS on purpose (double-submit); useless without the HttpOnly session cookie.
    headers.append(header::SET_COOKIE, cookie(CSRF_COOKIE, &csrf_token, false, state.config.secure_cookies, max_age)?);
    Ok((headers, body))
}

#[utoipa::path(get, path = "/api/v1/session", tag = "session",
    responses((status = 200, body = SessionResponse), (status = 401, body = crate::error::ErrorBody)))]
pub async fn session(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<SessionResponse>> {
    Ok(Json(SessionResponse {
        projects: projects_for(&state, &user).await?,
        user: user_dto(&state, user.user_id).await?,
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn attrs(v: &HeaderValue) -> Vec<String> {
        v.to_str().unwrap().split(';').map(|s| s.trim().to_owned()).collect()
    }

    #[test]
    fn session_cookie_is_http_only_strict_and_secure_outside_dev() {
        let a = attrs(&cookie(SESSION_COOKIE, "chs_x", true, true, 43_200).unwrap());
        assert_eq!(a[0], "controlhub_session=chs_x");
        for want in ["Path=/", "SameSite=Strict", "Max-Age=43200", "HttpOnly", "Secure"] {
            assert!(a.iter().any(|x| x == want), "missing {want}: {a:?}");
        }
        assert!(!a.iter().any(|x| x.starts_with("Domain")), "host-only cookie");
    }

    #[test]
    fn csrf_cookie_is_readable_by_js_but_still_strict() {
        let a = attrs(&cookie(CSRF_COOKIE, "chc_x", false, true, 60).unwrap());
        assert!(!a.iter().any(|x| x == "HttpOnly"), "double-submit needs JS access");
        assert!(a.iter().any(|x| x == "SameSite=Strict") && a.iter().any(|x| x == "Secure"));
    }

    #[test]
    fn dev_mode_only_drops_secure() {
        let a = attrs(&cookie(SESSION_COOKIE, "chs_x", true, false, 60).unwrap());
        assert!(!a.iter().any(|x| x == "Secure"));
        assert!(a.iter().any(|x| x == "HttpOnly") && a.iter().any(|x| x == "SameSite=Strict"));
    }

    #[test]
    fn logout_cookies_expire_immediately() {
        let a = attrs(&cookie(SESSION_COOKIE, "", true, true, 0).unwrap());
        assert_eq!(a[0], "controlhub_session=");
        assert!(a.iter().any(|x| x == "Max-Age=0"));
    }

    #[test]
    fn cookie_values_cannot_inject_attributes_or_headers() {
        assert!(cookie(SESSION_COOKIE, "x\r\nSet-Cookie: evil=1", true, true, 1).is_err());
        // Generated tokens are base64url, so they never contain `;`; the header stays one cookie.
        let t = controlhub_storage::secrets::generate_token("chs_").unwrap();
        assert_eq!(attrs(&cookie(SESSION_COOKIE, &t, true, true, 1).unwrap())[0], format!("{SESSION_COOKIE}={t}"));
    }

    #[test]
    fn dummy_hash_never_verifies() {
        for pw in ["", "password", "admin", "x"] {
            assert!(!verify_password(pw, DUMMY_HASH), "{pw:?}");
        }
    }

    /// Unknown users are checked against the dummy hash so both paths cost the same. That only
    /// holds if it parses (otherwise verification returns instantly) and uses the real parameters.
    #[test]
    fn dummy_hash_costs_the_same_as_a_real_hash() {
        use argon2::password_hash::PasswordHash;
        let dummy = PasswordHash::new(DUMMY_HASH).unwrap();
        let real_s = controlhub_storage::secrets::hash_password("x").unwrap();
        let real = PasswordHash::new(&real_s).unwrap();
        assert_eq!(dummy.algorithm, real.algorithm);
        assert_eq!(dummy.version, real.version);
        assert_eq!(dummy.params, real.params);
        assert_eq!(dummy.hash.map(|h| h.len()), real.hash.map(|h| h.len()));
    }
}
