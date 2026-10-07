pub mod account;
pub mod actions;
pub mod admin;
pub mod edge_traffic;
pub mod infra_timeline;
pub mod infrastructure;
pub mod invites;
pub mod live;
pub mod oauth;
pub mod organisations;
pub mod roles;
pub mod session;
pub mod signup;
pub mod telemetry;
pub mod tokens;

use axum::Json;
use axum::extract::{Path, State};
use karbird_storage::pg::AuditRow;
use serde::Serialize;
use utoipa::ToSchema;

use crate::auth::{CurrentUser, authorize};
use crate::error::ApiResult;
use crate::permissions::Permission;
use crate::state::AppState;

#[derive(Debug, Serialize, ToSchema)]
pub struct HealthResponse {
    pub status: String,
}

#[utoipa::path(get, path = "/api/v1/health", tag = "health", responses((status = 200, body = HealthResponse)))]
pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok".into() })
}

#[utoipa::path(get, path = "/api/v1/ready", tag = "health",
    responses((status = 200, body = HealthResponse), (status = 503, body = crate::error::ErrorBody)))]
pub async fn ready(State(state): State<AppState>) -> ApiResult<Json<HealthResponse>> {
    state.pg.ping().await?;
    state.ch.ping().await?;
    Ok(Json(HealthResponse { status: "ready".into() }))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuditEntryDto {
    pub id: String,
    pub occurred_at: String,
    pub actor_id: String,
    pub actor_kind: String,
    pub action: String,
    pub target: Option<String>,
    pub reason: Option<String>,
    pub outcome: String,
}

impl From<AuditRow> for AuditEntryDto {
    fn from(r: AuditRow) -> Self {
        Self {
            id: r.id.to_string(),
            occurred_at: r.occurred_at.format(&time::format_description::well_known::Rfc3339).unwrap_or_default(),
            actor_id: r.actor_id,
            actor_kind: r.actor_kind,
            action: r.action,
            target: r.target,
            reason: r.reason,
            outcome: r.outcome,
        }
    }
}

#[utoipa::path(get, path = "/api/v1/projects/{project_id}/environments/{environment_id}/audit", tag = "audit",
    params(("project_id" = String, Path), ("environment_id" = String, Path)),
    responses((status = 200, body = Vec<AuditEntryDto>), (status = 404, body = crate::error::ErrorBody)))]
pub async fn audit(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id)): Path<(String, String)>,
) -> ApiResult<Json<Vec<AuditEntryDto>>> {
    let (scope, _) = authorize(&state, &user, &project_id, &environment_id, Permission::TelemetryView).await?;
    let rows = state.pg.audit_entries(scope.tenant_id.as_str(), scope.project_id.as_str(), 200).await?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn audit_rows_map_to_rfc3339_dtos() {
        let r = AuditRow {
            id: uuid::Uuid::nil(),
            occurred_at: time::OffsetDateTime::from_unix_timestamp(0).unwrap(),
            actor_id: "u1".into(),
            actor_kind: "user".into(),
            action: "token.create".into(),
            target: Some("t1".into()),
            reason: None,
            outcome: "succeeded".into(),
        };
        let d = AuditEntryDto::from(r);
        assert_eq!(d.id, "00000000-0000-0000-0000-000000000000");
        assert_eq!(d.occurred_at, "1970-01-01T00:00:00Z");
        assert_eq!((d.action.as_str(), d.target.as_deref(), d.reason), ("token.create", Some("t1"), None));
    }
}
