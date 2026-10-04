//! Invite links: a member with `members.manage` creates a link that adds whoever opens it to the
//! project with one role. Only the hash of the secret is stored; the web app puts the secret in
//! the URL fragment, so it never reaches a server log.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use controlhub_storage::pg::{InviteAccept, InviteLookup, InviteRow, NewAuditEntry, NewInvite};
use controlhub_storage::secrets::{generate_token, hash_token, token_prefix};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::auth::{CurrentUser, ProjectAccess, authorize_project, check_origin};
use crate::error::{ApiError, ApiResult};
use crate::permissions::{Actor, Permission, check_invite, from_stored};
use crate::state::AppState;

pub const INVITE_PREFIX: &str = "chv_";
pub const DEFAULT_EXPIRY_DAYS: u32 = 7;
pub const MAX_EXPIRY_DAYS: u32 = 30;
pub const MAX_USES: i32 = 1000;

#[derive(Debug, Serialize, ToSchema)]
pub struct InviteDto {
    pub id: String,
    pub role_id: String,
    pub role_name: String,
    /// Non-secret start of the link's secret, for telling links apart.
    pub prefix: String,
    pub created_by: String,
    pub created_by_name: String,
    pub created_at: String,
    pub expires_at: String,
    /// `null` = unlimited.
    pub max_uses: Option<i32>,
    pub uses: i32,
}

fn rfc3339(t: OffsetDateTime) -> String {
    t.format(&Rfc3339).unwrap_or_default()
}

