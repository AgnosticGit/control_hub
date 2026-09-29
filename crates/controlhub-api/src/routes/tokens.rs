//! Token management for one environment: ingestion tokens (SDK/Collector) and read-only
//! API tokens (CLI `context`, MCP). Requires the admin role and a browser session (CSRF
//! protected); API tokens can never manage tokens. Secrets are returned exactly once and
//! stored only as SHA-256 hashes. Every mutation is written to the audit log.

use axum::Json;
use axum::extract::{Path, State};
use controlhub_domain::Scope;
use controlhub_storage::pg::{NewAuditEntry, TokenKind, TokenRow};
use controlhub_storage::secrets::{generate_token, hash_token, token_prefix};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::auth::{API_TOKEN_PREFIX, CurrentUser, Role, authorize};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const INGEST_TOKEN_PREFIX: &str = "chi_";
const MAX_API_TOKEN_DAYS: u32 = 90;
const DEFAULT_API_TOKEN_DAYS: u32 = 30;
const MAX_INGEST_TOKEN_DAYS: u32 = 365;
const MAX_ROTATION_GRACE_SECONDS: u32 = 24 * 3600;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TokenKindDto {
    Ingest,
    Api,
}

impl From<TokenKindDto> for TokenKind {
    fn from(k: TokenKindDto) -> Self {
        match k {
            TokenKindDto::Ingest => TokenKind::Ingest,
            TokenKindDto::Api => TokenKind::Api,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TokenDto {
    pub id: String,
    pub kind: TokenKindDto,
    pub name: String,
    /// Non-secret prefix for identification.
    pub prefix: String,
    pub signals: Option<Vec<String>>,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub revoked_at: Option<String>,
    pub last_used_at: Option<String>,
    pub active: bool,
}

fn rfc3339(t: OffsetDateTime) -> String {
    t.format(&Rfc3339).unwrap_or_default()
}

impl TokenDto {
    fn from_row(r: TokenRow) -> Self {
        let now = OffsetDateTime::now_utc();
        Self {
            id: r.id.to_string(),
            kind: if r.kind == "api" { TokenKindDto::Api } else { TokenKindDto::Ingest },
            name: r.name,
            prefix: r.token_prefix,
            signals: r.signals,
            created_at: rfc3339(r.created_at),
            active: r.revoked_at.is_none() && r.expires_at.is_none_or(|e| e > now),
            expires_at: r.expires_at.map(rfc3339),
            revoked_at: r.revoked_at.map(rfc3339),
            last_used_at: r.last_used_at.map(rfc3339),
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateTokenRequest {
    pub kind: TokenKindDto,
    pub name: String,
    /// API tokens: 1–90 days (default 30). Ingestion tokens: 1–365 days, or omitted for no expiry.
    pub expires_in_days: Option<u32>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreatedTokenResponse {
    pub token: TokenDto,
    /// Shown exactly once; only its hash is stored.
    pub secret: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RevokeTokenRequest {
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RotateTokenRequest {
    /// Seconds the old token keeps working (0–86400, default 0) so running senders can switch.
    pub grace_seconds: Option<u32>,
    pub reason: Option<String>,
}

fn validate_name(name: &str) -> ApiResult<&str> {
    let name = name.trim();
    let ok = !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-'));
    if ok { Ok(name) } else { Err(ApiError::BadRequest("name: 1-64 characters [A-Za-z0-9 ._-]".into())) }
}

fn validate_reason(reason: Option<&str>) -> ApiResult<Option<&str>> {
    match reason.map(str::trim).filter(|r| !r.is_empty()) {
        Some(r) if r.len() > 500 => Err(ApiError::BadRequest("reason: at most 500 characters".into())),
        r => Ok(r),
    }
}

fn expiry(kind: TokenKindDto, days: Option<u32>) -> ApiResult<Option<OffsetDateTime>> {
    let (days, max) = match kind {
        TokenKindDto::Api => (Some(days.unwrap_or(DEFAULT_API_TOKEN_DAYS)), MAX_API_TOKEN_DAYS),
        TokenKindDto::Ingest => (days, MAX_INGEST_TOKEN_DAYS),
    };
    match days {
        None => Ok(None),
        Some(d) if (1..=max).contains(&d) => Ok(Some(OffsetDateTime::now_utc() + time::Duration::days(i64::from(d)))),
        Some(_) => Err(ApiError::BadRequest(format!("expires_in_days must be 1-{max}"))),
    }
}

/// The replacement keeps the lifetime policy: same expiry length, capped by kind.
fn rotated_lifetime_days(
    kind: TokenKindDto,
    created_at: OffsetDateTime,
    expires_at: Option<OffsetDateTime>,
) -> Option<u32> {
    let max = if kind == TokenKindDto::Api { MAX_API_TOKEN_DAYS } else { MAX_INGEST_TOKEN_DAYS };
    expires_at.map(|e| ((e - created_at).whole_days().clamp(1, i64::from(u32::MAX)) as u32).min(max))
}

async fn issue(
    state: &AppState,
    scope: &Scope,
    user: &CurrentUser,
    kind: TokenKindDto,
    name: &str,
    expires_at: Option<OffsetDateTime>,
) -> ApiResult<(Uuid, String)> {
    let secret = generate_token(match kind {
        TokenKindDto::Api => API_TOKEN_PREFIX,
        TokenKindDto::Ingest => INGEST_TOKEN_PREFIX,
    })?;
    let (hash, prefix) = (hash_token(&secret), token_prefix(&secret));
    let id = match (kind, expires_at) {
        (TokenKindDto::Api, Some(exp)) => {
            state.pg.create_api_token(scope, name, &hash, &prefix, exp, user.user_id).await?
        }
        (TokenKindDto::Api, None) => return Err(ApiError::BadRequest("API tokens must expire".into())),
        (TokenKindDto::Ingest, exp) => {
            state.pg.create_ingest_token(scope, name, &hash, &prefix, exp, Some(user.user_id)).await?
        }
    };
    Ok((id, secret))
}

#[allow(clippy::too_many_arguments)]
async fn audit(
    state: &AppState,
    scope: &Scope,
    user: &CurrentUser,
    action: &str,
    target: Uuid,
    reason: Option<&str>,
    before: Option<&TokenDto>,
    after: Option<&TokenDto>,
) -> ApiResult<()> {
    let json = |t: &TokenDto| serde_json::json!({ "kind": t.kind, "name": t.name, "prefix": t.prefix, "expires_at": t.expires_at, "revoked_at": t.revoked_at });
    state
        .pg
        .audit(NewAuditEntry {
            tenant_id: Some(scope.tenant_id.as_str()),
            project_id: Some(scope.project_id.as_str()),
            actor_id: &user.user_id.to_string(),
            actor_kind: "user",
            action,
            target: Some(&target.to_string()),
            reason,
            before: before.map(json),
            after: after.map(json),
            outcome: "succeeded",
            request_id: None,
        })
        .await?;
    Ok(())
}

async fn find(state: &AppState, scope: &Scope, token_id: &str) -> ApiResult<TokenRow> {
    let id = Uuid::parse_str(token_id).map_err(|_| ApiError::NotFound)?;
    state.pg.list_tokens(scope).await?.into_iter().find(|t| t.id == id).ok_or(ApiError::NotFound)
}

fn kind_of(row: &TokenRow) -> TokenKindDto {
    if row.kind == "api" { TokenKindDto::Api } else { TokenKindDto::Ingest }
}

#[utoipa::path(get, path = "/api/v1/projects/{project_id}/environments/{environment_id}/tokens", tag = "tokens",
    params(("project_id" = String, Path), ("environment_id" = String, Path)),
    responses((status = 200, body = Vec<TokenDto>), (status = 403, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)))]
pub async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id)): Path<(String, String)>,
) -> ApiResult<Json<Vec<TokenDto>>> {
    let (scope, _) = authorize(&state, &user, &project_id, &environment_id, Role::Admin).await?;
    Ok(Json(state.pg.list_tokens(&scope).await?.into_iter().map(TokenDto::from_row).collect()))
}

#[utoipa::path(post, path = "/api/v1/projects/{project_id}/environments/{environment_id}/tokens", tag = "tokens",
    params(("project_id" = String, Path), ("environment_id" = String, Path)), request_body = CreateTokenRequest,
    responses((status = 200, body = CreatedTokenResponse), (status = 400, body = crate::error::ErrorBody),
        (status = 403, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)))]
pub async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id)): Path<(String, String)>,
    Json(req): Json<CreateTokenRequest>,
) -> ApiResult<Json<CreatedTokenResponse>> {
    let (scope, _) = authorize(&state, &user, &project_id, &environment_id, Role::Admin).await?;
    user.session_id()?;
    let name = validate_name(&req.name)?;
    let expires_at = expiry(req.kind, req.expires_in_days)?;
    let (id, secret) = issue(&state, &scope, &user, req.kind, name, expires_at).await?;
    let token = TokenDto::from_row(state.pg.find_token(&scope, req.kind.into(), id).await?.ok_or(ApiError::NotFound)?);
    audit(&state, &scope, &user, "token.create", id, None, None, Some(&token)).await?;
    Ok(Json(CreatedTokenResponse { token, secret }))
}

