//! The signed-in user's own account: profile, password, sessions, onboarding state, and the
//! projects they create or leave. Every mutation needs a browser session.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use controlhub_storage::pg::{MemberChange, NewAuditEntry, SessionInfoRow};
use controlhub_storage::secrets::{hash_password, verify_password};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::format_description::well_known::Rfc3339;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::auth::{CurrentUser, authorize_project};
use crate::error::{ApiError, ApiResult};
use crate::permissions::Permission;
use crate::routes::session::{ProjectDto, UserDto, projects_for, user_dto};
use crate::routes::signup::{MAX_PASSWORD_BYTES, check_new_password, required_name};
use crate::state::AppState;

/// Projects one account may own; bounds what a single sign-up can create.
pub const MAX_OWNED_PROJECTS: i64 = 20;
const NEW_PROJECT_ENVIRONMENT: &str = "production";

#[derive(Debug, Deserialize, ToSchema)]
pub struct AccountUpdateRequest {
    pub display_name: Option<String>,
    /// `true` finishes (or skips) onboarding, `false` offers it again.
    pub onboarding_done: Option<bool>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PasswordChangeRequest {
    pub current_password: String,
    /// Same rules as at sign-up (at least 12 characters, not the email).
    pub new_password: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SessionDto {
    pub id: String,
    pub created_at: String,
    pub expires_at: String,
    /// The session this request was made with.
    pub current: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RevokedSessions {
    pub revoked: u64,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ProjectRequest {
    pub name: String,
}

fn session_dto(s: SessionInfoRow, current: Uuid) -> SessionDto {
    SessionDto {
        id: s.id.to_string(),
        created_at: s.created_at.format(&Rfc3339).unwrap_or_default(),
        expires_at: s.expires_at.format(&Rfc3339).unwrap_or_default(),
        current: s.id == current,
    }
}

async fn audit(
    state: &AppState,
    scope: Option<(&str, &str)>,
    user: &CurrentUser,
    action: &str,
    target: &str,
    after: Option<Value>,
) -> ApiResult<()> {
    state
        .pg
        .audit(NewAuditEntry {
            tenant_id: scope.map(|s| s.0),
            project_id: scope.map(|s| s.1),
            actor_id: &user.user_id.to_string(),
            actor_kind: "user",
            action,
            target: Some(target),
            reason: None,
            before: None,
            after,
            outcome: "succeeded",
            request_id: None,
        })
        .await?;
    Ok(())
}

#[utoipa::path(patch, path = "/api/v1/account", tag = "account", operation_id = "update_account", request_body = AccountUpdateRequest,
    responses((status = 200, body = UserDto), (status = 400, body = crate::error::ErrorBody), (status = 401, body = crate::error::ErrorBody),
        (status = 403, body = crate::error::ErrorBody)))]
pub async fn update(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<AccountUpdateRequest>,
) -> ApiResult<Json<UserDto>> {
    user.session_id()?;
    if req.display_name.is_none() && req.onboarding_done.is_none() {
        return Err(ApiError::BadRequest("nothing to change".into()));
    }
    let name = req.display_name.as_deref().map(|n| required_name(n, "display name")).transpose()?;
    if let Some(name) = &name {
        state.pg.set_display_name(user.user_id, name).await?;
        audit(&state, None, &user, "user.update", &user.user_id.to_string(), Some(json!({ "display_name": name })))
            .await?;
    }
    if let Some(done) = req.onboarding_done {
        state.pg.set_onboarding_done(user.user_id, done).await?;
    }
    Ok(Json(user_dto(&state, user.user_id).await?))
}

#[utoipa::path(post, path = "/api/v1/account/password", tag = "account", request_body = PasswordChangeRequest,
    responses((status = 204, description = "Changed; all other sessions are signed out"), (status = 400, body = crate::error::ErrorBody),
        (status = 401, body = crate::error::ErrorBody), (status = 403, body = crate::error::ErrorBody), (status = 429, body = crate::error::ErrorBody)))]
pub async fn change_password(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<PasswordChangeRequest>,
) -> ApiResult<StatusCode> {
    let session_id = user.session_id()?;
    // Shares the login throttle, so the form cannot be used to guess the password faster.
    let key = user.email.clone();
    state.login_throttle.check(&key).await?;
    if req.current_password.len() > MAX_PASSWORD_BYTES {
        return Err(ApiError::BadRequest("current password is wrong".into()));
    }
    let account = state.pg.account(user.user_id).await?.ok_or(ApiError::Unauthenticated)?;
    let (current, hash) = (req.current_password.clone(), account.password_hash.clone());
    let ok = tokio::task::spawn_blocking(move || verify_password(&current, &hash))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !ok {
        state.login_throttle.failure(&key).await;
        return Err(ApiError::BadRequest("current password is wrong".into()));
    }
    state.login_throttle.success(&key).await;
    check_new_password(&account.email, &req.new_password)?;
    let new = req.new_password;
    let hash = tokio::task::spawn_blocking(move || hash_password(&new))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))??;
    let ended = state.pg.change_password(user.user_id, &hash, session_id).await?;
    audit(
        &state,
        None,
        &user,
        "user.password_change",
        &user.user_id.to_string(),
        Some(json!({ "sessions_ended": ended })),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(get, path = "/api/v1/account/sessions", tag = "account", operation_id = "list_sessions",
    responses((status = 200, body = Vec<SessionDto>), (status = 401, body = crate::error::ErrorBody), (status = 403, body = crate::error::ErrorBody)))]
