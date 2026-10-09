//! karbird administration (`/api/v1/admin/*`): every project, account and administrator of
//! the installation. Browser sessions only. Administrator grants follow the project rules (no
//! escalation, only a Super Admin touches a Super Admin, one always remains); every change and
//! every refused change is audited.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use karbird_storage::admin::{AdminAuditRow, AdminProjectRow, AdminUserRow};
use karbird_storage::pg::{EnvironmentRow, MemberChange, MemberRow, NewAuditEntry, RoleChange, RoleRow};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::permissions::{
    self, Actor, Builtin, Grant, PermissionInfo, Target, catalog_of, check_member_change, check_template_change,
    from_stored_of, parse_set_of,
};
use crate::routes::account::ProjectRequest;
use crate::routes::roles::{MemberDto, MemberUpdateRequest, RoleDto, RoleRequest, grant_json, validate_role_name};
use crate::routes::session::EnvironmentDto;
use crate::routes::signup::{normalise_email, required_name};
use crate::routes::{member_change_result, parse_id, rfc3339};
use crate::state::AppState;
use crate::system_permissions::{SystemPermission, SystemPermissionSet, check_account_change};

/// Projects that exist for the local stack itself and are never deleted from the panel.
const PROTECTED_PROJECTS: &[&str] = &["demo"];
const NEW_PROJECT_ENVIRONMENT: &str = "production";

// ------------------------------------------------------------------ access

/// The signed-in user's karbird administrator access.
#[derive(Debug, Clone)]
pub struct SystemAccess {
    pub actor: Actor<SystemPermission>,
}

/// Resolves the user's administrator grant and requires `need` (`None`: any administrator).
/// Users without any administrator permission get 404, so the panel's existence is not
/// confirmed to them; API tokens are refused (browser sessions only).
pub async fn authorize_system(
    state: &AppState,
    user: &CurrentUser,
    need: Option<SystemPermission>,
) -> ApiResult<SystemAccess> {
    user.session_id()?;
    let grant = state.pg.system_grant(user.user_id).await?.ok_or(ApiError::NotFound)?;
    let permissions: SystemPermissionSet = from_stored_of(&grant.permissions);
    if permissions.is_empty() {
        return Err(ApiError::NotFound);
    }
    if need.is_some_and(|p| !permissions.contains(&p)) {
        return Err(ApiError::Forbidden("insufficient permissions"));
    }
    Ok(SystemAccess { actor: Actor { user_id: user.user_id, permissions, is_owner: grant.is_super_admin() } })
}

/// The signed-in user's administrator access, for the session payload (`None` without any).
#[derive(Debug, Serialize, ToSchema)]
pub struct SystemAccessDto {
    /// `super_admin`, `admin`, `auditor`, `custom` (a role template) or `individual`.
    pub role: String,
    pub role_name: String,
    /// Effective ids (see `GET /api/v1/admin/permissions`).
    pub permissions: Vec<String>,
    pub is_super_admin: bool,
}

pub async fn system_access_for(state: &AppState, user: &CurrentUser) -> ApiResult<Option<SystemAccessDto>> {
    if !user.is_session() {
        return Ok(None);
    }
    let Some(grant) = state.pg.system_grant(user.user_id).await? else { return Ok(None) };
    let permissions: SystemPermissionSet = from_stored_of(&grant.permissions);
    if permissions.is_empty() {
        return Ok(None);
    }
    Ok(Some(SystemAccessDto {
        role: role_kind(grant.role_key.as_deref(), grant.role_id),
        is_super_admin: grant.is_super_admin(),
        role_name: grant.role_name,
        permissions: permissions::to_strings(&permissions),
    }))
}

fn role_kind(key: Option<&str>, role_id: Option<Uuid>) -> String {
    match (key, role_id) {
        (Some(key), _) => key.to_owned(),
        (None, Some(_)) => "custom".to_owned(),
        (None, None) => "individual".to_owned(),
    }
}

// ------------------------------------------------------------------ audit

struct AuditScope<'a> {
    tenant_id: Option<&'a str>,
    project_id: Option<&'a str>,
}

const SYSTEM_SCOPE: AuditScope<'static> = AuditScope { tenant_id: None, project_id: None };

fn project_scope(p: &AdminProjectRow) -> AuditScope<'_> {
    AuditScope { tenant_id: Some(&p.tenant_id), project_id: Some(&p.id) }
}

#[allow(clippy::too_many_arguments)]
async fn audit(
    state: &AppState,
    access: &SystemAccess,
    scope: AuditScope<'_>,
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
            tenant_id: scope.tenant_id,
            project_id: scope.project_id,
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
async fn deny(
    state: &AppState,
    access: &SystemAccess,
    scope: AuditScope<'_>,
    action: &str,
    target: &str,
    why: &'static str,
) -> ApiError {
    match audit(state, access, scope, action, target, Some(why), None, None, "denied").await {
        Ok(()) => ApiError::Forbidden(why),
        Err(e) => e,
    }
}

// ------------------------------------------------------------------ overview

#[derive(Debug, Serialize, ToSchema)]
pub struct AdminOverview {
    pub projects: i64,
    pub environments: i64,
    pub users: i64,
    pub disabled_users: i64,
    pub active_sessions: i64,
    pub administrators: i64,
}

#[utoipa::path(get, path = "/api/v1/admin/overview", tag = "admin", operation_id = "admin_overview",
    responses((status = 200, body = AdminOverview), (status = 401, body = crate::error::ErrorBody), (status = 403, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody)))]
