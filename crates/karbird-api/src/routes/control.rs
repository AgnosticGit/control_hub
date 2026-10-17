//! Enrollment and outbound command polling for a single Docker container.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, header};
use karbird_storage::control::{ControlCommand, ControlInstallation};
use karbird_storage::pg::NewAuditEntry;
use karbird_storage::secrets::{generate_token, hash_token};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::ToSchema;

use crate::auth::{CurrentUser, authorize};
use crate::error::{ApiError, ApiResult};
use crate::permissions::Permission;
use crate::routes::{parse_id, rfc3339};
use crate::state::AppState;

const CODE_PREFIX: &str = "chce_";
const CREDENTIAL_PREFIX: &str = "chcc_";

#[derive(Debug, Serialize, ToSchema)]
pub struct InstallationDto {
    pub id: String,
    pub name: String,
    pub container_id: Option<String>,
    pub container_name: Option<String>,
    pub reported_container_id: Option<String>,
    pub reported_container_name: Option<String>,
    pub reported_state: Option<String>,
    pub confirmed_at: Option<String>,
    pub last_seen_at: Option<String>,
    pub revoked_at: Option<String>,
}

impl From<ControlInstallation> for InstallationDto {
    fn from(row: ControlInstallation) -> Self {
        Self {
            id: row.id.to_string(),
            name: row.name,
            container_id: row.container_id,
            container_name: row.container_name,
            reported_container_id: row.reported_container_id,
            reported_container_name: row.reported_container_name,
            reported_state: row.reported_state,
            confirmed_at: row.confirmed_at.map(rfc3339),
            last_seen_at: row.last_seen_at.map(rfc3339),
            revoked_at: row.revoked_at.map(rfc3339),
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateInstallationRequest {
    pub name: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreatedInstallation {
    pub installation: InstallationDto,
    /// One-time code. Store in a local secret file; it expires in 15 minutes.
    pub enrollment_code: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ConfirmContainerRequest {
    pub container_id: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct EnrollRequest {
    pub enrollment_code: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EnrollResponse {
    pub installation_id: String,
    /// Shown only on enrollment and kept in a local secret file.
    pub credential: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ReportRequest {
    pub container_id: String,
    pub container_name: String,
    pub state: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CommandDto {
    pub id: String,
    pub verb: String,
    pub container_id: String,
    pub expires_at: String,
}

impl From<ControlCommand> for CommandDto {
    fn from(row: ControlCommand) -> Self {
        Self {
            id: row.id.to_string(),
            verb: row.verb,
            container_id: row.container_id,
            expires_at: rfc3339(row.expires_at),
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CompleteRequest {
    pub succeeded: bool,
    pub observed_state: String,
    pub previous_started_at: Option<String>,
    pub started_at: Option<String>,
    pub detail: Option<String>,
}

fn valid_container_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

async fn audit(state: &AppState, user: &CurrentUser, row: &ControlInstallation, action: &str) -> ApiResult<()> {
    state
        .pg
        .audit(NewAuditEntry {
            tenant_id: Some(&row.tenant_id),
            project_id: Some(&row.project_id),
            actor_id: &user.user_id.to_string(),
            actor_kind: "user",
            action,
            target: Some(&row.id.to_string()),
            reason: None,
            before: None,
            after: Some(json!({ "installation": row.id, "container_id": row.container_id })),
            outcome: "succeeded",
            request_id: None,
        })
        .await?;
    Ok(())
}

async fn control_identity(state: &AppState, headers: &HeaderMap) -> ApiResult<ControlInstallation> {
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or(ApiError::Unauthenticated)?;
    if !bearer.starts_with(CREDENTIAL_PREFIX) {
        return Err(ApiError::Unauthenticated);
    }
    state.pg.authenticate_control(&hash_token(bearer)).await?.ok_or(ApiError::Unauthenticated)
}

#[utoipa::path(get, path = "/api/v1/projects/{project_id}/environments/{environment_id}/control/installations", tag = "control", operation_id = "control_list",
    params(("project_id" = String, Path), ("environment_id" = String, Path)),
    responses((status = 200, body = Vec<InstallationDto>), (status = 403, body = crate::error::ErrorBody)))]
pub async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id)): Path<(String, String)>,
) -> ApiResult<Json<Vec<InstallationDto>>> {
    let (scope, _) = authorize(&state, &user, &project_id, &environment_id, Permission::ProjectManage).await?;
    Ok(Json(state.pg.list_control_installations(&scope).await?.into_iter().map(Into::into).collect()))
}

#[utoipa::path(post, path = "/api/v1/projects/{project_id}/environments/{environment_id}/control/installations", tag = "control", operation_id = "control_create",
    params(("project_id" = String, Path), ("environment_id" = String, Path)),
    request_body = CreateInstallationRequest,
    responses((status = 200, body = CreatedInstallation), (status = 403, body = crate::error::ErrorBody)))]
pub async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id)): Path<(String, String)>,
    Json(body): Json<CreateInstallationRequest>,
) -> ApiResult<Json<CreatedInstallation>> {
    let (scope, _) = authorize(&state, &user, &project_id, &environment_id, Permission::ProjectManage).await?;
    let name = body.name.trim();
    if name.is_empty() || name.len() > 64 || name.chars().any(char::is_control) {
        return Err(ApiError::BadRequest("name must be 1-64 printable characters".into()));
    }
    let code = generate_token(CODE_PREFIX)?;
    let row = state.pg.create_control_installation(&scope, name, &hash_token(&code)).await?;
    audit(&state, &user, &row, "control.create").await?;
    Ok(Json(CreatedInstallation { installation: row.into(), enrollment_code: code }))
}

#[utoipa::path(post, path = "/api/v1/projects/{project_id}/environments/{environment_id}/control/installations/{id}/confirm", tag = "control", operation_id = "control_confirm",
    params(("project_id" = String, Path), ("environment_id" = String, Path), ("id" = String, Path)),
    request_body = ConfirmContainerRequest,
    responses((status = 200, body = InstallationDto), (status = 403, body = crate::error::ErrorBody)))]
