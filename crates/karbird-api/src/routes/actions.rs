//! Controlled actions: propose (operator), approve (admin, never the proposer), reject (admin),
//! cancel (proposer or admin). The API never executes anything: approved plans are picked up
//! by the separate executor process. Every state change is committed with its audit entry.

use axum::Json;
use axum::extract::{Path, State};
use karbird_actions::{self as actions, Action, PolicyDecision, PolicyInput};
use karbird_domain::Scope;
use karbird_storage::actions::{ActionPlanRow, NewActionPlan, Transition};
use karbird_storage::pg::NewAuditEntry;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use utoipa::ToSchema;
use uuid::Uuid;

use super::AuditEntryDto;
use crate::auth::{Authorized, CurrentUser, authorize, authorize_as};
use crate::error::{ApiError, ApiResult};
use crate::permissions::Permission;
use crate::state::AppState;

#[derive(Debug, Serialize, ToSchema)]
pub struct ParamSpecDto {
    pub name: String,
    pub description: String,
    pub min: f64,
    pub max: f64,
    pub integer: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ActionSpecDto {
    pub action_type: String,
    pub title: String,
    pub description: String,
    /// `low` or `medium`.
    pub risk: String,
    pub demo_only: bool,
    pub targets: Vec<String>,
    pub params: Vec<ParamSpecDto>,
    pub verification: String,
    /// Whether policy allows this type in this environment at all (demo-only check).
    pub allowed_here: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ActionCatalogResponse {
    pub environment_is_demo: bool,
    pub actions: Vec<ActionSpecDto>,
}

/// Schema of the stored policy decision (serialized `karbird_actions::PolicyDecision`).
#[derive(Debug, Serialize, ToSchema)]
pub struct PolicyDto {
    pub allowed: bool,
    pub requires_approval: bool,
    pub checks: Vec<PolicyCheckDto>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PolicyCheckDto {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ActionPlanDto {
    pub id: String,
    pub action_type: String,
    pub title: String,
    pub risk: String,
    pub target: String,
    #[schema(value_type = std::collections::HashMap<String, f64>)]
    pub params: Value,
    pub reason: String,
    /// proposed, approved, rejected, cancelled, expired, running, succeeded, failed, verification_failed
    pub state: String,
    #[schema(value_type = PolicyDto)]
    pub policy: Value,
    pub created_by: String,
    pub created_at: String,
    /// Deadline for the decision (proposed) or for the executor to start (approved).
    pub expires_at: String,
    pub decided_by: Option<String>,
    pub decided_at: Option<String>,
    pub decision_comment: Option<String>,
    pub executor_id: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    #[schema(value_type = Option<Object>)]
    pub result: Option<Value>,
    #[schema(value_type = Option<Object>)]
    pub verification: Option<Value>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ActionPlanDetail {
    pub plan: ActionPlanDto,
    pub audit: Vec<AuditEntryDto>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ProposeActionRequest {
    pub action_type: String,
    pub target: String,
    #[serde(default)]
    #[schema(value_type = std::collections::HashMap<String, f64>)]
    pub params: Value,
    pub reason: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DecisionRequest {
    /// Required when rejecting.
    pub comment: Option<String>,
}

fn ts(t: OffsetDateTime) -> String {
    t.format(&Rfc3339).unwrap_or_default()
}

fn risk_str(r: actions::Risk) -> String {
    match r {
        actions::Risk::Low => "low".into(),
        actions::Risk::Medium => "medium".into(),
    }
}

impl From<ActionPlanRow> for ActionPlanDto {
    fn from(r: ActionPlanRow) -> Self {
        let spec = actions::spec(&r.action_type);
        Self {
            id: r.id.to_string(),
            title: spec.map_or_else(|| r.action_type.clone(), |s| s.title.to_owned()),
            risk: spec.map_or_else(|| "unknown".into(), |s| risk_str(s.risk)),
            action_type: r.action_type,
            target: r.target,
            params: r.params,
            reason: r.reason,
            state: r.state,
            policy: r.policy,
            created_by: r.created_by_email,
            created_at: ts(r.created_at),
            expires_at: ts(r.expires_at),
            decided_by: r.decided_by_email,
            decided_at: r.decided_at.map(ts),
            decision_comment: r.decision_comment,
            executor_id: r.executor_id,
            started_at: r.started_at.map(ts),
            finished_at: r.finished_at.map(ts),
            result: r.result,
            verification: r.verification,
        }
    }
}

fn clean_comment(c: Option<&str>) -> ApiResult<Option<String>> {
    match c.map(str::trim).filter(|c| !c.is_empty()) {
        Some(c) if c.chars().count() > 500 || c.chars().any(char::is_control) => {
            Err(ApiError::BadRequest("comment: at most 500 characters without control characters".into()))
        }
        c => Ok(c.map(str::to_owned)),
    }
}

async fn policy(
    state: &AppState,
    scope: &Scope,
    action: &Action,
    is_demo: bool,
    exclude_self: bool,
) -> ApiResult<PolicyDecision> {
    let (open, recent) = state.pg.action_counts(scope, action.target()).await?;
    let input = PolicyInput { environment_is_demo: is_demo, open_plans_for_target: open, plans_last_hour: recent };
    Ok(actions::evaluate(action, &input, exclude_self))
}

fn denied_message(d: &PolicyDecision) -> String {
    d.checks.iter().filter(|c| !c.passed).map(|c| format!("{}: {}", c.name, c.detail)).collect::<Vec<_>>().join("; ")
}

fn user_audit<'a>(
    scope: &'a Scope,
    user_id: &'a str,
    action: &'a str,
    reason: Option<&'a str>,
    after: Value,
) -> NewAuditEntry<'a> {
    NewAuditEntry {
        tenant_id: Some(scope.tenant_id.as_str()),
        project_id: Some(scope.project_id.as_str()),
        actor_id: user_id,
        actor_kind: "user",
        action,
        target: None,
        reason,
        before: None,
        after: Some(after),
        outcome: "succeeded",
        request_id: None,
    }
}

fn plan_id(id: &str) -> ApiResult<Uuid> {
    Uuid::parse_str(id).map_err(|_| ApiError::NotFound)
}

/// Re-validates a stored plan (it was validated at proposal; the catalog may have changed since).
fn stored_action(row: &ActionPlanRow) -> ApiResult<Action> {
    actions::validate(&row.action_type, &row.target, &row.params)
        .map_err(|_| ApiError::Conflict("plan no longer passes validation"))
}

#[utoipa::path(get, path = "/api/v1/projects/{project_id}/environments/{environment_id}/actions/catalog", tag = "actions", operation_id = "action_catalog",
    params(("project_id" = String, Path), ("environment_id" = String, Path)),
    responses((status = 200, body = ActionCatalogResponse), (status = 404, body = crate::error::ErrorBody)))]
pub async fn catalog(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id)): Path<(String, String)>,
) -> ApiResult<Json<ActionCatalogResponse>> {
    let (_, is_demo) = authorize(&state, &user, &project_id, &environment_id, Permission::ActionsView).await?;
    Ok(Json(ActionCatalogResponse {
        environment_is_demo: is_demo,
        actions: actions::CATALOG
            .iter()
            .map(|s| ActionSpecDto {
                action_type: s.action_type.into(),
                title: s.title.into(),
                description: s.description.into(),
                risk: risk_str(s.risk),
                demo_only: s.demo_only,
                targets: s.targets.iter().map(|t| (*t).to_owned()).collect(),
                params: s
                    .params
                    .iter()
                    .map(|p| ParamSpecDto {
                        name: p.name.into(),
                        description: p.description.into(),
                        min: p.min,
                        max: p.max,
                        integer: p.integer,
                    })
                    .collect(),
                verification: s.verification.into(),
                allowed_here: !s.demo_only || is_demo,
            })
            .collect(),
    }))
}

#[utoipa::path(get, path = "/api/v1/projects/{project_id}/environments/{environment_id}/actions", tag = "actions", operation_id = "list_actions",
    params(("project_id" = String, Path), ("environment_id" = String, Path)),
    responses((status = 200, body = Vec<ActionPlanDto>), (status = 404, body = crate::error::ErrorBody)))]