pub async fn overview(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<AdminOverview>> {
    authorize_system(&state, &user, None).await?;
    let c = state.pg.admin_counts().await?;
    Ok(Json(AdminOverview {
        projects: c.projects,
        environments: c.environments,
        users: c.users,
        disabled_users: c.disabled_users,
        active_sessions: c.active_sessions,
        administrators: c.administrators,
    }))
}

#[utoipa::path(get, path = "/api/v1/admin/permissions", tag = "admin", operation_id = "admin_catalog",
    responses((status = 200, body = Vec<PermissionInfo>), (status = 401, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)))]
pub async fn catalog(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Vec<PermissionInfo>>> {
    authorize_system(&state, &user, None).await?;
    Ok(Json(catalog_of::<SystemPermission>()))
}

// ------------------------------------------------------------------ projects

#[derive(Debug, Serialize, ToSchema)]
pub struct AdminProjectDto {
    pub id: String,
    pub tenant_id: String,
    pub name: String,
    pub created_at: String,
    pub members: i64,
    /// Emails of the project's owners.
    pub owners: Vec<String>,
    pub environments: Vec<EnvironmentDto>,
    /// Cannot be deleted from the panel (the local demo project).
    pub protected: bool,
}

fn project_dto(p: AdminProjectRow, envs: &[EnvironmentRow]) -> AdminProjectDto {
    AdminProjectDto {
        environments: envs
            .iter()
            .filter(|e| e.project_id == p.id && e.tenant_id == p.tenant_id)
            .map(|e| EnvironmentDto { id: e.id.clone(), name: e.name.clone(), is_demo: e.is_demo })
            .collect(),
        protected: PROTECTED_PROJECTS.contains(&p.id.as_str()),
        created_at: rfc3339(p.created_at),
        id: p.id,
        tenant_id: p.tenant_id,
        name: p.name,
        members: p.members,
        owners: p.owners,
    }
}

async fn load_project(state: &AppState, project_id: &str) -> ApiResult<AdminProjectRow> {
    if karbird_domain::ProjectId::parse(project_id).is_err() {
        return Err(ApiError::NotFound);
    }
    state.pg.admin_project(project_id).await?.ok_or(ApiError::NotFound)
}

async fn project_with_envs(state: &AppState, p: AdminProjectRow) -> ApiResult<AdminProjectDto> {
    let envs = state.pg.environments_for_projects(std::slice::from_ref(&p.id)).await?;
    Ok(project_dto(p, &envs))
}

#[utoipa::path(get, path = "/api/v1/admin/projects", tag = "admin", operation_id = "admin_list_projects",
    responses((status = 200, body = Vec<AdminProjectDto>), (status = 401, body = crate::error::ErrorBody), (status = 403, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody)))]
pub async fn list_projects(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Vec<AdminProjectDto>>> {
    authorize_system(&state, &user, Some(SystemPermission::ProjectsView)).await?;
    let rows = state.pg.admin_projects().await?;
    let ids: Vec<String> = rows.iter().map(|p| p.id.clone()).collect();
    let envs = state.pg.environments_for_projects(&ids).await?;
    Ok(Json(rows.into_iter().map(|p| project_dto(p, &envs)).collect()))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AdminProjectCreateRequest {
    pub name: String,
    /// Email of an active account that becomes the owner; the acting administrator when absent.
    pub owner_email: Option<String>,
}

/// An active account by email, for granting it something.
async fn active_user_by_email(state: &AppState, raw: &str) -> ApiResult<Uuid> {
    let email = normalise_email(raw)?;
    let user = state.pg.find_user_by_email(&email).await?;
    user.map(|u| u.id).ok_or_else(|| ApiError::BadRequest("no active account with this email".into()))
}

#[utoipa::path(post, path = "/api/v1/admin/projects", tag = "admin", operation_id = "admin_create_project", request_body = AdminProjectCreateRequest,
    responses((status = 201, body = AdminProjectDto), (status = 400, body = crate::error::ErrorBody), (status = 401, body = crate::error::ErrorBody),
        (status = 403, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)))]
pub async fn create_project(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<AdminProjectCreateRequest>,
) -> ApiResult<(StatusCode, Json<AdminProjectDto>)> {
    let access = authorize_system(&state, &user, Some(SystemPermission::ProjectsManage)).await?;
    let name = required_name(&req.name, "project name")?;
    let owner = match req.owner_email.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(email) => active_user_by_email(&state, email).await?,
        None => user.user_id,
    };
    let scope = state.pg.create_project(owner, &name, NEW_PROJECT_ENVIRONMENT).await?;
    let p = load_project(&state, scope.project_id.as_str()).await?;
    let after = json!({ "name": name, "owner": owner.to_string() });
    audit(&state, &access, project_scope(&p), "admin.project.create", &p.id, None, None, Some(after), "succeeded")
        .await?;
    Ok((StatusCode::CREATED, Json(project_with_envs(&state, p).await?)))
}

#[utoipa::path(patch, path = "/api/v1/admin/projects/{project_id}", tag = "admin", operation_id = "admin_rename_project",
    params(("project_id" = String, Path)), request_body = ProjectRequest,
    responses((status = 200, body = AdminProjectDto), (status = 400, body = crate::error::ErrorBody), (status = 403, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody), (status = 409, body = crate::error::ErrorBody)))]
