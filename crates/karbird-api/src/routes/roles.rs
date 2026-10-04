//! Project role templates and members: who has which permissions in a project.
//! Every change is checked against the acting member's own permissions (no escalation) and audited.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use karbird_storage::pg::{MemberChange, MemberRow, NewAuditEntry, RoleChange, RoleRow};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::format_description::well_known::Rfc3339;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::auth::{CurrentUser, ProjectAccess, authorize_project};
use crate::error::{ApiError, ApiResult};
use crate::permissions::{
    self, Grant, Permission, PermissionInfo, PermissionSet, Target, check_member_change, check_template_change,
    from_stored, parse_set,
};
use crate::state::AppState;

#[derive(Debug, Serialize, ToSchema)]
pub struct RoleDto {
    pub id: String,
    pub name: String,
    pub permissions: Vec<String>,
    /// `owner`, `admin`, `operator` or `viewer` for built-in (read-only) roles; absent for templates.
    pub builtin: Option<String>,
    /// Members holding this role.
    pub holders: i64,
}

impl From<RoleRow> for RoleDto {
    fn from(r: RoleRow) -> Self {
        Self {
            id: r.id.to_string(),
            name: r.name,
            permissions: permissions::to_strings(&from_stored(&r.permissions)),
            builtin: r.builtin,
            holders: r.holders,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MemberDto {
    pub user_id: String,
    pub email: String,
    pub display_name: String,
    /// Built-in role key, `custom` for a role template or `individual` for an individual set.
    pub role: String,
    pub role_id: Option<String>,
    pub role_name: String,
    /// For an individual set: the role it was derived from (display only).
    pub based_on: Option<String>,
    /// Effective permission ids.
    pub permissions: Vec<String>,
    pub is_owner: bool,
    pub joined_at: String,
}

impl From<MemberRow> for MemberDto {
    fn from(m: MemberRow) -> Self {
        let is_owner = m.is_owner();
        Self {
            role: match (&m.role_key, m.role_id) {
                (Some(key), _) => key.clone(),
                (None, Some(_)) => "custom".to_owned(),
                (None, None) => "individual".to_owned(),
            },
            user_id: m.user_id.to_string(),
            email: m.email,
            display_name: m.display_name,
            role_id: m.role_id.map(|id| id.to_string()),
            role_name: m.role_name,
            based_on: m.based_on.map(|id| id.to_string()),
            permissions: permissions::to_strings(&from_stored(&m.permissions)),
            is_owner,
            joined_at: m.created_at.format(&Rfc3339).unwrap_or_default(),
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RoleRequest {
    pub name: String,
    pub permissions: Vec<String>,
}

/// Exactly one of `role_id` or `permissions`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct MemberUpdateRequest {
    pub role_id: Option<String>,
    pub permissions: Option<Vec<String>>,
    /// With `permissions`: the role the set was derived from, shown as "Individual, based on …".
    pub based_on: Option<String>,
}

pub(crate) fn parse_id(id: &str) -> ApiResult<Uuid> {
    Uuid::parse_str(id).map_err(|_| ApiError::NotFound)
}

/// Role names are shown to other members: trimmed, 1-64 characters, no control characters.
pub(crate) fn validate_role_name(name: &str) -> ApiResult<&str> {
    let name = name.trim();
    let len = name.chars().count();
    if len == 0 || len > 64 || name.chars().any(char::is_control) {
        return Err(ApiError::BadRequest("name: 1-64 characters, no control characters".into()));
    }
    if name.eq_ignore_ascii_case("individual") {
        return Err(ApiError::BadRequest("name: \"Individual\" is reserved".into()));
    }
    Ok(name)
}

fn requested_permissions(items: &[String]) -> ApiResult<PermissionSet> {
    parse_set(items).map_err(|e| ApiError::BadRequest(format!("permissions: {e}")))
}

pub(crate) fn grant_json(role_name: &str, permissions: &[String]) -> Value {
    json!({ "role": role_name, "permissions": permissions })
}

#[allow(clippy::too_many_arguments)]
async fn audit(
    state: &AppState,
    access: &ProjectAccess,
    action: &str,
    target: &str,
    reason: Option<&str>,
    before: Option<Value>,
    after: Option<Value>,
    outcome: &str,
) -> ApiResult<()> {
    state
        .pg
        .audit(NewAuditEntry {
            tenant_id: Some(access.tenant_id.as_str()),
            project_id: Some(access.project_id.as_str()),
            actor_id: &access.actor.user_id.to_string(),
            actor_kind: "user",
            action,
            target: Some(target),
            reason,
            before,
            after,
            outcome,
            request_id: None,
        })
        .await?;
    Ok(())
}

/// Refusals are audited too; if that write fails the request fails.
async fn deny(state: &AppState, access: &ProjectAccess, action: &str, target: &str, why: &'static str) -> ApiError {
    match audit(state, access, action, target, Some(why), None, None, "denied").await {
        Ok(()) => ApiError::Forbidden(why),
        Err(e) => e,
    }
}

#[utoipa::path(get, path = "/api/v1/permissions", tag = "members",
    responses((status = 200, body = Vec<PermissionInfo>), (status = 401, body = crate::error::ErrorBody)))]
pub async fn catalog(_user: CurrentUser) -> Json<Vec<PermissionInfo>> {
    Json(permissions::catalog())
}

#[utoipa::path(get, path = "/api/v1/projects/{project_id}/roles", tag = "members",
    params(("project_id" = String, Path)),
    responses((status = 200, body = Vec<RoleDto>), (status = 403, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)))]
pub async fn list_roles(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(project_id): Path<String>,
) -> ApiResult<Json<Vec<RoleDto>>> {
    let access = authorize_project(&state, &user, &project_id, Permission::MembersView).await?;
    Ok(Json(state.pg.roles(access.project_id.as_str()).await?.into_iter().map(Into::into).collect()))
}

#[utoipa::path(post, path = "/api/v1/projects/{project_id}/roles", tag = "members",
    params(("project_id" = String, Path)), request_body = RoleRequest,
    responses((status = 200, body = RoleDto), (status = 400, body = crate::error::ErrorBody), (status = 403, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody), (status = 409, body = crate::error::ErrorBody)))]