pub async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id)): Path<(String, String)>,
) -> ApiResult<Json<Vec<ActionPlanDto>>> {
    let (scope, _) = authorize(&state, &user, &project_id, &environment_id, Permission::ActionsView).await?;
    Ok(Json(state.pg.list_action_plans(&scope, 100).await?.into_iter().map(Into::into).collect()))
}

#[utoipa::path(get, path = "/api/v1/projects/{project_id}/environments/{environment_id}/actions/{plan_id}", tag = "actions", operation_id = "get_action",
    params(("project_id" = String, Path), ("environment_id" = String, Path), ("plan_id" = String, Path)),
    responses((status = 200, body = ActionPlanDetail), (status = 404, body = crate::error::ErrorBody)))]
pub async fn detail(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id, id)): Path<(String, String, String)>,
) -> ApiResult<Json<ActionPlanDetail>> {
    let (scope, _) = authorize(&state, &user, &project_id, &environment_id, Permission::ActionsView).await?;
    let id = plan_id(&id)?;
    let row = state.pg.action_plan(&scope, id).await?.ok_or(ApiError::NotFound)?;
    let audit =
        state.pg.audit_entries_for_target(scope.tenant_id.as_str(), scope.project_id.as_str(), &id.to_string()).await?;
    Ok(Json(ActionPlanDetail { plan: row.into(), audit: audit.into_iter().map(Into::into).collect() }))
}