pub async fn rename_project(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(project_id): Path<String>,
    Json(req): Json<ProjectRequest>,
) -> ApiResult<Json<AdminProjectDto>> {
    let access = authorize_system(&state, &user, Some(SystemPermission::ProjectsManage)).await?;
    let p = load_project(&state, &project_id).await?;
    let name = required_name(&req.name, "project name")?;
    if !state.pg.rename_project(&p.id, &name).await? {
        return Err(ApiError::Conflict("another project of this organisation has this name"));
    }
    let (before, after) = (json!({ "name": p.name }), json!({ "name": name }));
    audit(
        &state,
        &access,
        project_scope(&p),
        "admin.project.rename",
        &p.id,
        None,
        Some(before),
        Some(after),
        "succeeded",
    )
    .await?;
    let updated = load_project(&state, &p.id).await?;
    Ok(Json(project_with_envs(&state, updated).await?))
}

#[utoipa::path(delete, path = "/api/v1/admin/projects/{project_id}", tag = "admin", operation_id = "admin_delete_project",
    params(("project_id" = String, Path)),
    responses((status = 204, description = "Deleted with its environments, members, roles, invites, tokens and plans"),
        (status = 403, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody), (status = 409, body = crate::error::ErrorBody)))]
pub async fn delete_project(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(project_id): Path<String>,
) -> ApiResult<StatusCode> {
    let access = authorize_system(&state, &user, Some(SystemPermission::ProjectsManage)).await?;
    let p = load_project(&state, &project_id).await?;
    if PROTECTED_PROJECTS.contains(&p.id.as_str()) {
        return Err(ApiError::Conflict("the demo project cannot be deleted"));
    }
    if !state.pg.delete_project(&p.id).await? {
        return Err(ApiError::NotFound);
    }
    let before = json!({ "name": p.name, "members": p.members, "owners": p.owners });
    audit(&state, &access, project_scope(&p), "admin.project.delete", &p.id, None, Some(before), None, "succeeded")
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct EnvironmentRequest {
    pub name: String,
}

#[utoipa::path(post, path = "/api/v1/admin/projects/{project_id}/environments", tag = "admin", operation_id = "admin_create_environment",
    params(("project_id" = String, Path)), request_body = EnvironmentRequest,
    responses((status = 201, body = AdminProjectDto), (status = 400, body = crate::error::ErrorBody), (status = 403, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody), (status = 409, body = crate::error::ErrorBody)))]
pub async fn create_environment(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(project_id): Path<String>,
    Json(req): Json<EnvironmentRequest>,
) -> ApiResult<(StatusCode, Json<AdminProjectDto>)> {
    let access = authorize_system(&state, &user, Some(SystemPermission::ProjectsManage)).await?;
    let p = load_project(&state, &project_id).await?;
    let name = required_name(&req.name, "environment name")?;
    let env = state
        .pg
        .create_environment(&p.id, &name)
        .await?
        .ok_or(ApiError::Conflict("the project already has an environment with this name"))?;
    let after = json!({ "name": name });
    audit(
        &state,
        &access,
        project_scope(&p),
        "admin.environment.create",
        &env.id,
        None,
        None,
        Some(after),
        "succeeded",
    )
    .await?;
    Ok((StatusCode::CREATED, Json(project_with_envs(&state, p).await?)))
}

// ------------------------------------------------------------------ project members

#[utoipa::path(get, path = "/api/v1/admin/projects/{project_id}/members", tag = "admin", operation_id = "admin_list_project_members",
    params(("project_id" = String, Path)),
    responses((status = 200, body = Vec<MemberDto>), (status = 403, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)))]
pub async fn list_project_members(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(project_id): Path<String>,
) -> ApiResult<Json<Vec<MemberDto>>> {
    authorize_system(&state, &user, Some(SystemPermission::ProjectsView)).await?;
    let p = load_project(&state, &project_id).await?;
    Ok(Json(state.pg.members(&p.id).await?.into_iter().map(Into::into).collect()))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AdminProjectMemberRequest {
    pub email: String,
    /// Built-in project role: `owner`, `admin`, `operator` or `viewer`.
    pub role: String,
}

#[utoipa::path(put, path = "/api/v1/admin/projects/{project_id}/members", tag = "admin", operation_id = "admin_put_project_member",
    params(("project_id" = String, Path)), request_body = AdminProjectMemberRequest,
    responses((status = 200, body = MemberDto), (status = 400, body = crate::error::ErrorBody), (status = 403, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody), (status = 409, body = crate::error::ErrorBody)))]
