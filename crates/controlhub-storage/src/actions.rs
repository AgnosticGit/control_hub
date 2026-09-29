//! Action plan persistence. Every state change is committed together with its audit entry:
//! if the audit write fails, the state change is rolled back (fail closed).

use serde_json::Value;
use sqlx::{FromRow, Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::Result;
use crate::pg::{NewAuditEntry, PgStore};
use controlhub_domain::Scope;

const PLAN_COLUMNS: &str =
    "p.id, p.tenant_id, p.project_id, p.environment_id, p.action_type, p.target, p.params, p.reason,
    p.state, p.policy, p.created_by, cu.email AS created_by_email, p.created_at, p.expires_at, p.decided_by,
    du.email AS decided_by_email, p.decided_at, p.decision_comment, p.executor_id, p.lease_until, p.started_at,
    p.finished_at, p.result, p.verification";
const PLAN_FROM: &str =
    "action_plans p JOIN users cu ON cu.id = p.created_by LEFT JOIN users du ON du.id = p.decided_by";

#[derive(Debug, Clone, FromRow)]
pub struct ActionPlanRow {
    pub id: Uuid,
    pub tenant_id: String,
    pub project_id: String,
    pub environment_id: String,
    pub action_type: String,
    pub target: String,
    pub params: Value,
    pub reason: String,
    pub state: String,
    pub policy: Value,
    pub created_by: Uuid,
    pub created_by_email: String,
    pub created_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
    pub decided_by: Option<Uuid>,
    pub decided_by_email: Option<String>,
    pub decided_at: Option<OffsetDateTime>,
    pub decision_comment: Option<String>,
    pub executor_id: Option<String>,
    pub lease_until: Option<OffsetDateTime>,
    pub started_at: Option<OffsetDateTime>,
    pub finished_at: Option<OffsetDateTime>,
    pub result: Option<Value>,
    pub verification: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct NewActionPlan<'a> {
    pub scope: &'a Scope,
    pub action_type: &'a str,
    pub target: &'a str,
    pub params: Value,
    pub reason: &'a str,
    pub policy: Value,
    pub created_by: Uuid,
    pub decision_window_secs: i64,
}

/// A state change requested through the API.
#[derive(Debug, Clone)]
pub struct Transition<'a> {
    pub from: &'a [&'a str],
    pub to: &'a str,
    pub decided_by: Uuid,
    pub comment: Option<&'a str>,
    /// New deadline (e.g. execution window after approval).
    pub expires_in_secs: Option<i64>,
    /// Re-evaluated policy to store with the plan.
    pub policy: Option<Value>,
}