pub async fn confirm(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id, id)): Path<(String, String, String)>,
    Json(body): Json<ConfirmContainerRequest>,
) -> ApiResult<Json<InstallationDto>> {
    let (scope, _) = authorize(&state, &user, &project_id, &environment_id, Permission::ProjectManage).await?;
    if !valid_container_id(&body.container_id) {
        return Err(ApiError::BadRequest("invalid Docker container ID".into()));
    }
    let id = parse_id(&id)?;
    if !state.pg.confirm_control_container(&scope, id, &body.container_id).await? {
        return Err(ApiError::Conflict("container identity was not reported by this installation"));
    }
    let row = state.pg.control_installation(&scope, id).await?.ok_or(ApiError::NotFound)?;
    audit(&state, &user, &row, "control.confirm").await?;
    Ok(Json(row.into()))
}

#[utoipa::path(post, path = "/api/v1/projects/{project_id}/environments/{environment_id}/control/installations/{id}/revoke", tag = "control", operation_id = "control_revoke",
    params(("project_id" = String, Path), ("environment_id" = String, Path), ("id" = String, Path)),
    responses((status = 200, body = InstallationDto), (status = 403, body = crate::error::ErrorBody)))]
pub async fn revoke(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id, id)): Path<(String, String, String)>,
) -> ApiResult<Json<InstallationDto>> {
    let (scope, _) = authorize(&state, &user, &project_id, &environment_id, Permission::ProjectManage).await?;
    let id = parse_id(&id)?;
    if !state.pg.revoke_control_installation(&scope, id).await? {
        return Err(ApiError::NotFound);
    }
    let row = state.pg.control_installation(&scope, id).await?.ok_or(ApiError::NotFound)?;
    audit(&state, &user, &row, "control.revoke").await?;
    Ok(Json(row.into()))
}