pub async fn put_project_member(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(project_id): Path<String>,
    Json(req): Json<AdminProjectMemberRequest>,
) -> ApiResult<Json<MemberDto>> {
    let access = authorize_system(&state, &user, Some(SystemPermission::ProjectsManage)).await?;
    let p = load_project(&state, &project_id).await?;
    let builtin = Builtin::ALL
        .into_iter()
        .find(|b| b.key() == req.role)
        .ok_or_else(|| ApiError::BadRequest("role: owner, admin, operator or viewer".into()))?;
    let member_id = active_user_by_email(&state, &req.email).await?;
    let before = state.pg.member(&p.id, member_id).await?;
    match &before {
        Some(_) => {
            let role = state.pg.roles(&p.id).await?.into_iter().find(|r| r.builtin.as_deref() == Some(builtin.key()));
            let role = role.ok_or(ApiError::NotFound)?;
            let change = state.pg.set_member_role(&p.id, member_id, role.id).await?;
            member_change_result(change, "a project needs at least one owner")?;
        }
        None => state.pg.upsert_membership(member_id, &p.id, builtin.key()).await?,
    }
    let updated = state.pg.member(&p.id, member_id).await?.ok_or(ApiError::NotFound)?;
    let before = before.map(|m| grant_json(&m.role_name, &m.permissions));
    let after = grant_json(&updated.role_name, &updated.permissions);
    let target = member_id.to_string();
    audit(&state, &access, project_scope(&p), "admin.member.put", &target, None, before, Some(after), "succeeded")
        .await?;
    Ok(Json(updated.into()))
}

#[utoipa::path(delete, path = "/api/v1/admin/projects/{project_id}/members/{user_id}", tag = "admin", operation_id = "admin_remove_project_member",
    params(("project_id" = String, Path), ("user_id" = String, Path)),
    responses((status = 204), (status = 403, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody),
        (status = 409, body = crate::error::ErrorBody)))]
pub async fn remove_project_member(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, user_id)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    let access = authorize_system(&state, &user, Some(SystemPermission::ProjectsManage)).await?;
    let p = load_project(&state, &project_id).await?;
    let member_id = parse_id(&user_id)?;
    let before = state.pg.member(&p.id, member_id).await?.ok_or(ApiError::NotFound)?;
    member_change_result(state.pg.remove_member(&p.id, member_id).await?, "a project needs at least one owner")?;
    let before = grant_json(&before.role_name, &before.permissions);
    audit(&state, &access, project_scope(&p), "admin.member.remove", &user_id, None, Some(before), None, "succeeded")
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ------------------------------------------------------------------ accounts

#[derive(Debug, Serialize, ToSchema)]
pub struct AdminUserDto {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub created_at: String,
    pub disabled: bool,
    /// Projects the account is a member of.
    pub projects: i64,
    pub active_sessions: i64,
    /// Name of the account's administrator role, if it has one.
    pub system_role: Option<String>,
}

impl From<AdminUserRow> for AdminUserDto {
    fn from(u: AdminUserRow) -> Self {
        Self {
            id: u.id.to_string(),
            email: u.email,
            display_name: u.display_name,
            created_at: rfc3339(u.created_at),
            disabled: u.disabled_at.is_some(),
            projects: u.projects,
            active_sessions: u.active_sessions,
            system_role: u.system_role,
        }
    }
}

#[utoipa::path(get, path = "/api/v1/admin/users", tag = "admin", operation_id = "admin_list_users",
    responses((status = 200, body = Vec<AdminUserDto>), (status = 401, body = crate::error::ErrorBody), (status = 403, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody)))]
pub async fn list_users(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Vec<AdminUserDto>>> {
    authorize_system(&state, &user, Some(SystemPermission::UsersView)).await?;
    Ok(Json(state.pg.admin_users().await?.into_iter().map(Into::into).collect()))
}

fn system_target(m: &MemberRow) -> Target<SystemPermission> {
    Target {
        user_id: m.user_id,
        is_owner: m.role_key.as_deref() == Some(karbird_storage::admin::SUPER_ADMIN),
        permissions: from_stored_of(&m.permissions),
    }
}

/// Loads the account and checks that `access` may change it; refusals are audited.
async fn account_target(
    state: &AppState,
    access: &SystemAccess,
    user_id: &str,
    action: &str,
) -> ApiResult<AdminUserRow> {
    let id = parse_id(user_id)?;
    let target = state.pg.admin_user(id).await?.ok_or(ApiError::NotFound)?;
    let admin = state.pg.system_member(id).await?.map(|m| system_target(&m));
    if let Err(why) = check_account_change(&access.actor, id, admin.as_ref()) {
        return Err(deny(state, access, SYSTEM_SCOPE, action, user_id, why).await);
    }
    Ok(target)
}

async fn set_disabled(
    state: AppState,
    user: CurrentUser,
    user_id: String,
    disabled: bool,
) -> ApiResult<Json<AdminUserDto>> {
    let access = authorize_system(&state, &user, Some(SystemPermission::UsersManage)).await?;
    let action = if disabled { "admin.user.disable" } else { "admin.user.enable" };
    let target = account_target(&state, &access, &user_id, action).await?;
    match state.pg.set_user_disabled(target.id, disabled).await? {
        MemberChange::Done | MemberChange::NotFound => {}
        MemberChange::LastOwner => return Err(ApiError::Conflict("karbird needs at least one active Super Admin")),
    }
    audit(
        &state,
        &access,
        SYSTEM_SCOPE,
        action,
        &user_id,
        None,
        None,
        Some(json!({ "email": target.email })),
        "succeeded",
    )
    .await?;
    Ok(Json(state.pg.admin_user(target.id).await?.ok_or(ApiError::NotFound)?.into()))
}

#[utoipa::path(post, path = "/api/v1/admin/users/{user_id}/disable", tag = "admin", operation_id = "admin_disable_user",
    params(("user_id" = String, Path)),
    responses((status = 200, body = AdminUserDto, description = "Disabled; every session of the account is ended"),
        (status = 403, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody), (status = 409, body = crate::error::ErrorBody)))]