async fn audit_tx(tx: &mut Transaction<'_, Postgres>, e: &NewAuditEntry<'_>) -> Result<()> {
    sqlx::query(
        "INSERT INTO audit_log (id, tenant_id, project_id, actor_id, actor_kind, action, target, reason, before, after, outcome, request_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(Uuid::now_v7())
    .bind(e.tenant_id)
    .bind(e.project_id)
    .bind(e.actor_id)
    .bind(e.actor_kind)
    .bind(e.action)
    .bind(e.target)
    .bind(e.reason)
    .bind(&e.before)
    .bind(&e.after)
    .bind(e.outcome)
    .bind(e.request_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn system_audit<'a>(row: &'a ActionPlanRow, actor: &'a str, action: &'a str, after: Value) -> NewAuditEntry<'a> {
    NewAuditEntry {
        tenant_id: Some(&row.tenant_id),
        project_id: Some(&row.project_id),
        actor_id: actor,
        actor_kind: "system",
        action,
        target: None,
        reason: None,
        before: None,
        after: Some(after),
        outcome: "succeeded",
        request_id: None,
    }
}

impl PgStore {
    /// Open plans for the target and plans created in the last hour, for the policy.
    pub async fn action_counts(&self, scope: &Scope, target: &str) -> Result<(i64, i64)> {
        let row: (i64, i64) = sqlx::query_as(
            "SELECT
               count(*) FILTER (WHERE target = $4 AND state IN ('proposed', 'approved', 'running')),
               count(*) FILTER (WHERE created_at > now() - interval '1 hour')
             FROM action_plans WHERE tenant_id = $1 AND project_id = $2 AND environment_id = $3",
        )
        .bind(scope.tenant_id.as_str())
        .bind(scope.project_id.as_str())
        .bind(scope.environment_id.as_str())
        .bind(target)
        .fetch_one(self.pool())
        .await?;
        Ok(row)
    }

    pub async fn create_action_plan(&self, p: NewActionPlan<'_>, audit: NewAuditEntry<'_>) -> Result<Uuid> {
        let id = Uuid::now_v7();
        let mut tx = self.pool().begin().await?;
        sqlx::query(
            "INSERT INTO action_plans (id, tenant_id, project_id, environment_id, action_type, target, params, reason, state, policy,
                                       created_by, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'proposed', $9, $10, now() + make_interval(secs => $11))",
        )
        .bind(id)
        .bind(p.scope.tenant_id.as_str())
        .bind(p.scope.project_id.as_str())
        .bind(p.scope.environment_id.as_str())
        .bind(p.action_type)
        .bind(p.target)
        .bind(&p.params)
        .bind(p.reason)
        .bind(&p.policy)
        .bind(p.created_by)
        .bind(p.decision_window_secs as f64)
        .execute(&mut *tx)
        .await?;
        let target = id.to_string();
        audit_tx(&mut tx, &NewAuditEntry { target: Some(&target), ..audit }).await?;
        tx.commit().await?;
        Ok(id)
    }

    pub async fn list_action_plans(&self, scope: &Scope, limit: i64) -> Result<Vec<ActionPlanRow>> {
        Ok(sqlx::query_as::<_, ActionPlanRow>(&format!(
            "SELECT {PLAN_COLUMNS} FROM {PLAN_FROM}
             WHERE p.tenant_id = $1 AND p.project_id = $2 AND p.environment_id = $3 ORDER BY p.created_at DESC LIMIT $4"
        ))
        .bind(scope.tenant_id.as_str())
        .bind(scope.project_id.as_str())
        .bind(scope.environment_id.as_str())
        .bind(limit)
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn action_plan(&self, scope: &Scope, id: Uuid) -> Result<Option<ActionPlanRow>> {
        Ok(sqlx::query_as::<_, ActionPlanRow>(&format!(
            "SELECT {PLAN_COLUMNS} FROM {PLAN_FROM}
             WHERE p.id = $1 AND p.tenant_id = $2 AND p.project_id = $3 AND p.environment_id = $4"
        ))
        .bind(id)
        .bind(scope.tenant_id.as_str())
        .bind(scope.project_id.as_str())
        .bind(scope.environment_id.as_str())
        .fetch_optional(self.pool())
        .await?)
    }

    async fn plan_by_id(&self, id: Uuid) -> Result<Option<ActionPlanRow>> {
        Ok(sqlx::query_as::<_, ActionPlanRow>(&format!("SELECT {PLAN_COLUMNS} FROM {PLAN_FROM} WHERE p.id = $1"))
            .bind(id)
            .fetch_optional(self.pool())
            .await?)
    }

    /// Applies a decision (approve/reject/cancel) if the plan is still in one of `from` and
    /// not past its deadline. Returns `None` when the transition is no longer possible.
    pub async fn transition_action_plan(
        &self,
        scope: &Scope,
        id: Uuid,
        t: Transition<'_>,
        audit: NewAuditEntry<'_>,
    ) -> Result<Option<ActionPlanRow>> {
        let mut tx = self.pool().begin().await?;
        let updated: Option<(Uuid,)> = sqlx::query_as(
            "UPDATE action_plans SET state = $6, decided_by = $7, decided_at = now(), decision_comment = $8,
                    expires_at = CASE WHEN $9::float8 IS NULL THEN expires_at ELSE now() + make_interval(secs => $9::float8) END,
                    policy = COALESCE($10, policy)
             WHERE id = $1 AND tenant_id = $2 AND project_id = $3 AND environment_id = $4
               AND state = ANY($5) AND expires_at > now()
             RETURNING id",
        )
        .bind(id)
        .bind(scope.tenant_id.as_str())
        .bind(scope.project_id.as_str())
        .bind(scope.environment_id.as_str())
        .bind(t.from)
        .bind(t.to)
        .bind(t.decided_by)
        .bind(t.comment)
        .bind(t.expires_in_secs.map(|s| s as f64))
        .bind(&t.policy)
        .fetch_optional(&mut *tx)
        .await?;
        if updated.is_none() {
            return Ok(None);
        }
        let target = id.to_string();
        audit_tx(&mut tx, &NewAuditEntry { target: Some(&target), ..audit }).await?;
        tx.commit().await?;
        self.plan_by_id(id).await
    }

    // ------------------------------------------------------------ executor

    /// Marks proposals and approvals past their deadline as expired.
    pub async fn expire_action_plans(&self, actor: &str) -> Result<u64> {
        let mut tx = self.pool().begin().await?;
        let ids: Vec<(Uuid,)> = sqlx::query_as(
            "UPDATE action_plans SET state = 'expired'
             WHERE state IN ('proposed', 'approved') AND expires_at <= now() RETURNING id",
        )
        .fetch_all(&mut *tx)
        .await?;
        for (id,) in &ids {
            let row =
                sqlx::query_as::<_, ActionPlanRow>(&format!("SELECT {PLAN_COLUMNS} FROM {PLAN_FROM} WHERE p.id = $1"))
                    .bind(id)
                    .fetch_one(&mut *tx)
                    .await?;
            let target = id.to_string();
            audit_tx(
                &mut tx,
                &NewAuditEntry {
                    target: Some(&target),
                    ..system_audit(&row, actor, "action.expire", serde_json::json!({ "state": "expired" }))
                },
            )
            .await?;
        }
        tx.commit().await?;
        Ok(ids.len() as u64)
    }

    /// Running plans whose executor lease ran out: the outcome is unknown, so they fail and
    /// are never retried automatically.
    pub async fn fail_lost_action_plans(&self, actor: &str) -> Result<u64> {
        let mut tx = self.pool().begin().await?;
        let ids: Vec<(Uuid,)> = sqlx::query_as(
            "UPDATE action_plans SET state = 'failed', finished_at = now(),
                    result = jsonb_build_object('error', 'executor lease expired; outcome unknown', 'executor_id', executor_id)
             WHERE state = 'running' AND lease_until < now() RETURNING id",
        )
        .fetch_all(&mut *tx)
        .await?;
        for (id,) in &ids {
            let row =
                sqlx::query_as::<_, ActionPlanRow>(&format!("SELECT {PLAN_COLUMNS} FROM {PLAN_FROM} WHERE p.id = $1"))
                    .bind(id)
                    .fetch_one(&mut *tx)
                    .await?;
            let target = id.to_string();
            let mut e = system_audit(&row, actor, "action.lost", serde_json::json!({ "state": "failed" }));
            e.outcome = "failed";
            audit_tx(&mut tx, &NewAuditEntry { target: Some(&target), ..e }).await?;
        }
        tx.commit().await?;
        Ok(ids.len() as u64)
    }

    /// Claims the oldest approved plan (SKIP LOCKED: safe with several executors).
    pub async fn claim_action_plan(&self, executor_id: &str, lease_secs: i64) -> Result<Option<ActionPlanRow>> {
        let mut tx = self.pool().begin().await?;
        let claimed: Option<(Uuid,)> = sqlx::query_as(
            "UPDATE action_plans SET state = 'running', executor_id = $1, started_at = now(),
                    lease_until = now() + make_interval(secs => $2)
             WHERE id = (SELECT id FROM action_plans WHERE state = 'approved' AND expires_at > now()
                         ORDER BY decided_at FOR UPDATE SKIP LOCKED LIMIT 1)
             RETURNING id",
        )
        .bind(executor_id)
        .bind(lease_secs as f64)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((id,)) = claimed else { return Ok(None) };
        let row =
            sqlx::query_as::<_, ActionPlanRow>(&format!("SELECT {PLAN_COLUMNS} FROM {PLAN_FROM} WHERE p.id = $1"))
                .bind(id)
                .fetch_one(&mut *tx)
                .await?;
        let target = id.to_string();
        audit_tx(
            &mut tx,
            &NewAuditEntry {
                target: Some(&target),
                ..system_audit(&row, executor_id, "action.start", serde_json::json!({ "state": "running" }))
            },
        )
        .await?;
        tx.commit().await?;
        Ok(Some(row))
    }

    pub async fn extend_action_lease(&self, id: Uuid, executor_id: &str, lease_secs: i64) -> Result<bool> {
        let r = sqlx::query(
            "UPDATE action_plans SET lease_until = now() + make_interval(secs => $3)
             WHERE id = $1 AND executor_id = $2 AND state = 'running'",
        )
        .bind(id)
        .bind(executor_id)
        .bind(lease_secs as f64)
        .execute(self.pool())
        .await?;
        Ok(r.rows_affected() == 1)
    }

    /// Records the final state. Only the executor holding the plan can finish it.
    pub async fn finish_action_plan(
        &self,
        id: Uuid,
        executor_id: &str,
        state: &str,
        result: &Value,
        verification: Option<&Value>,
    ) -> Result<bool> {
        let mut tx = self.pool().begin().await?;
        let row = sqlx::query_as::<_, ActionPlanRow>(&format!(
            "SELECT {PLAN_COLUMNS} FROM {PLAN_FROM} WHERE p.id = $1 AND p.executor_id = $2 AND p.state = 'running' FOR UPDATE OF p"
        ))
        .bind(id)
        .bind(executor_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else { return Ok(false) };
        sqlx::query(
            "UPDATE action_plans SET state = $2, finished_at = now(), lease_until = NULL, result = $3, verification = $4 WHERE id = $1",
        )
        .bind(id)
        .bind(state)
        .bind(result)
        .bind(verification)
        .execute(&mut *tx)
        .await?;
        let target = id.to_string();
        let mut e = system_audit(
            &row,
            executor_id,
            "action.finish",
            serde_json::json!({ "state": state, "result": result, "verification": verification }),
        );
        e.outcome = if state == "succeeded" { "succeeded" } else { "failed" };
        audit_tx(&mut tx, &NewAuditEntry { target: Some(&target), ..e }).await?;
        tx.commit().await?;
        Ok(true)
    }
}