#[utoipa::path(post, path = "/api/v1/control/enroll", tag = "control", operation_id = "control_enroll", request_body = EnrollRequest,
    responses((status = 200, body = EnrollResponse), (status = 401, body = crate::error::ErrorBody)))]
pub async fn enroll(State(state): State<AppState>, Json(body): Json<EnrollRequest>) -> ApiResult<Json<EnrollResponse>> {
    if !body.enrollment_code.starts_with(CODE_PREFIX) || body.enrollment_code.len() > 100 {
        return Err(ApiError::Unauthenticated);
    }
    let credential = generate_token(CREDENTIAL_PREFIX)?;
    let row = state
        .pg
        .enroll_control(&hash_token(&body.enrollment_code), &hash_token(&credential))
        .await?
        .ok_or(ApiError::Unauthenticated)?;
    Ok(Json(EnrollResponse { installation_id: row.id.to_string(), credential }))
}

#[utoipa::path(post, path = "/api/v1/control/report", tag = "control", operation_id = "control_report", request_body = ReportRequest,
    responses((status = 200, body = Object), (status = 401, body = crate::error::ErrorBody)))]
pub async fn report(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ReportRequest>,
) -> ApiResult<Json<Value>> {
    let identity = control_identity(&state, &headers).await?;
    if !valid_container_id(&body.container_id)
        || body.container_name.len() > 128
        || !matches!(body.state.as_str(), "running" | "stopped" | "starting" | "unhealthy")
    {
        return Err(ApiError::BadRequest("invalid container report".into()));
    }
    if !state.pg.report_control_container(identity.id, &body.container_id, &body.container_name, &body.state).await? {
        return Err(ApiError::Conflict(
            "reported Docker ID differs from the confirmed container; create a new installation",
        ));
    }
    Ok(Json(json!({ "ok": true })))
}

#[utoipa::path(get, path = "/api/v1/control/commands", tag = "control", operation_id = "control_poll",
    responses((status = 200, body = Option<CommandDto>), (status = 401, body = crate::error::ErrorBody)))]
pub async fn poll(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Option<CommandDto>>> {
    let identity = control_identity(&state, &headers).await?;
    Ok(Json(state.pg.poll_control_command(identity.id).await?.map(Into::into)))
}

#[utoipa::path(post, path = "/api/v1/control/commands/{command_id}/complete", tag = "control", operation_id = "control_complete",
    params(("command_id" = String, Path)), request_body = CompleteRequest,
    responses((status = 200, body = Object), (status = 401, body = crate::error::ErrorBody)))]
pub async fn complete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(command_id): Path<String>,
    Json(body): Json<CompleteRequest>,
) -> ApiResult<Json<Value>> {
    let identity = control_identity(&state, &headers).await?;
    if !matches!(body.observed_state.as_str(), "running" | "stopped" | "unknown")
        || body.detail.as_ref().is_some_and(|v| v.len() > 500 || v.chars().any(char::is_control))
        || body.previous_started_at.as_ref().is_some_and(|v| v.len() > 64)
        || body.started_at.as_ref().is_some_and(|v| v.len() > 64)
    {
        return Err(ApiError::BadRequest("invalid Control result".into()));
    }
    let id = parse_id(&command_id)?;
    let result = json!({
        "succeeded": body.succeeded,
        "observed_state": body.observed_state,
        "previous_started_at": body.previous_started_at,
        "started_at": body.started_at,
        "detail": body.detail,
    });
    if !state.pg.complete_control_command(identity.id, id, &result).await? {
        return Err(ApiError::Conflict("command not active"));
    }
    Ok(Json(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::valid_container_id;

    #[test]
    fn only_full_docker_ids_can_be_confirmed() {
        assert!(valid_container_id(&"a".repeat(64)));
        for invalid in ["abc", "container-name", "--privileged", &"z".repeat(64)] {
            assert!(!valid_container_id(invalid));
        }
    }
}
