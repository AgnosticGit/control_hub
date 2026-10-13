//! Scoped incident rules and durable incident queue.

use axum::Json;
use axum::extract::{Path, State};
use karbird_domain::incidents::MetricKind;
use karbird_storage::StorageError;
use karbird_storage::incidents::{IncidentRow, IncidentRuleRow, NewRule, RuleUpdate};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::{parse_id, rfc3339};
use crate::auth::{CurrentUser, authorize};
use crate::error::{ApiError, ApiResult};
use crate::permissions::Permission;
use crate::state::AppState;

#[derive(Debug, Serialize, ToSchema)]
pub struct IncidentRuleDto {
    pub id: String,
    pub service_id: String,
    pub kind: String,
    pub threshold: f64,
    pub min_requests: i32,
    pub enabled: bool,
    pub last_window_end_ns: String,
    /// unknown, healthy or breaching; insufficient data is never healthy.
    pub observation_state: String,
    pub created_at: String,
}

impl From<IncidentRuleRow> for IncidentRuleDto {
    fn from(row: IncidentRuleRow) -> Self {
        Self {
            id: row.id.to_string(),
            service_id: row.service_id,
            kind: row.kind,
            threshold: row.threshold,
            min_requests: row.min_requests,
            enabled: row.enabled,
            last_window_end_ns: row.last_window_end_ns.to_string(),
            observation_state: row.observation_state,
            created_at: rfc3339(row.created_at),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct IncidentDto {
    pub id: String,
    pub rule_id: String,
    pub service_id: String,
    pub kind: String,
    pub threshold: f64,
    /// open, recovered or closed (rule reconfigured or disabled).
    pub state: String,
    pub opened_window_end_ns: String,
    pub last_window_end_ns: String,
    pub requests: i64,
    pub errors: i64,
    pub p95_ms: Option<f64>,
    pub acknowledged: bool,
    pub acknowledged_at: Option<String>,
    pub recovered_at: Option<String>,
    pub closed_reason: Option<String>,
    pub created_at: String,
}

impl From<IncidentRow> for IncidentDto {
    fn from(row: IncidentRow) -> Self {
        Self {
            id: row.id.to_string(),
            rule_id: row.rule_id.to_string(),
            service_id: row.service_id,
            kind: row.kind,
            threshold: row.threshold,
            state: row.state,
            opened_window_end_ns: row.opened_window_end_ns.to_string(),
            last_window_end_ns: row.last_window_end_ns.to_string(),
            requests: row.requests,
            errors: row.errors,
            p95_ms: row.p95_ms,
            acknowledged: row.acknowledged_at.is_some(),
            acknowledged_at: row.acknowledged_at.map(rfc3339),
            recovered_at: row.recovered_at.map(rfc3339),
            closed_reason: row.closed_reason,
            created_at: rfc3339(row.created_at),
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RuleRequest {
    pub service_id: String,
    /// `error_rate` (threshold in percent) or `p95_latency` (threshold in milliseconds).
    pub kind: String,
    pub threshold: f64,
    pub min_requests: i32,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateRuleRequest {
    pub threshold: f64,
    pub min_requests: i32,
    pub enabled: bool,
}

fn validate(kind: &str, service_id: &str, threshold: f64, min_requests: i32) -> ApiResult<()> {
    if service_id.is_empty() || service_id.len() > 200 || service_id.chars().any(char::is_control) {
        return Err(ApiError::BadRequest("service_id must be 1–200 characters without control characters".into()));
    }
    let kind =
        MetricKind::parse(kind).ok_or_else(|| ApiError::BadRequest("kind must be error_rate or p95_latency".into()))?;
    if !(1..=100_000).contains(&min_requests) {
        return Err(ApiError::BadRequest("min_requests must be between 1 and 100000".into()));
    }
    let max = if kind == MetricKind::ErrorRate { 100.0 } else { 60_000.0 };
    if !threshold.is_finite() || threshold <= 0.0 || threshold > max {
        return Err(ApiError::BadRequest(format!("threshold must be greater than 0 and at most {max}")));
    }
    Ok(())
}

/// Last full minute whose ingestion has had two minutes to settle.
pub(crate) fn closed_window_end_ns(now_ns: i64) -> i64 {
    const MINUTE: i64 = 60_000_000_000;
    now_ns.saturating_sub(2 * MINUTE).div_euclid(MINUTE) * MINUTE
}

fn current_window_end() -> i64 {
    let now = time::OffsetDateTime::now_utc().unix_timestamp_nanos();
    closed_window_end_ns(now.clamp(0, i64::MAX as i128) as i64)
}

#[utoipa::path(get, path = "/api/v1/projects/{project_id}/environments/{environment_id}/incident-rules", tag = "incidents",
    params(("project_id" = String, Path), ("environment_id" = String, Path)),
    responses((status = 200, body = Vec<IncidentRuleDto>), (status = 401), (status = 404)))]
pub async fn list_rules(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id)): Path<(String, String)>,
) -> ApiResult<Json<Vec<IncidentRuleDto>>> {
    let (scope, _) = authorize(&state, &user, &project_id, &environment_id, Permission::TelemetryView).await?;
    Ok(Json(state.pg.incident_rules(&scope).await?.into_iter().map(Into::into).collect()))
}

#[utoipa::path(post, path = "/api/v1/projects/{project_id}/environments/{environment_id}/incident-rules", tag = "incidents",
    params(("project_id" = String, Path), ("environment_id" = String, Path)), request_body = RuleRequest,
    responses((status = 200, body = IncidentRuleDto), (status = 400), (status = 401), (status = 403), (status = 404), (status = 409)))]
pub async fn create_rule(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id)): Path<(String, String)>,
    Json(body): Json<RuleRequest>,
) -> ApiResult<Json<IncidentRuleDto>> {
    let (scope, _) = authorize(&state, &user, &project_id, &environment_id, Permission::IncidentsManage).await?;
    validate(&body.kind, &body.service_id, body.threshold, body.min_requests)?;
    let input = NewRule {
        scope: &scope,
        service_id: &body.service_id,
        kind: &body.kind,
        threshold: body.threshold,
        min_requests: body.min_requests,
        actor: user.user_id,
        previous_window_end_ns: current_window_end().saturating_sub(2 * 60_000_000_000),
    };
    let row = state.pg.create_incident_rule(input).await.map_err(|e| match &e {
        StorageError::Postgres(sqlx::Error::Database(db)) if db.code().as_deref() == Some("23505") => {
            ApiError::Conflict("a rule for this service and metric already exists")
        }
        _ => ApiError::Storage(e),
    })?;
    Ok(Json(row.into()))
}