pub async fn sessions(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Vec<SessionDto>>> {
    let current = user.session_id()?;
    let rows = state.pg.active_sessions(user.user_id).await?;
    Ok(Json(rows.into_iter().map(|s| session_dto(s, current)).collect()))
}

#[utoipa::path(delete, path = "/api/v1/account/sessions/{session_id}", tag = "account",
    params(("session_id" = String, Path)),
    responses((status = 204), (status = 401, body = crate::error::ErrorBody), (status = 403, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)))]
pub async fn revoke_session(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(session_id): Path<String>,
) -> ApiResult<StatusCode> {
    user.session_id()?;
    let id = Uuid::parse_str(&session_id).map_err(|_| ApiError::NotFound)?;
    if !state.pg.revoke_user_session(user.user_id, id).await? {
        return Err(ApiError::NotFound);
    }
    audit(&state, None, &user, "session.revoke", &session_id, None).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(post, path = "/api/v1/account/sessions/revoke-others", tag = "account",
    responses((status = 200, body = RevokedSessions), (status = 401, body = crate::error::ErrorBody), (status = 403, body = crate::error::ErrorBody)))]
pub async fn revoke_other_sessions(
    State(state): State<AppState>,
    user: CurrentUser,
) -> ApiResult<Json<RevokedSessions>> {
    let current = user.session_id()?;
    let revoked = state.pg.revoke_other_sessions(user.user_id, current).await?;
    audit(&state, None, &user, "session.revoke_others", &user.user_id.to_string(), Some(json!({ "revoked": revoked })))
        .await?;
    Ok(Json(RevokedSessions { revoked }))
}

#[utoipa::path(post, path = "/api/v1/projects", tag = "projects", request_body = ProjectRequest,
    responses((status = 201, body = ProjectDto), (status = 400, body = crate::error::ErrorBody), (status = 401, body = crate::error::ErrorBody),
        (status = 403, body = crate::error::ErrorBody), (status = 409, body = crate::error::ErrorBody)))]
pub async fn create_project(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<ProjectRequest>,
) -> ApiResult<(StatusCode, Json<ProjectDto>)> {
    user.session_id()?;
    let name = required_name(&req.name, "project name")?;
    if state.pg.owned_project_count(user.user_id).await? >= MAX_OWNED_PROJECTS {
        return Err(ApiError::Conflict("you already own the maximum number of projects (20)"));
    }
    let scope = state.pg.create_project(user.user_id, &name, NEW_PROJECT_ENVIRONMENT).await?;
    let (t, p) = (scope.tenant_id.as_str(), scope.project_id.as_str());
    audit(&state, Some((t, p)), &user, "project.create", p, Some(json!({ "name": name }))).await?;
    let project = projects_for(&state, &user).await?.into_iter().find(|x| x.id == p).ok_or(ApiError::NotFound)?;
    Ok((StatusCode::CREATED, Json(project)))
}

#[utoipa::path(patch, path = "/api/v1/projects/{project_id}", tag = "projects",
    params(("project_id" = String, Path)), request_body = ProjectRequest,
    responses((status = 204), (status = 400, body = crate::error::ErrorBody), (status = 403, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody), (status = 409, body = crate::error::ErrorBody)))]
pub async fn rename_project(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(project_id): Path<String>,
    Json(req): Json<ProjectRequest>,
) -> ApiResult<StatusCode> {
    let access = authorize_project(&state, &user, &project_id, Permission::ProjectManage).await?;
    user.session_id()?;
    let name = required_name(&req.name, "project name")?;
    if !state.pg.rename_project(access.project_id.as_str(), &name).await? {
        return Err(ApiError::Conflict("another project of this organisation has this name"));
    }
    let scope = Some((access.tenant_id.as_str(), access.project_id.as_str()));
    audit(&state, scope, &user, "project.rename", access.project_id.as_str(), Some(json!({ "name": name }))).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(post, path = "/api/v1/projects/{project_id}/leave", tag = "projects",
    params(("project_id" = String, Path)),
    responses((status = 204), (status = 403, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody),
        (status = 409, description = "The last owner cannot leave", body = crate::error::ErrorBody)))]
pub async fn leave_project(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(project_id): Path<String>,
) -> ApiResult<StatusCode> {
    user.session_id()?;
    let membership = state
        .pg
        .memberships(user.user_id)
        .await?
        .into_iter()
        .find(|m| m.project_id == project_id)
        .ok_or(ApiError::NotFound)?;
    match state.pg.remove_member(&membership.project_id, user.user_id).await? {
        MemberChange::Done => {}
        MemberChange::NotFound => return Err(ApiError::NotFound),
        MemberChange::LastOwner => {
            return Err(ApiError::Conflict("you are the last owner; make someone else an owner first"));
        }
    }
    let scope = Some((membership.tenant_id.as_str(), membership.project_id.as_str()));
    audit(
        &state,
        scope,
        &user,
        "member.leave",
        &user.user_id.to_string(),
        Some(json!({ "role": membership.role_name })),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn sessions_mark_only_the_current_one() {
        let at = time::OffsetDateTime::from_unix_timestamp(0).unwrap();
        let row = |n| SessionInfoRow { id: Uuid::from_u128(n), created_at: at, expires_at: at };
        let current = Uuid::from_u128(2);
        let dtos: Vec<SessionDto> = [1, 2, 3].map(|n| session_dto(row(n), current)).into();
        assert_eq!(dtos.iter().filter(|d| d.current).count(), 1);
        assert!(dtos[1].current);
        assert_eq!(dtos[0].created_at, "1970-01-01T00:00:00Z");
    }
}