pub async fn disable_user(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(user_id): Path<String>,
) -> ApiResult<Json<AdminUserDto>> {
    set_disabled(state, user, user_id, true).await
}

#[utoipa::path(post, path = "/api/v1/admin/users/{user_id}/enable", tag = "admin", operation_id = "admin_enable_user",
    params(("user_id" = String, Path)),
    responses((status = 200, body = AdminUserDto), (status = 403, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)))]
pub async fn enable_user(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(user_id): Path<String>,
) -> ApiResult<Json<AdminUserDto>> {
    set_disabled(state, user, user_id, false).await
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EndedSessions {
    pub revoked: u64,
}

#[utoipa::path(post, path = "/api/v1/admin/users/{user_id}/sessions/revoke", tag = "admin", operation_id = "admin_revoke_user_sessions",
    params(("user_id" = String, Path)),
    responses((status = 200, body = EndedSessions), (status = 403, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)))]
pub async fn revoke_user_sessions(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(user_id): Path<String>,
) -> ApiResult<Json<EndedSessions>> {
    let access = authorize_system(&state, &user, Some(SystemPermission::UsersManage)).await?;
    let target = account_target(&state, &access, &user_id, "admin.user.sessions_revoke").await?;
    let revoked = state.pg.revoke_all_sessions(target.id).await?;
    let after = json!({ "revoked": revoked });
    audit(&state, &access, SYSTEM_SCOPE, "admin.user.sessions_revoke", &user_id, None, None, Some(after), "succeeded")
        .await?;
    Ok(Json(EndedSessions { revoked }))
}

// ------------------------------------------------------------------ administrators

fn system_permissions_of(items: &[String]) -> ApiResult<SystemPermissionSet> {
    parse_set_of(items).map_err(|e| ApiError::BadRequest(format!("permissions: {e}")))
}

fn system_role_dto(r: RoleRow) -> RoleDto {
    RoleDto {
        id: r.id.to_string(),
        name: r.name,
        permissions: permissions::to_strings(&from_stored_of::<SystemPermission>(&r.permissions)),
        builtin: r.builtin,
        holders: r.holders,
    }
}

fn system_member_dto(m: MemberRow) -> MemberDto {
    let is_owner = m.role_key.as_deref() == Some(karbird_storage::admin::SUPER_ADMIN);
    MemberDto {
        role: role_kind(m.role_key.as_deref(), m.role_id),
        user_id: m.user_id.to_string(),
        email: m.email,
        display_name: m.display_name,
        role_id: m.role_id.map(|id| id.to_string()),
        role_name: m.role_name,
        based_on: m.based_on.map(|id| id.to_string()),
        permissions: permissions::to_strings(&from_stored_of::<SystemPermission>(&m.permissions)),
        is_owner,
        joined_at: rfc3339(m.created_at),
    }
}

fn system_grant_json(m: &MemberRow) -> Value {
    grant_json(&m.role_name, &permissions::to_strings(&from_stored_of::<SystemPermission>(&m.permissions)))
}

/// A system role as a grant (`owner` = the Super Admin role).
fn role_grant(r: &RoleRow) -> Grant<SystemPermission> {
    Grant::Role {
        permissions: from_stored_of(&r.permissions),
        owner: r.builtin.as_deref() == Some(karbird_storage::admin::SUPER_ADMIN),
    }
}

#[utoipa::path(get, path = "/api/v1/admin/members", tag = "admin", operation_id = "admin_list_members",
    responses((status = 200, body = Vec<MemberDto>), (status = 401, body = crate::error::ErrorBody), (status = 403, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody)))]
pub async fn list_members(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Vec<MemberDto>>> {
    authorize_system(&state, &user, Some(SystemPermission::TeamView)).await?;
    Ok(Json(state.pg.system_members().await?.into_iter().map(system_member_dto).collect()))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AdminMemberCreateRequest {
    pub email: String,
    pub role_id: String,
}

#[utoipa::path(post, path = "/api/v1/admin/members", tag = "admin", operation_id = "admin_add_member", request_body = AdminMemberCreateRequest,
    responses((status = 201, body = MemberDto), (status = 400, body = crate::error::ErrorBody), (status = 403, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody), (status = 409, body = crate::error::ErrorBody)))]
pub async fn add_member(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<AdminMemberCreateRequest>,
) -> ApiResult<(StatusCode, Json<MemberDto>)> {
    let access = authorize_system(&state, &user, Some(SystemPermission::TeamManage)).await?;
    let member_id = active_user_by_email(&state, &req.email).await?;
    let role = state.pg.system_role(parse_id(&req.role_id)?).await?.ok_or(ApiError::NotFound)?;
    let target = member_id.to_string();
    if state.pg.system_member(member_id).await?.is_some() {
        return Err(ApiError::Conflict("this account is already an administrator"));
    }
    let nobody = Target { user_id: member_id, is_owner: false, permissions: SystemPermissionSet::new() };
    if let Err(why) = check_member_change(&access.actor, &nobody, &role_grant(&role), 0) {
        return Err(deny(&state, &access, SYSTEM_SCOPE, "admin.admin.add", &target, why).await);
    }
    match state.pg.add_system_member(member_id, role.id).await? {
        MemberChange::Done => {}
        MemberChange::NotFound => return Err(ApiError::Conflict("this account is already an administrator")),
        MemberChange::LastOwner => return Err(ApiError::Conflict("karbird needs at least one Super Admin")),
    }
    let added = state.pg.system_member(member_id).await?.ok_or(ApiError::NotFound)?;
    let after = system_grant_json(&added);
    audit(&state, &access, SYSTEM_SCOPE, "admin.admin.add", &target, None, None, Some(after), "succeeded").await?;
    Ok((StatusCode::CREATED, Json(system_member_dto(added))))
}