#[utoipa::path(post, path = "/api/v1/projects/{project_id}/environments/{environment_id}/tokens/{token_id}/revoke", tag = "tokens",
    params(("project_id" = String, Path), ("environment_id" = String, Path), ("token_id" = String, Path)),
    request_body = RevokeTokenRequest,
    responses((status = 200, body = TokenDto), (status = 403, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)))]
pub async fn revoke(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id, token_id)): Path<(String, String, String)>,
    Json(req): Json<RevokeTokenRequest>,
) -> ApiResult<Json<TokenDto>> {
    let (scope, _) = authorize(&state, &user, &project_id, &environment_id, Role::Admin).await?;
    user.session_id()?;
    let reason = validate_reason(req.reason.as_deref())?;
    let row = find(&state, &scope, &token_id).await?;
    let kind = kind_of(&row);
    let before = TokenDto::from_row(row.clone());
    if !state.pg.end_token(&scope, kind.into(), row.id, OffsetDateTime::now_utc()).await? {
        return Err(ApiError::BadRequest("token is already revoked".into()));
    }
    let after = TokenDto::from_row(find(&state, &scope, &token_id).await?);
    audit(&state, &scope, &user, "token.revoke", row.id, reason, Some(&before), Some(&after)).await?;
    Ok(Json(after))
}

