use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use karbird_storage::pg::{
    InviteProjectAssignment, NewOrganizationInvite, OrganizationInviteRow, OrganizationPersonRow,
};
use karbird_storage::secrets::{generate_token, hash_token, token_prefix};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::auth::{CurrentUser, authorize_organization_admin};
use crate::error::{ApiError, ApiResult};
use crate::mail::InviteEmail;
use crate::routes::signup::normalise_email;
use crate::state::AppState;

const ORGANIZATION_INVITE_PREFIX: &str = "koi_";

#[derive(Debug, Serialize, ToSchema)]
pub struct ProjectAssignmentDto {
    pub project_id: String,
    pub role_id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OrganizationMemberDto {
    pub user_id: String,
    pub email: String,
    pub display_name: String,
    pub organization_id: String,
    pub role: String,
    pub assignments: Vec<ProjectAssignmentDto>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AssignmentRequest {
    pub project_id: String,
    pub role_id: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AddOrganizationMemberRequest {
    pub email: String,
    pub organization_role: String,
    #[serde(default)]
    pub assignments: Vec<AssignmentRequest>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateProjectAssignmentRequest {
    pub role_id: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateOrganizationInviteRequest {
    pub email: String,
    pub organization_role: String,
    #[serde(default)]
    pub assignments: Vec<AssignmentRequest>,
    pub expires_in_days: Option<u32>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OrganizationInviteDto {
    pub id: String,
    pub email: String,
    pub organization_role: String,
    pub assignments: Vec<ProjectAssignmentDto>,
    pub prefix: String,
    pub expires_at: String,
    pub status: String,
    pub delivery_error: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreatedOrganizationInvite {
    pub invite: OrganizationInviteDto,
    /// Returned once so local installations without email can still deliver the link safely.
    pub secret: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AcceptOrganizationInviteRequest {
    pub token: String,
}

fn parse_uuid(value: &str) -> ApiResult<Uuid> {
    Uuid::parse_str(value).map_err(|_| ApiError::NotFound)
}

fn organization_role(value: &str) -> ApiResult<&str> {
    match value {
        "admin" | "member" => Ok(value),
        _ => Err(ApiError::BadRequest("organization_role must be admin or member".into())),
    }
}

fn assignments(requests: &[AssignmentRequest]) -> ApiResult<Vec<InviteProjectAssignment>> {
    requests
        .iter()
        .map(|assignment| {
            Ok(InviteProjectAssignment {
                project_id: assignment.project_id.clone(),
                role_id: parse_uuid(&assignment.role_id)?,
            })
        })
        .collect()
}

fn invite_dto(row: OrganizationInviteRow) -> OrganizationInviteDto {
    let assignments: Vec<InviteProjectAssignment> = serde_json::from_value(row.project_assignments).unwrap_or_default();
    let status = if row.accepted_at.is_some() {
        "accepted"
    } else if row.revoked_at.is_some() {
        "revoked"
    } else if row.expires_at <= OffsetDateTime::now_utc() {
        "expired"
    } else {
        row.delivery_status.as_str()
    };
    OrganizationInviteDto {
        id: row.id.to_string(),
        email: row.email,
        organization_role: row.organization_role,
        assignments: assignments
            .into_iter()
            .map(|assignment| ProjectAssignmentDto {
                project_id: assignment.project_id,
                role_id: Some(assignment.role_id.to_string()),
            })
            .collect(),
        prefix: row.token_prefix,
        expires_at: row.expires_at.format(&time::format_description::well_known::Rfc3339).unwrap_or_default(),
        status: status.to_owned(),
        delivery_error: row.delivery_error,
    }
}

async fn validate_assignments(
    state: &AppState,
    organization_id: &str,
    assignments: &[InviteProjectAssignment],
) -> ApiResult<()> {
    for assignment in assignments {
        let organization = state
            .pg
            .organisation_for_project(&assignment.project_id)
            .await?
            .filter(|organization| organization.id == organization_id)
            .ok_or(ApiError::NotFound)?;
        let _ = organization;
        state.pg.role(&assignment.project_id, assignment.role_id).await?.ok_or(ApiError::NotFound)?;
    }
    Ok(())
}

async fn member_dto(state: &AppState, row: OrganizationPersonRow) -> ApiResult<OrganizationMemberDto> {
    let assignments = state
        .pg
        .organization_project_assignments(&row.organization_id, row.user_id)
        .await?
        .into_iter()
        .map(|assignment| ProjectAssignmentDto {
            project_id: assignment.project_id,
            role_id: assignment.role_id.map(|id| id.to_string()),
        })
        .collect();
    Ok(OrganizationMemberDto {
        user_id: row.user_id.to_string(),
        email: row.email,
        display_name: row.display_name,
        organization_id: row.organization_id,
        role: row.role,
        assignments,
    })
}

#[utoipa::path(get, path = "/api/v1/organizations/{organization_id}/members", tag = "organizations", operation_id = "list_organization_members",
    params(("organization_id" = String, Path)), responses((status = 200, body = Vec<OrganizationMemberDto>)))]
pub async fn list_members(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(organization_id): Path<String>,
) -> ApiResult<Json<Vec<OrganizationMemberDto>>> {
    let access = authorize_organization_admin(&state, &user, &organization_id).await?;
    let mut result = Vec::new();
    for member in state.pg.organization_members(access.organization_id.as_str()).await? {
        result.push(member_dto(&state, member).await?);
    }
    Ok(Json(result))
}

#[utoipa::path(post, path = "/api/v1/organizations/{organization_id}/members", tag = "organizations", operation_id = "add_organization_member",
    params(("organization_id" = String, Path)), request_body = AddOrganizationMemberRequest,
    responses((status = 201, body = OrganizationMemberDto), (status = 404, body = crate::error::ErrorBody)))]
pub async fn add_member(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(organization_id): Path<String>,
    Json(request): Json<AddOrganizationMemberRequest>,
) -> ApiResult<(StatusCode, Json<OrganizationMemberDto>)> {
    let access = authorize_organization_admin(&state, &user, &organization_id).await?;
    let email = normalise_email(&request.email)?;
    let role = organization_role(&request.organization_role)?;
    let target = state.pg.user_id_by_email(&email).await?.ok_or(ApiError::NotFound)?;
    let assignments = assignments(&request.assignments)?;
    validate_assignments(&state, access.organization_id.as_str(), &assignments).await?;
    state.pg.upsert_organization_member(access.organization_id.as_str(), target, role).await?;
    for assignment in assignments {
        if !state
            .pg
            .assign_project_role(access.organization_id.as_str(), &assignment.project_id, target, assignment.role_id)
            .await?
        {
            return Err(ApiError::NotFound);
        }
    }
    let row = state
        .pg
        .organization_members(access.organization_id.as_str())
        .await?
        .into_iter()
        .find(|member| member.user_id == target)
        .ok_or(ApiError::NotFound)?;
    Ok((StatusCode::CREATED, Json(member_dto(&state, row).await?)))
}

#[utoipa::path(put, path = "/api/v1/organizations/{organization_id}/projects/{project_id}/members/{user_id}", tag = "organizations", operation_id = "put_organization_project_assignment",
    params(("organization_id" = String, Path), ("project_id" = String, Path), ("user_id" = String, Path)),
    request_body = UpdateProjectAssignmentRequest, responses((status = 204), (status = 404, body = crate::error::ErrorBody)))]
pub async fn put_project_assignment(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((organization_id, project_id, user_id)): Path<(String, String, String)>,
    Json(request): Json<UpdateProjectAssignmentRequest>,
) -> ApiResult<StatusCode> {
    let access = authorize_organization_admin(&state, &user, &organization_id).await?;
    let target = parse_uuid(&user_id)?;
    if !state
        .pg
        .assign_project_role(access.organization_id.as_str(), &project_id, target, parse_uuid(&request.role_id)?)
        .await?
    {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(delete, path = "/api/v1/organizations/{organization_id}/projects/{project_id}/members/{user_id}", tag = "organizations", operation_id = "delete_organization_project_assignment",
    params(("organization_id" = String, Path), ("project_id" = String, Path), ("user_id" = String, Path)),
    responses((status = 204), (status = 404, body = crate::error::ErrorBody)))]
pub async fn delete_project_assignment(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((organization_id, project_id, user_id)): Path<(String, String, String)>,
) -> ApiResult<StatusCode> {
    let access = authorize_organization_admin(&state, &user, &organization_id).await?;
    let project = state
        .pg
        .organisation_for_project(&project_id)
        .await?
        .filter(|organization| organization.id == access.organization_id.as_str())
        .ok_or(ApiError::NotFound)?;
    let _ = project;
    match state.pg.remove_member(&project_id, parse_uuid(&user_id)?).await? {
        karbird_storage::pg::MemberChange::Done => Ok(StatusCode::NO_CONTENT),
        karbird_storage::pg::MemberChange::NotFound => Err(ApiError::NotFound),
        karbird_storage::pg::MemberChange::LastOwner => Err(ApiError::Conflict("a project needs at least one owner")),
    }
}

#[utoipa::path(get, path = "/api/v1/organizations/{organization_id}/invites", tag = "organizations", operation_id = "list_organization_invites",
    params(("organization_id" = String, Path)), responses((status = 200, body = Vec<OrganizationInviteDto>)))]
pub async fn list_invites(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(organization_id): Path<String>,
) -> ApiResult<Json<Vec<OrganizationInviteDto>>> {
    let access = authorize_organization_admin(&state, &user, &organization_id).await?;
    Ok(Json(
        state.pg.organization_invites(access.organization_id.as_str()).await?.into_iter().map(invite_dto).collect(),
    ))
}

#[utoipa::path(post, path = "/api/v1/organizations/{organization_id}/invites", tag = "organizations", operation_id = "create_organization_invite",
    params(("organization_id" = String, Path)), request_body = CreateOrganizationInviteRequest,
    responses((status = 201, body = CreatedOrganizationInvite), (status = 400, body = crate::error::ErrorBody)))]