#[utoipa::path(patch, path = "/api/v1/admin/members/{user_id}", tag = "admin", operation_id = "admin_update_member",
    params(("user_id" = String, Path)), request_body = MemberUpdateRequest,
    responses((status = 200, body = MemberDto), (status = 400, body = crate::error::ErrorBody), (status = 403, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody), (status = 409, body = crate::error::ErrorBody)))]
pub async fn update_member(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(user_id): Path<String>,
    Json(req): Json<MemberUpdateRequest>,
) -> ApiResult<Json<MemberDto>> {
    let access = authorize_system(&state, &user, Some(SystemPermission::TeamManage)).await?;
    let member_id = parse_id(&user_id)?;
    let (grant, role_id) = match (&req.role_id, &req.permissions) {
        (Some(role_id), None) => {
            let r = state.pg.system_role(parse_id(role_id)?).await?.ok_or(ApiError::NotFound)?;
            (role_grant(&r), Some(r.id))
        }
        (None, Some(items)) => (Grant::Individual(system_permissions_of(items)?), None),
        _ => return Err(ApiError::BadRequest("send exactly one of role_id or permissions".into())),
    };
    let based_on = match req.based_on.as_deref() {
        Some(id) if role_id.is_none() => Some(parse_id(id).map_err(|_| ApiError::BadRequest("based_on".into()))?),
        _ => None,
    };
    let target = state.pg.system_member(member_id).await?.ok_or(ApiError::NotFound)?;
    let supers = state.pg.count_super_admins().await?;
    if let Err(why) = check_member_change(&access.actor, &system_target(&target), &grant, supers) {
        return Err(deny(&state, &access, SYSTEM_SCOPE, "admin.admin.update", &user_id, why).await);
    }
    let change = match (role_id, &grant) {
        (Some(id), _) => state.pg.set_system_member_role(member_id, id).await?,
        (None, Grant::Individual(set)) => {
            state.pg.set_system_member_permissions(member_id, &permissions::to_strings(set), based_on).await?
        }
        (None, _) => MemberChange::NotFound,
    };
    member_change_result(change, "karbird needs at least one Super Admin")?;
    let updated = state.pg.system_member(member_id).await?.ok_or(ApiError::NotFound)?;
    let (before, after) = (system_grant_json(&target), system_grant_json(&updated));
    audit(&state, &access, SYSTEM_SCOPE, "admin.admin.update", &user_id, None, Some(before), Some(after), "succeeded")
        .await?;
    Ok(Json(system_member_dto(updated)))
}

#[utoipa::path(delete, path = "/api/v1/admin/members/{user_id}", tag = "admin", operation_id = "admin_remove_member",
    params(("user_id" = String, Path)),
    responses((status = 204), (status = 403, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody),
        (status = 409, body = crate::error::ErrorBody)))]
pub async fn remove_member(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(user_id): Path<String>,
) -> ApiResult<StatusCode> {
    let access = authorize_system(&state, &user, Some(SystemPermission::TeamManage)).await?;
    let member_id = parse_id(&user_id)?;
    let target = state.pg.system_member(member_id).await?.ok_or(ApiError::NotFound)?;
    let supers = state.pg.count_super_admins().await?;
    if let Err(why) = check_member_change(&access.actor, &system_target(&target), &Grant::Remove, supers) {
        return Err(deny(&state, &access, SYSTEM_SCOPE, "admin.admin.remove", &user_id, why).await);
    }
    member_change_result(state.pg.remove_system_member(member_id).await?, "karbird needs at least one Super Admin")?;
    let before = system_grant_json(&target);
    audit(&state, &access, SYSTEM_SCOPE, "admin.admin.remove", &user_id, None, Some(before), None, "succeeded").await?;
    Ok(StatusCode::NO_CONTENT)
}

// ------------------------------------------------------------------ administrator roles

#[utoipa::path(get, path = "/api/v1/admin/roles", tag = "admin", operation_id = "admin_list_roles",
    responses((status = 200, body = Vec<RoleDto>), (status = 401, body = crate::error::ErrorBody), (status = 403, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody)))]
pub async fn list_roles(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Vec<RoleDto>>> {
    authorize_system(&state, &user, Some(SystemPermission::TeamView)).await?;
    Ok(Json(state.pg.system_roles().await?.into_iter().map(system_role_dto).collect()))
}

#[utoipa::path(post, path = "/api/v1/admin/roles", tag = "admin", operation_id = "admin_create_role", request_body = RoleRequest,
    responses((status = 200, body = RoleDto), (status = 400, body = crate::error::ErrorBody), (status = 403, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody), (status = 409, body = crate::error::ErrorBody)))]