impl From<InviteRow> for InviteDto {
    fn from(r: InviteRow) -> Self {
        Self {
            id: r.id.to_string(),
            role_id: r.role_id.to_string(),
            role_name: r.role_name,
            prefix: r.token_prefix,
            created_by: r.created_by.to_string(),
            created_by_name: r.created_by_name,
            created_at: rfc3339(r.created_at),
            expires_at: rfc3339(r.expires_at),
            max_uses: r.max_uses,
            uses: r.uses,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateInviteRequest {
    pub role_id: String,
    /// 1-30, default 7.
    pub expires_in_days: Option<u32>,
    /// 1-1000; absent or `null` = unlimited.
    pub max_uses: Option<i32>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreatedInvite {
    pub invite: InviteDto,
    /// Shown once; the link is `<web app>/app/invite#<secret>`.
    pub secret: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct InviteTokenRequest {
    pub token: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct InvitePreview {
    pub project_name: String,
    pub role_name: String,
    pub invited_by: String,
    pub expires_at: String,
    /// Whether the request came with a browser session.
    pub signed_in: bool,
    /// Signed in and already a member of the project.
    pub already_member: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AcceptedInvite {
    pub project_id: String,
    /// First environment of the project, to open after joining.
    pub environment_id: Option<String>,
    pub already_member: bool,
}

fn parse_id(id: &str) -> ApiResult<Uuid> {
    Uuid::parse_str(id).map_err(|_| ApiError::NotFound)
}

fn expiry_days(days: Option<u32>) -> ApiResult<u32> {
    match days.unwrap_or(DEFAULT_EXPIRY_DAYS) {
        d @ 1..=MAX_EXPIRY_DAYS => Ok(d),
        _ => Err(ApiError::BadRequest(format!("expires_in_days must be 1-{MAX_EXPIRY_DAYS}"))),
    }
}

fn max_uses(n: Option<i32>) -> ApiResult<Option<i32>> {
    match n {
        None => Ok(None),
        Some(n @ 1..=MAX_USES) => Ok(Some(n)),
        Some(_) => Err(ApiError::BadRequest(format!("max_uses must be 1-{MAX_USES}"))),
    }
}

/// Anything that cannot be one of our secrets is "not found" before any lookup.
fn token_hash(token: &str) -> ApiResult<Vec<u8>> {
    let t = token.trim();
    let well_formed = t.starts_with(INVITE_PREFIX)
        && t.len() <= 128
        && t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if well_formed { Ok(hash_token(t)) } else { Err(ApiError::NotFound) }
}

#[allow(clippy::too_many_arguments)]
async fn audit(
    state: &AppState,
    tenant_id: &str,
    project_id: &str,
    actor_id: Uuid,
    action: &str,
    target: &str,
    reason: Option<&str>,
    after: Option<Value>,
    outcome: &str,
) -> ApiResult<()> {
    state
        .pg
        .audit(NewAuditEntry {
            tenant_id: Some(tenant_id),
            project_id: Some(project_id),
            actor_id: &actor_id.to_string(),
            actor_kind: "user",
            action,
            target: Some(target),
            reason,
            before: None,
            after,
            outcome,
            request_id: None,
        })
        .await?;
    Ok(())
}

async fn deny(state: &AppState, access: &ProjectAccess, target: &str, why: &'static str) -> ApiError {
    let (t, p) = (access.tenant_id.as_str(), access.project_id.as_str());
    match audit(state, t, p, access.actor.user_id, "invite.create", target, Some(why), None, "denied").await {
        Ok(()) => ApiError::Forbidden(why),
        Err(e) => e,
    }
}

#[utoipa::path(get, path = "/api/v1/projects/{project_id}/invites", tag = "members", operation_id = "list_invites",
    params(("project_id" = String, Path)),
    responses((status = 200, body = Vec<InviteDto>), (status = 403, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)))]
pub async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(project_id): Path<String>,
) -> ApiResult<Json<Vec<InviteDto>>> {
    let access = authorize_project(&state, &user, &project_id, Permission::MembersManage).await?;
    Ok(Json(state.pg.invites(access.project_id.as_str()).await?.into_iter().map(Into::into).collect()))
}

#[utoipa::path(post, path = "/api/v1/projects/{project_id}/invites", tag = "members", operation_id = "create_invite",
    params(("project_id" = String, Path)), request_body = CreateInviteRequest,
    responses((status = 201, body = CreatedInvite), (status = 400, body = crate::error::ErrorBody),
        (status = 403, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)))]
pub async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(project_id): Path<String>,
    Json(req): Json<CreateInviteRequest>,
) -> ApiResult<(StatusCode, Json<CreatedInvite>)> {
    let access = authorize_project(&state, &user, &project_id, Permission::MembersManage).await?;
    user.session_id()?;
    let days = expiry_days(req.expires_in_days)?;
    let uses = max_uses(req.max_uses)?;
    let project = access.project_id.as_str();
    let role = state.pg.role(project, parse_id(&req.role_id)?).await?.ok_or(ApiError::NotFound)?;
    let owner = role.builtin.as_deref() == Some("owner");
    if let Err(why) = check_invite(&access.actor, &from_stored(&role.permissions), owner) {
        return Err(deny(&state, &access, &req.role_id, why).await);
    }
    let secret = generate_token(INVITE_PREFIX)?;
    let expires_at = OffsetDateTime::now_utc() + time::Duration::days(i64::from(days));
    let id = state
        .pg
        .create_invite(&NewInvite {
            project_id: project,
            role_id: role.id,
            token_hash: &hash_token(&secret),
            token_prefix: &token_prefix(&secret),
            created_by: user.user_id,
            expires_at,
            max_uses: uses,
        })
        .await?
        .ok_or(ApiError::NotFound)?;
    let invite = state.pg.invite(project, id).await?.ok_or(ApiError::NotFound)?;
    let after = json!({ "role": role.name, "expires_at": rfc3339(expires_at), "max_uses": uses });
    audit(
        &state,
        access.tenant_id.as_str(),
        project,
        user.user_id,
        "invite.create",
        &id.to_string(),
        None,
        Some(after),
        "succeeded",
    )
    .await?;
    Ok((StatusCode::CREATED, Json(CreatedInvite { invite: invite.into(), secret })))
}

#[utoipa::path(delete, path = "/api/v1/projects/{project_id}/invites/{invite_id}", tag = "members", operation_id = "revoke_invite",
    params(("project_id" = String, Path), ("invite_id" = String, Path)),
    responses((status = 204), (status = 403, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)))]
pub async fn revoke(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, invite_id)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    let access = authorize_project(&state, &user, &project_id, Permission::MembersManage).await?;
    user.session_id()?;
    let project = access.project_id.as_str();
    if !state.pg.revoke_invite(project, parse_id(&invite_id)?).await? {
        return Err(ApiError::NotFound);
    }
    audit(
        &state,
        access.tenant_id.as_str(),
        project,
        user.user_id,
        "invite.revoke",
        &invite_id,
        None,
        None,
        "succeeded",
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The invite behind a secret, if it can still be accepted: not revoked, expired or used up,
/// and its creator may still grant the role.
async fn usable_invite(state: &AppState, token: &str) -> ApiResult<InviteLookup> {
    let invite = state.pg.find_invite(&token_hash(token)?).await?.ok_or(ApiError::NotFound)?;
    if !invite.is_usable(OffsetDateTime::now_utc()) {
        return Err(ApiError::Gone("this invite link has expired, was used up or was revoked"));
    }
    let creator = state.pg.member(&invite.project_id, invite.created_by).await?;
    let still_allowed = creator.is_some_and(|m| {
        let actor = Actor { user_id: m.user_id, permissions: from_stored(&m.permissions), is_owner: m.is_owner() };
        check_invite(&actor, &from_stored(&invite.role_permissions), invite.role_key.as_deref() == Some("owner"))
            .is_ok()
    });
    if !still_allowed {
        return Err(ApiError::Gone("this invite link is no longer valid"));
    }
    Ok(invite)
}

async fn is_member(state: &AppState, user_id: Uuid, project_id: &str) -> ApiResult<bool> {
    Ok(state.pg.memberships(user_id).await?.iter().any(|m| m.project_id == project_id))
}

#[utoipa::path(post, path = "/api/v1/invites/preview", tag = "members", operation_id = "preview_invite", request_body = InviteTokenRequest,
    responses((status = 200, body = InvitePreview), (status = 404, body = crate::error::ErrorBody), (status = 410, body = crate::error::ErrorBody)))]