#[utoipa::path(post, path = "/api/v1/projects/{project_id}/environments/{environment_id}/actions", tag = "actions", operation_id = "propose_action",
    params(("project_id" = String, Path), ("environment_id" = String, Path)), request_body = ProposeActionRequest,
    responses((status = 200, body = ActionPlanDto), (status = 400, body = crate::error::ErrorBody),
        (status = 403, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)))]
pub async fn propose(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id)): Path<(String, String)>,
    Json(req): Json<ProposeActionRequest>,
) -> ApiResult<Json<ActionPlanDto>> {
    let (scope, is_demo) = authorize(&state, &user, &project_id, &environment_id, Permission::ActionsPropose).await?;
    user.session_id()?;
    let reason = actions::validate_reason(&req.reason).map_err(ApiError::BadRequest)?;
    let action = actions::validate(&req.action_type, &req.target, &req.params)
        .map_err(|e| ApiError::BadRequest(e.join("; ")))?;
    let decision = policy(&state, &scope, &action, is_demo, false).await?;
    let actor = user.user_id.to_string();
    let summary = json!({ "action_type": action.action_type(), "target": action.target(), "params": action.params() });
    if !decision.allowed {
        // Denials are audited too; if that write fails the request fails.
        state
            .pg
            .audit(NewAuditEntry {
                outcome: "denied",
                ..user_audit(
                    &scope,
                    &actor,
                    "action.propose",
                    Some(reason),
                    json!({ "proposal": summary, "policy": decision }),
                )
            })
            .await?;
        return Err(ApiError::PolicyDenied(denied_message(&decision)));
    }
    let policy_json = serde_json::to_value(&decision).map_err(|e| ApiError::Internal(e.to_string()))?;
    let id = state
        .pg
        .create_action_plan(
            NewActionPlan {
                scope: &scope,
                action_type: action.action_type(),
                target: action.target(),
                params: action.params(),
                reason,
                policy: policy_json.clone(),
                created_by: user.user_id,
                decision_window_secs: actions::DECISION_WINDOW_SECS,
            },
            user_audit(
                &scope,
                &actor,
                "action.propose",
                Some(reason),
                json!({ "proposal": summary, "policy": policy_json }),
            ),
        )
        .await?;
    let row = state.pg.action_plan(&scope, id).await?.ok_or(ApiError::NotFound)?;
    Ok(Json(row.into()))
}