pub async fn create_role(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<RoleRequest>,
) -> ApiResult<Json<RoleDto>> {
    let access = authorize_system(&state, &user, Some(SystemPermission::RolesManage)).await?;
    let name = validate_role_name(&req.name)?;
    let wanted = system_permissions_of(&req.permissions)?;
    if let Err(why) = check_template_change(&access.actor, None, &wanted) {
        return Err(deny(&state, &access, SYSTEM_SCOPE, "admin.role.create", name, why).await);
    }
    let stored = permissions::to_strings(&wanted);
    let id = state
        .pg
        .create_system_role(name, &stored)
        .await?
        .ok_or(ApiError::Conflict("a role with this name already exists"))?;
    let role = state.pg.system_role(id).await?.ok_or(ApiError::NotFound)?;
    let after = grant_json(name, &stored);
    audit(&state, &access, SYSTEM_SCOPE, "admin.role.create", &id.to_string(), None, None, Some(after), "succeeded")
        .await?;
    Ok(Json(system_role_dto(role)))
}

#[utoipa::path(patch, path = "/api/v1/admin/roles/{role_id}", tag = "admin", operation_id = "admin_update_role",
    params(("role_id" = String, Path)), request_body = RoleRequest,
    responses((status = 200, body = RoleDto), (status = 400, body = crate::error::ErrorBody), (status = 403, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody), (status = 409, body = crate::error::ErrorBody)))]
pub async fn update_role(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(role_id): Path<String>,
    Json(req): Json<RoleRequest>,
) -> ApiResult<Json<RoleDto>> {
    let access = authorize_system(&state, &user, Some(SystemPermission::RolesManage)).await?;
    let id = parse_id(&role_id)?;
    let name = validate_role_name(&req.name)?;
    let wanted = system_permissions_of(&req.permissions)?;
    let old = state.pg.system_role(id).await?.ok_or(ApiError::NotFound)?;
    if old.builtin.is_some() {
        return Err(deny(
            &state,
            &access,
            SYSTEM_SCOPE,
            "admin.role.update",
            &role_id,
            "built-in roles cannot be changed",
        )
        .await);
    }
    let old_set: SystemPermissionSet = from_stored_of(&old.permissions);
    if let Err(why) = check_template_change(&access.actor, Some(&old_set), &wanted) {
        return Err(deny(&state, &access, SYSTEM_SCOPE, "admin.role.update", &role_id, why).await);
    }
    let stored = permissions::to_strings(&wanted);
    match state.pg.update_system_role(id, name, &stored).await? {
        RoleChange::Done => {}
        RoleChange::NameTaken => return Err(ApiError::Conflict("a role with this name already exists")),
        RoleChange::Builtin => return Err(ApiError::Forbidden("built-in roles cannot be changed")),
        RoleChange::NotFound | RoleChange::Held(_) => return Err(ApiError::NotFound),
    }
    let role = state.pg.system_role(id).await?.ok_or(ApiError::NotFound)?;
    let before = grant_json(&old.name, &permissions::to_strings(&old_set));
    audit(
        &state,
        &access,
        SYSTEM_SCOPE,
        "admin.role.update",
        &role_id,
        None,
        Some(before),
        Some(grant_json(name, &stored)),
        "succeeded",
    )
    .await?;
    Ok(Json(system_role_dto(role)))
}

#[utoipa::path(delete, path = "/api/v1/admin/roles/{role_id}", tag = "admin", operation_id = "admin_delete_role",
    params(("role_id" = String, Path)),
    responses((status = 204), (status = 403, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody),
        (status = 409, body = crate::error::ErrorBody)))]
pub async fn delete_role(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(role_id): Path<String>,
) -> ApiResult<StatusCode> {
    let access = authorize_system(&state, &user, Some(SystemPermission::RolesManage)).await?;
    let id = parse_id(&role_id)?;
    let old = state.pg.system_role(id).await?.ok_or(ApiError::NotFound)?;
    if old.builtin.is_some() {
        return Err(deny(
            &state,
            &access,
            SYSTEM_SCOPE,
            "admin.role.delete",
            &role_id,
            "built-in roles cannot be changed",
        )
        .await);
    }
    let old_set: SystemPermissionSet = from_stored_of(&old.permissions);
    if let Err(why) = check_template_change(&access.actor, Some(&old_set), &SystemPermissionSet::new()) {
        return Err(deny(&state, &access, SYSTEM_SCOPE, "admin.role.delete", &role_id, why).await);
    }
    match state.pg.delete_system_role(id).await? {
        RoleChange::Done => {}
        RoleChange::Held(_) => return Err(ApiError::Conflict("the role is still assigned to administrators")),
        RoleChange::Builtin => return Err(ApiError::Forbidden("built-in roles cannot be changed")),
        RoleChange::NotFound | RoleChange::NameTaken => return Err(ApiError::NotFound),
    }
    let before = grant_json(&old.name, &permissions::to_strings(&old_set));
    audit(&state, &access, SYSTEM_SCOPE, "admin.role.delete", &role_id, None, Some(before), None, "succeeded").await?;
    Ok(StatusCode::NO_CONTENT)
}

// ------------------------------------------------------------------ audit