pub async fn preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: Option<CurrentUser>,
    Json(req): Json<InviteTokenRequest>,
) -> ApiResult<Json<InvitePreview>> {
    check_origin(&state, &headers)?;
    let invite = usable_invite(&state, &req.token).await?;
    let session = user.filter(CurrentUser::is_session);
    let already_member = match &session {
        Some(u) => is_member(&state, u.user_id, &invite.project_id).await?,
        None => false,
    };
    Ok(Json(InvitePreview {
        project_name: invite.project_name,
        role_name: invite.role_name,
        invited_by: invite.inviter_name,
        expires_at: rfc3339(invite.expires_at),
        signed_in: session.is_some(),
        already_member,
    }))
}

#[utoipa::path(post, path = "/api/v1/invites/accept", tag = "members", operation_id = "accept_invite", request_body = InviteTokenRequest,
    responses((status = 200, body = AcceptedInvite), (status = 401, body = crate::error::ErrorBody), (status = 403, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody), (status = 410, body = crate::error::ErrorBody)))]
pub async fn accept(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<InviteTokenRequest>,
) -> ApiResult<Json<AcceptedInvite>> {
    user.session_id()?;
    let invite = usable_invite(&state, &req.token).await?;
    let outcome = state.pg.accept_invite(invite.id, user.user_id).await?;
    if outcome == InviteAccept::Gone {
        return Err(ApiError::Gone("this invite link has expired, was used up or was revoked"));
    }
    let already_member = outcome == InviteAccept::AlreadyMember;
    if !already_member {
        let after = json!({ "role": invite.role_name, "invite": invite.id.to_string() });
        let target = user.user_id.to_string();
        audit(
            &state,
            &invite.tenant_id,
            &invite.project_id,
            user.user_id,
            "invite.accept",
            &target,
            None,
            Some(after),
            "succeeded",
        )
        .await?;
    }
    let envs = state.pg.environments_for_projects(std::slice::from_ref(&invite.project_id)).await?;
    Ok(Json(AcceptedInvite {
        environment_id: envs.into_iter().next().map(|e| e.id),
        project_id: invite.project_id,
        already_member,
    }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn expiry_defaults_to_a_week_and_is_bounded() {
        assert_eq!(expiry_days(None).unwrap(), 7);
        assert_eq!(expiry_days(Some(1)).unwrap(), 1);
        assert_eq!(expiry_days(Some(30)).unwrap(), 30);
        for bad in [0, 31, 365, u32::MAX] {
            assert!(matches!(expiry_days(Some(bad)), Err(ApiError::BadRequest(_))), "{bad}");
        }
    }

    #[test]
    fn use_limits_are_optional_and_bounded() {
        assert_eq!(max_uses(None).unwrap(), None);
        assert_eq!(max_uses(Some(1)).unwrap(), Some(1));
        assert_eq!(max_uses(Some(1000)).unwrap(), Some(1000));
        for bad in [0, -1, 1001, i32::MIN, i32::MAX] {
            assert!(matches!(max_uses(Some(bad)), Err(ApiError::BadRequest(_))), "{bad}");
        }
    }

    #[test]
    fn only_well_formed_invite_secrets_are_looked_up() {
        let t = generate_token(INVITE_PREFIX).unwrap();
        assert_eq!(token_hash(&t).unwrap(), hash_token(&t));
        assert_eq!(token_hash(&format!("  {t}\n")).unwrap(), hash_token(&t), "copy-paste whitespace");
        let too_long = format!("chv_{}", "a".repeat(200));
        for bad in ["", "cha_abc", "chs_abc", "chi_abc", "chv_a b", "chv_a%20", "chv_<script>", too_long.as_str()] {
            assert!(matches!(token_hash(bad), Err(ApiError::NotFound)), "{bad:?}");
        }
    }
}