pub async fn create_role(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(project_id): Path<String>,
    Json(req): Json<RoleRequest>,
) -> ApiResult<Json<RoleDto>> {
    let access = authorize_project(&state, &user, &project_id, Permission::RolesManage).await?;
    user.session_id()?;
    let name = validate_role_name(&req.name)?;
    let wanted = requested_permissions(&req.permissions)?;
    if let Err(why) = check_template_change(&access.actor, None, &wanted) {
        return Err(deny(&state, &access, "role.create", name, why).await);
    }
    let stored = permissions::to_strings(&wanted);
    let project = access.project_id.as_str();
    let id = state
        .pg
        .create_role(project, name, &stored)
        .await?
        .ok_or(ApiError::Conflict("a role with this name already exists"))?;
    let role = state.pg.role(project, id).await?.ok_or(ApiError::NotFound)?;
    audit(&state, &access, "role.create", &id.to_string(), None, None, Some(grant_json(name, &stored)), "succeeded")
        .await?;
    Ok(Json(role.into()))
}

#[utoipa::path(patch, path = "/api/v1/projects/{project_id}/roles/{role_id}", tag = "members",
    params(("project_id" = String, Path), ("role_id" = String, Path)), request_body = RoleRequest,
    responses((status = 200, body = RoleDto), (status = 400, body = crate::error::ErrorBody), (status = 403, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody), (status = 409, body = crate::error::ErrorBody)))]
pub async fn update_role(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, role_id)): Path<(String, String)>,
    Json(req): Json<RoleRequest>,
) -> ApiResult<Json<RoleDto>> {
    let access = authorize_project(&state, &user, &project_id, Permission::RolesManage).await?;
    user.session_id()?;
    let id = parse_id(&role_id)?;
    let name = validate_role_name(&req.name)?;
    let wanted = requested_permissions(&req.permissions)?;
    let project = access.project_id.as_str();
    let old = state.pg.role(project, id).await?.ok_or(ApiError::NotFound)?;
    if old.builtin.is_some() {
        return Err(deny(&state, &access, "role.update", &role_id, "built-in roles cannot be changed").await);
    }
    let old_set = from_stored(&old.permissions);
    if let Err(why) = check_template_change(&access.actor, Some(&old_set), &wanted) {
        return Err(deny(&state, &access, "role.update", &role_id, why).await);
    }
    let stored = permissions::to_strings(&wanted);
    match state.pg.update_role(project, id, name, &stored).await? {
        RoleChange::Done => {}
        RoleChange::NameTaken => return Err(ApiError::Conflict("a role with this name already exists")),
        RoleChange::Builtin => return Err(ApiError::Forbidden("built-in roles cannot be changed")),
        RoleChange::NotFound | RoleChange::Held(_) => return Err(ApiError::NotFound),
    }
    let role = state.pg.role(project, id).await?.ok_or(ApiError::NotFound)?;
    let before = grant_json(&old.name, &permissions::to_strings(&old_set));
    audit(&state, &access, "role.update", &role_id, None, Some(before), Some(grant_json(name, &stored)), "succeeded")
        .await?;
    Ok(Json(role.into()))
}