pub async fn create_invite(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(organization_id): Path<String>,
    Json(request): Json<CreateOrganizationInviteRequest>,
) -> ApiResult<(StatusCode, Json<CreatedOrganizationInvite>)> {
    let access = authorize_organization_admin(&state, &user, &organization_id).await?;
    let email = normalise_email(&request.email)?;
    let role = organization_role(&request.organization_role)?;
    let assignments = assignments(&request.assignments)?;
    validate_assignments(&state, access.organization_id.as_str(), &assignments).await?;
    let days = request.expires_in_days.unwrap_or(7);
    if !(1..=30).contains(&days) {
        return Err(ApiError::BadRequest("expires_in_days must be 1-30".into()));
    }
    let secret = generate_token(ORGANIZATION_INVITE_PREFIX)?;
    let invite_id = state
        .pg
        .create_organization_invite(NewOrganizationInvite {
            organization_id: access.organization_id.as_str(),
            email: &email,
            organization_role: role,
            assignments: &assignments,
            token_hash: &hash_token(&secret),
            token_prefix: &token_prefix(&secret),
            created_by: user.user_id,
            expires_at: OffsetDateTime::now_utc() + time::Duration::days(i64::from(days)),
        })
        .await?
        .ok_or(ApiError::NotFound)?;
    let organization = state
        .pg
        .organisations_for_user(user.user_id)
        .await?
        .into_iter()
        .find(|organization| organization.id == access.organization_id.as_str())
        .ok_or(ApiError::NotFound)?;
    let delivery = state
        .mailer
        .send_invite(InviteEmail {
            recipient: email,
            organization: organization.name,
            inviter: user.email.clone(),
            accept_url: format!("{}/app/invite#{}", state.public_url, secret),
        })
        .await;
    let (status, error) = match delivery {
        Ok(()) => ("sent", None),
        Err(error) => ("failed", Some(error.to_string())),
    };
    state.pg.set_organization_invite_delivery(invite_id, status, error.as_deref()).await?;
    let row = state
        .pg
        .organization_invites(access.organization_id.as_str())
        .await?
        .into_iter()
        .find(|invite| invite.id == invite_id)
        .ok_or(ApiError::NotFound)?;
    Ok((StatusCode::CREATED, Json(CreatedOrganizationInvite { invite: invite_dto(row), secret })))
}