#[utoipa::path(post, path = "/api/v1/projects/{project_id}/environments/{environment_id}/tokens/{token_id}/rotate", tag = "tokens",
    params(("project_id" = String, Path), ("environment_id" = String, Path), ("token_id" = String, Path)),
    request_body = RotateTokenRequest,
    responses((status = 200, body = CreatedTokenResponse), (status = 400, body = crate::error::ErrorBody),
        (status = 403, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)))]
pub async fn rotate(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id, token_id)): Path<(String, String, String)>,
    Json(req): Json<RotateTokenRequest>,
) -> ApiResult<Json<CreatedTokenResponse>> {
    let (scope, _) = authorize(&state, &user, &project_id, &environment_id, Role::Admin).await?;
    user.session_id()?;
    let reason = validate_reason(req.reason.as_deref())?;
    let grace = req.grace_seconds.unwrap_or(0);
    if grace > MAX_ROTATION_GRACE_SECONDS {
        return Err(ApiError::BadRequest(format!("grace_seconds must be 0-{MAX_ROTATION_GRACE_SECONDS}")));
    }
    let old = find(&state, &scope, &token_id).await?;
    if old.revoked_at.is_some() {
        return Err(ApiError::BadRequest("token is already revoked".into()));
    }
    let kind = kind_of(&old);
    let now = OffsetDateTime::now_utc();
    let expires_at = expiry(kind, rotated_lifetime_days(kind, old.created_at, old.expires_at))?;
    let (new_id, secret) = issue(&state, &scope, &user, kind, &old.name, expires_at).await?;
    let before = TokenDto::from_row(old.clone());
    state.pg.end_token(&scope, kind.into(), old.id, now + time::Duration::seconds(i64::from(grace))).await?;
    let old_after = TokenDto::from_row(find(&state, &scope, &token_id).await?);
    let token = TokenDto::from_row(state.pg.find_token(&scope, kind.into(), new_id).await?.ok_or(ApiError::NotFound)?);
    audit(&state, &scope, &user, "token.rotate", old.id, reason, Some(&before), Some(&old_after)).await?;
    audit(&state, &scope, &user, "token.create", new_id, reason, None, Some(&token)).await?;
    Ok(Json(CreatedTokenResponse { token, secret }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn names_and_expiry_are_validated() {
        assert!(validate_name("ci deploy-1").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name("<script>").is_err());
        assert!(validate_name(&"x".repeat(65)).is_err());
        assert!(expiry(TokenKindDto::Api, None).unwrap().is_some(), "API tokens always expire");
        assert!(expiry(TokenKindDto::Api, Some(91)).is_err());
        assert!(expiry(TokenKindDto::Ingest, None).unwrap().is_none());
        assert!(expiry(TokenKindDto::Ingest, Some(0)).is_err());
    }

    fn days_from_now(t: Option<OffsetDateTime>) -> i64 {
        (t.unwrap() - OffsetDateTime::now_utc()).whole_hours().div_euclid(24) + 1
    }

    #[test]
    fn expiry_bounds_per_kind() {
        assert_eq!(days_from_now(expiry(TokenKindDto::Api, None).unwrap()), 30, "default 30 days");
        assert_eq!(days_from_now(expiry(TokenKindDto::Api, Some(1)).unwrap()), 1);
        assert_eq!(days_from_now(expiry(TokenKindDto::Api, Some(90)).unwrap()), 90);
        assert!(expiry(TokenKindDto::Api, Some(0)).is_err());
        assert!(expiry(TokenKindDto::Api, Some(u32::MAX)).is_err());
        assert_eq!(days_from_now(expiry(TokenKindDto::Ingest, Some(365)).unwrap()), 365);
        assert!(expiry(TokenKindDto::Ingest, Some(366)).is_err());
        match expiry(TokenKindDto::Api, Some(91)) {
            Err(ApiError::BadRequest(m)) => assert_eq!(m, "expires_in_days must be 1-90"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn names_allow_only_a_safe_charset() {
        assert_eq!(validate_name("  deploy bot  ").unwrap(), "deploy bot", "trimmed");
        assert!(validate_name(&"x".repeat(64)).is_ok());
        for bad in ["   ", "a/b", "a\nb", "a;b", "naïve", "a\"b", "a'b", "a\u{0}b", "a\u{202e}b"] {
            assert!(validate_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn reasons_are_optional_and_bounded() {
        assert_eq!(validate_reason(None).unwrap(), None);
        assert_eq!(validate_reason(Some("  ")).unwrap(), None);
        assert_eq!(validate_reason(Some(" leaked in CI ")).unwrap(), Some("leaked in CI"));
        assert!(validate_reason(Some(&"x".repeat(500))).is_ok());
        assert!(validate_reason(Some(&"x".repeat(501))).is_err());
    }

    fn row(kind: &str, expires_at: Option<OffsetDateTime>, revoked_at: Option<OffsetDateTime>) -> TokenRow {
        TokenRow {
            id: Uuid::nil(),
            kind: kind.into(),
            name: "ci".into(),
            token_prefix: "cha_abcdef".into(),
            signals: None,
            created_at: OffsetDateTime::now_utc() - time::Duration::days(1),
            expires_at,
            revoked_at,
            last_used_at: None,
            created_by: None,
        }
    }

    #[test]
    fn token_dto_activity_and_kind() {
        let now = OffsetDateTime::now_utc();
        let future = Some(now + time::Duration::days(1));
        let past = Some(now - time::Duration::seconds(1));
        assert!(TokenDto::from_row(row("api", future, None)).active);
        assert!(TokenDto::from_row(row("ingest", None, None)).active, "ingest tokens may never expire");
        assert!(!TokenDto::from_row(row("api", past, None)).active, "expired");
        assert!(!TokenDto::from_row(row("api", future, past)).active, "revoked");
        assert_eq!(TokenDto::from_row(row("api", future, None)).kind, TokenKindDto::Api);
        assert_eq!(TokenDto::from_row(row("ingest", None, None)).kind, TokenKindDto::Ingest);
        assert_eq!(kind_of(&row("api", None, None)), TokenKindDto::Api);
        assert_eq!(kind_of(&row("ingest", None, None)), TokenKindDto::Ingest);
        let v = serde_json::to_value(TokenDto::from_row(row("api", future, None))).unwrap();
        assert!(v.get("token_hash").is_none() && v.get("secret").is_none(), "metadata only: {v}");
        assert_eq!(v["kind"], "api");
    }

    #[test]
    fn rotation_keeps_the_lifetime_capped_by_kind() {
        let c = OffsetDateTime::from_unix_timestamp(1_790_000_000).unwrap();
        let d = |n: i64| Some(c + time::Duration::days(n));
        assert_eq!(rotated_lifetime_days(TokenKindDto::Api, c, d(30)), Some(30));
        assert_eq!(rotated_lifetime_days(TokenKindDto::Api, c, d(400)), Some(MAX_API_TOKEN_DAYS));
        assert_eq!(rotated_lifetime_days(TokenKindDto::Ingest, c, d(400)), Some(MAX_INGEST_TOKEN_DAYS));
        assert_eq!(rotated_lifetime_days(TokenKindDto::Ingest, c, None), None, "no expiry stays no expiry");
        assert_eq!(
            rotated_lifetime_days(TokenKindDto::Api, c, Some(c + time::Duration::hours(3))),
            Some(1),
            "at least a day"
        );
        assert_eq!(rotated_lifetime_days(TokenKindDto::Api, c, Some(c - time::Duration::days(5))), Some(1));
        // Whatever it returns is accepted by `expiry` for the same kind.
        for kind in [TokenKindDto::Api, TokenKindDto::Ingest] {
            for n in [-10, 0, 1, 89, 90, 91, 365, 10_000] {
                assert!(expiry(kind, rotated_lifetime_days(kind, c, d(n))).is_ok(), "{kind:?} {n}");
            }
        }
    }

    #[test]
    fn kind_dto_maps_to_storage_kind() {
        assert_eq!(TokenKind::from(TokenKindDto::Api), TokenKind::Api);
        assert_eq!(TokenKind::from(TokenKindDto::Ingest), TokenKind::Ingest);
        assert_eq!(serde_json::from_str::<TokenKindDto>("\"ingest\"").unwrap(), TokenKindDto::Ingest);
        assert!(serde_json::from_str::<TokenKindDto>("\"admin\"").is_err());
    }
}