#[utoipa::path(delete, path = "/api/v1/projects/{project_id}/roles/{role_id}", tag = "members",
    params(("project_id" = String, Path), ("role_id" = String, Path)),
    responses((status = 204), (status = 403, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody),
        (status = 409, body = crate::error::ErrorBody)))]
pub async fn delete_role(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, role_id)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    let access = authorize_project(&state, &user, &project_id, Permission::RolesManage).await?;
    user.session_id()?;
    let id = parse_id(&role_id)?;
    let project = access.project_id.as_str();
    let old = state.pg.role(project, id).await?.ok_or(ApiError::NotFound)?;
    if old.builtin.is_some() {
        return Err(deny(&state, &access, "role.delete", &role_id, "built-in roles cannot be changed").await);
    }
    let old_set = from_stored(&old.permissions);
    if let Err(why) = check_template_change(&access.actor, Some(&old_set), &PermissionSet::new()) {
        return Err(deny(&state, &access, "role.delete", &role_id, why).await);
    }
    match state.pg.delete_role(project, id).await? {
        RoleChange::Done => {}
        RoleChange::Held(_) => return Err(ApiError::Conflict("the role is still assigned to members")),
        RoleChange::Builtin => return Err(ApiError::Forbidden("built-in roles cannot be changed")),
        RoleChange::NotFound | RoleChange::NameTaken => return Err(ApiError::NotFound),
    }
    let before = grant_json(&old.name, &permissions::to_strings(&old_set));
    audit(&state, &access, "role.delete", &role_id, None, Some(before), None, "succeeded").await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(get, path = "/api/v1/projects/{project_id}/members", tag = "members",
    params(("project_id" = String, Path)),
    responses((status = 200, body = Vec<MemberDto>), (status = 403, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)))]
pub async fn list_members(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(project_id): Path<String>,
) -> ApiResult<Json<Vec<MemberDto>>> {
    let access = authorize_project(&state, &user, &project_id, Permission::MembersView).await?;
    Ok(Json(state.pg.members(access.project_id.as_str()).await?.into_iter().map(Into::into).collect()))
}

fn target_of(m: &MemberRow) -> Target {
    Target { user_id: m.user_id, is_owner: m.is_owner(), permissions: from_stored(&m.permissions) }
}

fn member_change_result(r: MemberChange) -> ApiResult<()> {
    match r {
        MemberChange::Done => Ok(()),
        MemberChange::NotFound => Err(ApiError::NotFound),
        MemberChange::LastOwner => Err(ApiError::Conflict("a project needs at least one owner")),
    }
}

#[utoipa::path(patch, path = "/api/v1/projects/{project_id}/members/{user_id}", tag = "members",
    params(("project_id" = String, Path), ("user_id" = String, Path)), request_body = MemberUpdateRequest,
    responses((status = 200, body = MemberDto), (status = 400, body = crate::error::ErrorBody), (status = 403, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody), (status = 409, body = crate::error::ErrorBody)))]
pub async fn update_member(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, user_id)): Path<(String, String)>,
    Json(req): Json<MemberUpdateRequest>,
) -> ApiResult<Json<MemberDto>> {
    let access = authorize_project(&state, &user, &project_id, Permission::MembersManage).await?;
    user.session_id()?;
    let member_id = parse_id(&user_id)?;
    let project = access.project_id.as_str();
    let (grant, role_id) = match (&req.role_id, &req.permissions) {
        (Some(role_id), None) => {
            let r = state.pg.role(project, parse_id(role_id)?).await?.ok_or(ApiError::NotFound)?;
            let owner = r.builtin.as_deref() == Some("owner");
            (Grant::Role { permissions: from_stored(&r.permissions), owner }, Some(r.id))
        }
        (None, Some(items)) => (Grant::Individual(requested_permissions(items)?), None),
        _ => return Err(ApiError::BadRequest("send exactly one of role_id or permissions".into())),
    };
    let based_on = match req.based_on.as_deref() {
        Some(id) if role_id.is_none() => Some(parse_id(id).map_err(|_| ApiError::BadRequest("based_on".into()))?),
        _ => None,
    };
    let target = state.pg.member(project, member_id).await?.ok_or(ApiError::NotFound)?;
    let owners = state.pg.count_owners(project).await?;
    if let Err(why) = check_member_change(&access.actor, &target_of(&target), &grant, owners) {
        return Err(deny(&state, &access, "member.update", &user_id, why).await);
    }
    let change = match (role_id, &grant) {
        (Some(id), _) => state.pg.set_member_role(project, member_id, id).await?,
        (None, Grant::Individual(set)) => {
            state.pg.set_member_permissions(project, member_id, &permissions::to_strings(set), based_on).await?
        }
        (None, _) => MemberChange::NotFound,
    };
    member_change_result(change)?;
    let updated = state.pg.member(project, member_id).await?.ok_or(ApiError::NotFound)?;
    let before = grant_json(&target.role_name, &permissions::to_strings(&from_stored(&target.permissions)));
    let after = grant_json(&updated.role_name, &permissions::to_strings(&from_stored(&updated.permissions)));
    audit(&state, &access, "member.update", &user_id, None, Some(before), Some(after), "succeeded").await?;
    Ok(Json(updated.into()))
}