#[utoipa::path(patch, path = "/api/v1/projects/{project_id}/environments/{environment_id}/incident-rules/{rule_id}", tag = "incidents",
    params(("project_id" = String, Path), ("environment_id" = String, Path), ("rule_id" = String, Path)), request_body = UpdateRuleRequest,
    responses((status = 200, body = IncidentRuleDto), (status = 400), (status = 401), (status = 403), (status = 404)))]
pub async fn update_rule(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id, rule_id)): Path<(String, String, String)>,
    Json(body): Json<UpdateRuleRequest>,
) -> ApiResult<Json<IncidentRuleDto>> {
    let (scope, _) = authorize(&state, &user, &project_id, &environment_id, Permission::IncidentsManage).await?;
    let id = parse_id(&rule_id)?;
    let current = state.pg.incident_rule(&scope, id).await?.ok_or(ApiError::NotFound)?;
    validate(&current.kind, &current.service_id, body.threshold, body.min_requests)?;
    let row = state
        .pg
        .update_incident_rule(RuleUpdate {
            scope: &scope,
            id,
            actor: user.user_id,
            threshold: body.threshold,
            min_requests: body.min_requests,
            enabled: body.enabled,
            previous_window_end_ns: current_window_end().saturating_sub(60_000_000_000),
        })
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(row.into()))
}

#[utoipa::path(get, path = "/api/v1/projects/{project_id}/environments/{environment_id}/incidents", tag = "incidents",
    params(("project_id" = String, Path), ("environment_id" = String, Path)),
    responses((status = 200, body = Vec<IncidentDto>), (status = 401), (status = 404)))]
pub async fn list_incidents(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id)): Path<(String, String)>,
) -> ApiResult<Json<Vec<IncidentDto>>> {
    let (scope, _) = authorize(&state, &user, &project_id, &environment_id, Permission::TelemetryView).await?;
    Ok(Json(state.pg.incidents(&scope).await?.into_iter().map(Into::into).collect()))
}

#[utoipa::path(get, path = "/api/v1/projects/{project_id}/environments/{environment_id}/incidents/{incident_id}", tag = "incidents",
    params(("project_id" = String, Path), ("environment_id" = String, Path), ("incident_id" = String, Path)),
    responses((status = 200, body = IncidentDto), (status = 401), (status = 404)))]
pub async fn incident_detail(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id, incident_id)): Path<(String, String, String)>,
) -> ApiResult<Json<IncidentDto>> {
    let (scope, _) = authorize(&state, &user, &project_id, &environment_id, Permission::TelemetryView).await?;
    let row = state.pg.incident(&scope, parse_id(&incident_id)?).await?.ok_or(ApiError::NotFound)?;
    Ok(Json(row.into()))
}

#[utoipa::path(post, path = "/api/v1/projects/{project_id}/environments/{environment_id}/incidents/{incident_id}/acknowledge", tag = "incidents",
    params(("project_id" = String, Path), ("environment_id" = String, Path), ("incident_id" = String, Path)),
    responses((status = 200, body = IncidentDto), (status = 401), (status = 403), (status = 404)))]
pub async fn acknowledge(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id, incident_id)): Path<(String, String, String)>,
) -> ApiResult<Json<IncidentDto>> {
    let (scope, _) = authorize(&state, &user, &project_id, &environment_id, Permission::IncidentsManage).await?;
    let row = state
        .pg
        .acknowledge_incident(&scope, parse_id(&incident_id)?, user.user_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(row.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_thresholds_and_untrusted_service_ids() {
        assert!(validate("error_rate", "checkout", 20.0, 20).is_ok());
        assert!(validate("error_rate", "checkout", f64::NAN, 20).is_err());
        assert!(validate("error_rate", "checkout", 101.0, 20).is_err());
        assert!(validate("p95_latency", "checkout", 60_001.0, 20).is_err());
        assert!(validate("other", "checkout", 20.0, 20).is_err());
        assert!(validate("error_rate", "bad\nname", 20.0, 20).is_err());
        assert!(validate("error_rate", "checkout", 20.0, 0).is_err());
    }

    #[test]
    fn evaluator_uses_closed_aligned_windows() {
        assert_eq!(closed_window_end_ns(300_000_000_000), 180_000_000_000);
        assert_eq!(closed_window_end_ns(359_999_999_999), 180_000_000_000);
    }
}