#[derive(Debug, Deserialize, IntoParams)]
pub struct AdminAuditQuery {
    /// Only entries of this project.
    pub project_id: Option<String>,
    /// 1-500, default 200.
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AdminAuditEntryDto {
    pub id: String,
    pub occurred_at: String,
    pub project_id: Option<String>,
    pub actor_id: String,
    /// Email of the acting account, when it still exists.
    pub actor_email: Option<String>,
    pub actor_kind: String,
    pub action: String,
    pub target: Option<String>,
    pub reason: Option<String>,
    pub outcome: String,
}

impl From<AdminAuditRow> for AdminAuditEntryDto {
    fn from(r: AdminAuditRow) -> Self {
        Self {
            id: r.id.to_string(),
            occurred_at: rfc3339(r.occurred_at),
            project_id: r.project_id,
            actor_id: r.actor_id,
            actor_email: r.actor_email,
            actor_kind: r.actor_kind,
            action: r.action,
            target: r.target,
            reason: r.reason,
            outcome: r.outcome,
        }
    }
}

#[utoipa::path(get, path = "/api/v1/admin/audit", tag = "admin", operation_id = "admin_audit_log", params(AdminAuditQuery),
    responses((status = 200, body = Vec<AdminAuditEntryDto>), (status = 401, body = crate::error::ErrorBody), (status = 403, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody)))]
pub async fn audit_log(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<AdminAuditQuery>,
) -> ApiResult<Json<Vec<AdminAuditEntryDto>>> {
    authorize_system(&state, &user, Some(SystemPermission::AuditView)).await?;
    let project = q.project_id.as_deref().filter(|p| !p.is_empty());
    if project.is_some_and(|p| karbird_domain::ProjectId::parse(p).is_err()) {
        return Err(ApiError::BadRequest("project_id".into()));
    }
    let rows = state.pg.admin_audit(project, q.limit.unwrap_or(200)).await?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use time::OffsetDateTime;

    fn member(role_key: Option<&str>, role_id: Option<Uuid>, permissions: &[&str]) -> MemberRow {
        MemberRow {
            user_id: Uuid::from_u128(7),
            email: "a@example.test".into(),
            display_name: "A".into(),
            role_id,
            role_key: role_key.map(Into::into),
            role_name: "R".into(),
            based_on: None,
            permissions: permissions.iter().map(|s| (*s).to_owned()).collect(),
            created_at: OffsetDateTime::from_unix_timestamp(0).unwrap(),
        }
    }

    #[test]
    fn administrator_dtos_use_system_permissions_and_mark_the_super_admin() {
        let sa = system_member_dto(member(
            Some("super_admin"),
            Some(Uuid::nil()),
            &["system.team.manage", "telemetry.view"],
        ));
        assert!(sa.is_owner, "the Super Admin is the top role");
        assert_eq!(sa.role, "super_admin");
        assert_eq!(sa.permissions, vec!["system.team.view", "system.team.manage"], "normalised; project ids dropped");
        let custom = system_member_dto(member(None, Some(Uuid::nil()), &[]));
        assert_eq!((custom.role.as_str(), custom.is_owner), ("custom", false));
        let owner_key = system_member_dto(member(Some("owner"), Some(Uuid::nil()), &[]));
        assert!(!owner_key.is_owner, "a project key never makes a Super Admin");
        assert!(system_target(&member(Some("super_admin"), None, &[])).is_owner);
        assert!(!system_target(&member(Some("admin"), None, &[])).is_owner);
    }

    #[test]
    fn only_the_super_admin_role_is_an_owner_grant() {
        let role = |builtin: Option<&str>| RoleRow {
            id: Uuid::nil(),
            name: "x".into(),
            permissions: vec!["system.audit.view".into()],
            builtin: builtin.map(Into::into),
            holders: 0,
        };
        assert!(matches!(role_grant(&role(Some("super_admin"))), Grant::Role { owner: true, .. }));
        for b in [Some("admin"), Some("auditor"), Some("owner"), None] {
            assert!(matches!(role_grant(&role(b)), Grant::Role { owner: false, .. }), "{b:?}");
        }
    }

    #[test]
    fn the_demo_project_is_protected_and_shows_only_its_environments() {
        let p = |id: &str| AdminProjectRow {
            id: id.into(),
            tenant_id: "t".into(),
            name: "P".into(),
            created_at: OffsetDateTime::from_unix_timestamp(0).unwrap(),
            members: 1,
            owners: vec!["o@example.test".into()],
        };
        let env = |id: &str, project: &str, tenant: &str| EnvironmentRow {
            id: id.into(),
            project_id: project.into(),
            tenant_id: tenant.into(),
            name: id.into(),
            is_demo: false,
        };
        let envs = [env("e1", "demo", "t"), env("e2", "other", "t"), env("e3", "demo", "foreign")];
        let demo = project_dto(p("demo"), &envs);
        assert!(demo.protected);
        assert_eq!(
            demo.environments.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            ["e1"],
            "same project and tenant only"
        );
        assert!(!project_dto(p("p-1"), &envs).protected);
    }

    #[test]
    fn requested_system_permissions_are_strict() {
        assert!(system_permissions_of(&["system.users.manage".into()]).unwrap().contains(&SystemPermission::UsersView));
        for bad in ["telemetry.view", "system.*", "super_admin"] {
            assert!(matches!(system_permissions_of(&[bad.into()]), Err(ApiError::BadRequest(_))), "{bad}");
        }
    }
}