#[utoipa::path(delete, path = "/api/v1/projects/{project_id}/members/{user_id}", tag = "members",
    params(("project_id" = String, Path), ("user_id" = String, Path)),
    responses((status = 204), (status = 403, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody),
        (status = 409, body = crate::error::ErrorBody)))]
pub async fn remove_member(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, user_id)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    let access = authorize_project(&state, &user, &project_id, Permission::MembersManage).await?;
    user.session_id()?;
    let member_id = parse_id(&user_id)?;
    let project = access.project_id.as_str();
    let target = state.pg.member(project, member_id).await?.ok_or(ApiError::NotFound)?;
    let owners = state.pg.count_owners(project).await?;
    if let Err(why) = check_member_change(&access.actor, &target_of(&target), &Grant::Remove, owners) {
        return Err(deny(&state, &access, "member.remove", &user_id, why).await);
    }
    member_change_result(state.pg.remove_member(project, member_id).await?)?;
    let before = grant_json(&target.role_name, &permissions::to_strings(&from_stored(&target.permissions)));
    audit(&state, &access, "member.remove", &user_id, None, Some(before), None, "succeeded").await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn role_names_are_trimmed_bounded_and_printable() {
        assert_eq!(validate_role_name("  Manager ").unwrap(), "Manager");
        assert_eq!(validate_role_name("Менеджер").unwrap(), "Менеджер", "non-ASCII names are fine");
        assert!(validate_role_name(&"я".repeat(64)).is_ok(), "64 characters, not bytes");
        for bad in ["", "   ", "a\nb", "tab\there", "\u{7f}", "Individual", "INDIVIDUAL"] {
            assert!(matches!(validate_role_name(bad), Err(ApiError::BadRequest(_))), "{bad:?}");
        }
        assert!(validate_role_name(&"a".repeat(65)).is_err());
    }

    #[test]
    fn requested_permissions_are_strict() {
        let ok = requested_permissions(&["tokens.manage".into()]).unwrap();
        assert!(ok.contains(&Permission::TokensView), "implied permissions are added");
        assert!(matches!(requested_permissions(&["tokens.*".into()]), Err(ApiError::BadRequest(_))));
        assert!(matches!(requested_permissions(&["owner".into()]), Err(ApiError::BadRequest(_))));
    }

    fn member(role_key: Option<&str>, role_id: Option<Uuid>, permissions: &[&str]) -> MemberRow {
        MemberRow {
            user_id: Uuid::from_u128(7),
            email: "m@example.test".into(),
            display_name: "M".into(),
            role_id,
            role_key: role_key.map(Into::into),
            role_name: "R".into(),
            based_on: None,
            permissions: permissions.iter().map(|s| (*s).to_owned()).collect(),
            created_at: time::OffsetDateTime::from_unix_timestamp(0).unwrap(),
        }
    }

    #[test]
    fn member_dtos_report_the_kind_of_role_and_hide_unknown_permissions() {
        let owner = MemberDto::from(member(Some("owner"), Some(Uuid::nil()), &["telemetry.view"]));
        assert_eq!((owner.role.as_str(), owner.is_owner), ("owner", true));
        let custom = MemberDto::from(member(None, Some(Uuid::nil()), &["tokens.manage", "future.power"]));
        assert_eq!(custom.role, "custom");
        assert_eq!(custom.permissions, vec!["tokens.view", "tokens.manage"], "normalised, unknown dropped");
        let individual = MemberDto::from(member(None, None, &[]));
        assert_eq!((individual.role.as_str(), individual.is_owner), ("individual", false));
        assert_eq!(individual.joined_at, "1970-01-01T00:00:00Z");
    }
}