#[utoipa::path(post, path = "/api/v1/projects/{project_id}/environments/{environment_id}/actions/{plan_id}/approve", tag = "actions", operation_id = "approve_action",
    params(("project_id" = String, Path), ("environment_id" = String, Path), ("plan_id" = String, Path)), request_body = DecisionRequest,
    responses((status = 200, body = ActionPlanDto), (status = 403, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody), (status = 409, body = crate::error::ErrorBody)))]
pub async fn approve(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id, id)): Path<(String, String, String)>,
    Json(req): Json<DecisionRequest>,
) -> ApiResult<Json<ActionPlanDto>> {
    let (scope, is_demo) = authorize(&state, &user, &project_id, &environment_id, Permission::ActionsApprove).await?;
    user.session_id()?;
    let comment = clean_comment(req.comment.as_deref())?;
    let row = state.pg.action_plan(&scope, plan_id(&id)?).await?.ok_or(ApiError::NotFound)?;
    if row.created_by == user.user_id {
        return Err(ApiError::Forbidden("a plan must be approved by someone other than its proposer"));
    }
    let action = stored_action(&row)?;
    let decision = policy(&state, &scope, &action, is_demo, true).await?;
    if !decision.allowed {
        return Err(ApiError::PolicyDenied(denied_message(&decision)));
    }
    let policy_json = serde_json::to_value(&decision).map_err(|e| ApiError::Internal(e.to_string()))?;
    let actor = user.user_id.to_string();
    let updated = state
        .pg
        .transition_action_plan(
            &scope,
            row.id,
            Transition {
                from: &["proposed"],
                to: "approved",
                decided_by: user.user_id,
                comment: comment.as_deref(),
                expires_in_secs: Some(actions::EXECUTION_WINDOW_SECS),
                policy: Some(policy_json.clone()),
            },
            user_audit(
                &scope,
                &actor,
                "action.approve",
                comment.as_deref(),
                json!({ "state": "approved", "policy": policy_json }),
            ),
        )
        .await?
        .ok_or(ApiError::Conflict("plan is not awaiting a decision (decided, cancelled or expired)"))?;
    Ok(Json(updated.into()))
}

#[utoipa::path(post, path = "/api/v1/projects/{project_id}/environments/{environment_id}/actions/{plan_id}/reject", tag = "actions", operation_id = "reject_action",
    params(("project_id" = String, Path), ("environment_id" = String, Path), ("plan_id" = String, Path)), request_body = DecisionRequest,
    responses((status = 200, body = ActionPlanDto), (status = 400, body = crate::error::ErrorBody), (status = 403, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody), (status = 409, body = crate::error::ErrorBody)))]
pub async fn reject(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id, id)): Path<(String, String, String)>,
    Json(req): Json<DecisionRequest>,
) -> ApiResult<Json<ActionPlanDto>> {
    let (scope, _) = authorize(&state, &user, &project_id, &environment_id, Permission::ActionsApprove).await?;
    user.session_id()?;
    let comment = clean_comment(req.comment.as_deref())?
        .ok_or_else(|| ApiError::BadRequest("comment is required when rejecting".into()))?;
    let id = plan_id(&id)?;
    state.pg.action_plan(&scope, id).await?.ok_or(ApiError::NotFound)?;
    let actor = user.user_id.to_string();
    let updated = state
        .pg
        .transition_action_plan(
            &scope,
            id,
            Transition {
                from: &["proposed"],
                to: "rejected",
                decided_by: user.user_id,
                comment: Some(&comment),
                expires_in_secs: None,
                policy: None,
            },
            user_audit(&scope, &actor, "action.reject", Some(&comment), json!({ "state": "rejected" })),
        )
        .await?
        .ok_or(ApiError::Conflict("plan is not awaiting a decision (decided, cancelled or expired)"))?;
    Ok(Json(updated.into()))
}

#[utoipa::path(post, path = "/api/v1/projects/{project_id}/environments/{environment_id}/actions/{plan_id}/cancel", tag = "actions", operation_id = "cancel_action",
    params(("project_id" = String, Path), ("environment_id" = String, Path), ("plan_id" = String, Path)), request_body = DecisionRequest,
    responses((status = 200, body = ActionPlanDto), (status = 403, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody), (status = 409, body = crate::error::ErrorBody)))]