#[utoipa::path(delete, path = "/api/v1/organizations/{organization_id}/invites/{invite_id}", tag = "organizations", operation_id = "revoke_organization_invite",
    params(("organization_id" = String, Path), ("invite_id" = String, Path)), responses((status = 204)))]
pub async fn revoke_invite(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((organization_id, invite_id)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    let access = authorize_organization_admin(&state, &user, &organization_id).await?;
    if !state.pg.revoke_organization_invite(access.organization_id.as_str(), parse_uuid(&invite_id)?).await? {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(post, path = "/api/v1/organization-invites/accept", tag = "organizations", operation_id = "accept_organization_invite",
    request_body = AcceptOrganizationInviteRequest, responses((status = 204), (status = 403, body = crate::error::ErrorBody),
    (status = 410, body = crate::error::ErrorBody)))]
pub async fn accept_invite(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(request): Json<AcceptOrganizationInviteRequest>,
) -> ApiResult<StatusCode> {
    user.session_id()?;
    let token = request.token.trim();
    if !token.starts_with(ORGANIZATION_INVITE_PREFIX) || token.len() > 128 {
        return Err(ApiError::NotFound);
    }
    let invite = state.pg.find_organization_invite(&hash_token(token)).await?.ok_or(ApiError::NotFound)?;
    if invite.email != user.email {
        return Err(ApiError::Forbidden("this invitation belongs to another email address"));
    }
    let assignments: Vec<InviteProjectAssignment> =
        serde_json::from_value(invite.project_assignments).map_err(|_| ApiError::Internal("invalid invite".into()))?;
    if !state
        .pg
        .accept_organization_invite(
            invite.id,
            user.user_id,
            &invite.organization_id,
            &invite.organization_role,
            &assignments,
        )
        .await?
    {
        return Err(ApiError::Gone("this invitation has expired, was revoked or was already accepted"));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn organization_invites_accept_only_bounded_roles_and_assignments() {
        assert_eq!(organization_role("member").unwrap(), "member");
        assert!(organization_role("owner").is_err());
        assert!(assignments(&[AssignmentRequest { project_id: "p-a".into(), role_id: "bad".into() }]).is_err());
    }
}