pub async fn cancel(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id, id)): Path<(String, String, String)>,
    Json(req): Json<DecisionRequest>,
) -> ApiResult<Json<ActionPlanDto>> {
    let Authorized { scope, actor, .. } =
        authorize_as(&state, &user, &project_id, &environment_id, Permission::ActionsView).await?;
    if !actor.has(Permission::ActionsPropose) && !actor.has(Permission::ActionsApprove) {
        return Err(ApiError::Forbidden("insufficient permissions"));
    }
    user.session_id()?;
    let comment = clean_comment(req.comment.as_deref())?;
    let row = state.pg.action_plan(&scope, plan_id(&id)?).await?.ok_or(ApiError::NotFound)?;
    // Own plans need propose; someone else's plan needs approve.
    let need = if row.created_by == user.user_id { Permission::ActionsPropose } else { Permission::ActionsApprove };
    if !actor.has(need) {
        return Err(ApiError::Forbidden("insufficient permissions"));
    }
    let actor = user.user_id.to_string();
    let updated = state
        .pg
        .transition_action_plan(
            &scope,
            row.id,
            Transition {
                from: &["proposed", "approved"],
                to: "cancelled",
                decided_by: user.user_id,
                comment: comment.as_deref(),
                expires_in_secs: None,
                policy: None,
            },
            user_audit(&scope, &actor, "action.cancel", comment.as_deref(), json!({ "state": "cancelled" })),
        )
        .await?
        .ok_or(ApiError::Conflict("only proposed or approved plans can be cancelled"))?;
    Ok(Json(updated.into()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use karbird_domain::{EnvironmentId, ProjectId, TenantId};

    fn row(action_type: &str, target: &str, params: Value) -> ActionPlanRow {
        let t = OffsetDateTime::from_unix_timestamp(1_790_000_000).unwrap();
        ActionPlanRow {
            id: Uuid::nil(),
            tenant_id: "local".into(),
            project_id: "demo".into(),
            environment_id: "local-demo".into(),
            action_type: action_type.into(),
            target: target.into(),
            params,
            reason: "reason text".into(),
            state: "proposed".into(),
            policy: json!({ "allowed": true }),
            created_by: Uuid::nil(),
            created_by_email: "op@example.test".into(),
            created_at: t,
            expires_at: t + time::Duration::minutes(30),
            decided_by: None,
            decided_by_email: None,
            decided_at: None,
            decision_comment: None,
            executor_id: None,
            lease_until: None,
            started_at: None,
            finished_at: None,
            result: None,
            verification: None,
        }
    }

    fn bad_request<T: std::fmt::Debug>(r: ApiResult<T>) -> String {
        match r {
            Err(ApiError::BadRequest(m)) => m,
            other => panic!("expected 400, got {other:?}"),
        }
    }

    #[test]
    fn comments_are_optional_bounded_and_single_line() {
        assert_eq!(clean_comment(None).unwrap(), None);
        assert_eq!(clean_comment(Some("   ")).unwrap(), None, "blank means absent");
        assert_eq!(clean_comment(Some("  looks good  ")).unwrap().as_deref(), Some("looks good"));
        assert!(clean_comment(Some(&"x".repeat(500))).is_ok());
        assert!(clean_comment(Some(&"é".repeat(500))).is_ok(), "counted in characters");
        assert!(bad_request(clean_comment(Some(&"x".repeat(501)))).contains("500"));
        for c in ["a\nb", "a\rb", "tab\there", "\u{1b}[2J", "nul\0"] {
            assert!(clean_comment(Some(c)).is_err(), "{c:?}");
        }
    }

    #[test]
    fn plan_ids_must_be_uuids_and_unknown_ids_look_missing() {
        assert_eq!(plan_id("00000000-0000-0000-0000-000000000000").unwrap(), Uuid::nil());
        for bad in ["", "1", "../../tokens", "00000000-0000-0000-0000-00000000000g", "' OR 1=1"] {
            assert!(matches!(plan_id(bad), Err(ApiError::NotFound)), "{bad:?}");
        }
    }

    #[test]
    fn stored_plans_are_revalidated_before_approval() {
        assert!(stored_action(&row("demo.faults.clear", "orders", json!({}))).is_ok());
        for r in [
            row("shell.exec", "orders", json!({})),
            row("demo.faults.clear", "postgres", json!({})),
            row("demo.faults.inject", "orders", json!({ "latency_ms": 99_999, "error_rate": 0.1 })),
            row("demo.faults.clear", "orders", json!({ "cmd": "id" })),
        ] {
            assert!(
                matches!(stored_action(&r), Err(ApiError::Conflict("plan no longer passes validation"))),
                "{} {} {}",
                r.action_type,
                r.target,
                r.params
            );
        }
    }

    #[test]
    fn plan_dto_uses_catalog_titles_and_marks_unknown_types() {
        let d = ActionPlanDto::from(row("demo.faults.inject", "orders", json!({ "latency_ms": 5, "error_rate": 0.0 })));
        assert_eq!((d.title.as_str(), d.risk.as_str()), ("Inject faults", "medium"));
        assert_eq!(d.created_at, "2026-09-21T14:13:20Z");
        assert_eq!(d.expires_at, "2026-09-21T14:43:20Z");
        assert_eq!((d.decided_at, d.started_at, d.finished_at), (None, None, None));
        assert_eq!(d.created_by, "op@example.test");
        let d = ActionPlanDto::from(row("demo.faults.clear", "orders", json!({})));
        assert_eq!(d.risk, "low");
        let d = ActionPlanDto::from(row("retired.action", "x", json!({})));
        assert_eq!((d.title.as_str(), d.risk.as_str()), ("retired.action", "unknown"));
    }

    #[test]
    fn denial_messages_list_only_failed_checks() {
        let a = Action::DemoRestartDatabase;
        let d = actions::evaluate(
            &a,
            &PolicyInput { environment_is_demo: false, open_plans_for_target: 1, plans_last_hour: 0 },
            false,
        );
        let m = denied_message(&d);
        assert!(m.starts_with("demo environment: demo-only action"), "{m}");
        assert!(m.contains("; one open plan per target: 1 other open plan(s) for demo-postgres"), "{m}");
        assert!(!m.contains("allowlisted") && !m.contains("rate limit"), "{m}");
        let ok = actions::evaluate(
            &a,
            &PolicyInput { environment_is_demo: true, open_plans_for_target: 0, plans_last_hour: 0 },
            false,
        );
        assert_eq!(denied_message(&ok), "");
    }

    #[test]
    fn user_audit_entries_are_scoped_from_the_authorized_scope() {
        let scope = Scope {
            tenant_id: TenantId::parse("local").unwrap(),
            project_id: ProjectId::parse("demo").unwrap(),
            environment_id: EnvironmentId::parse("local-demo").unwrap(),
        };
        let e = user_audit(&scope, "user-1", "action.propose", Some("why"), json!({ "x": 1 }));
        assert_eq!((e.tenant_id, e.project_id), (Some("local"), Some("demo")));
        assert_eq!((e.actor_id, e.actor_kind, e.action, e.outcome), ("user-1", "user", "action.propose", "succeeded"));
        assert_eq!(e.reason, Some("why"));
        assert_eq!(e.target, None, "set by storage to the plan ID");
    }

    #[test]
    fn stored_policy_matches_the_documented_schema() {
        let d = actions::evaluate(
            &Action::DemoRestartDatabase,
            &PolicyInput { environment_is_demo: true, open_plans_for_target: 0, plans_last_hour: 0 },
            false,
        );
        let v = serde_json::to_value(&d).unwrap();
        let keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys.len(), 3);
        for k in ["allowed", "requires_approval", "checks"] {
            assert!(v.get(k).is_some(), "{k}");
        }
        for c in v["checks"].as_array().unwrap() {
            assert!(c["name"].is_string() && c["passed"].is_boolean() && c["detail"].is_string());
        }
    }

    #[test]
    fn risk_strings() {
        assert_eq!(risk_str(actions::Risk::Low), "low");
        assert_eq!(risk_str(actions::Risk::Medium), "medium");
    }
}
