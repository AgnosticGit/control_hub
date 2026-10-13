//! Scoped PostgreSQL persistence for service incident rules and their history.

use karbird_domain::Scope;
use karbird_domain::incidents::{Observation, Streak, Transition, Verdict, advance, consecutive_streak};
use sqlx::{FromRow, Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::Result;
use crate::pg::PgStore;

#[derive(Debug, Clone, FromRow)]
pub struct IncidentRuleRow {
    pub id: Uuid,
    pub tenant_id: String,
    pub project_id: String,
    pub environment_id: String,
    pub service_id: String,
    pub kind: String,
    pub threshold: f64,
    pub min_requests: i32,
    pub enabled: bool,
    pub last_window_end_ns: i64,
    pub bad_streak: i32,
    pub good_streak: i32,
    pub observation_state: String,
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Clone, FromRow)]
pub struct IncidentRow {
    pub id: Uuid,
    pub rule_id: Uuid,
    pub service_id: String,
    pub kind: String,
    pub threshold: f64,
    pub state: String,
    pub opened_window_end_ns: i64,
    pub last_window_end_ns: i64,
    pub requests: i64,
    pub errors: i64,
    pub p95_ms: Option<f64>,
    pub acknowledged_by: Option<Uuid>,
    pub acknowledged_at: Option<OffsetDateTime>,
    pub recovered_at: Option<OffsetDateTime>,
    pub closed_reason: Option<String>,
    pub created_at: OffsetDateTime,
}

#[derive(Debug)]
pub struct NewRule<'a> {
    pub scope: &'a Scope,
    pub service_id: &'a str,
    pub kind: &'a str,
    pub threshold: f64,
    pub min_requests: i32,
    pub actor: Uuid,
    pub previous_window_end_ns: i64,
}

#[derive(Debug)]
pub struct RuleUpdate<'a> {
    pub scope: &'a Scope,
    pub id: Uuid,
    pub actor: Uuid,
    pub threshold: f64,
    pub min_requests: i32,
    pub enabled: bool,
    pub previous_window_end_ns: i64,
}

const RULE_COLUMNS: &str =
    "id, tenant_id, project_id, environment_id, service_id, kind, threshold, min_requests, enabled,
    last_window_end_ns, bad_streak, good_streak, observation_state, created_at";
const INCIDENT_COLUMNS: &str =
    "id, rule_id, service_id, kind, threshold, state, opened_window_end_ns, last_window_end_ns,
    requests, errors, p95_ms, acknowledged_by, acknowledged_at, recovered_at, closed_reason, created_at";

async fn audit(
    tx: &mut Transaction<'_, Postgres>,
    scope: &Scope,
    actor: &str,
    action: &str,
    target: Uuid,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO audit_log (id, tenant_id, project_id, actor_id, actor_kind, action, target, outcome)
         VALUES ($1, $2, $3, $4, 'user', $5, $6, 'succeeded')",
    )
    .bind(Uuid::now_v7())
    .bind(scope.tenant_id.as_str())
    .bind(scope.project_id.as_str())
    .bind(actor)
    .bind(action)
    .bind(target.to_string())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

impl PgStore {
    pub async fn incident_rules(&self, scope: &Scope) -> Result<Vec<IncidentRuleRow>> {
        let sql = format!(
            "SELECT {RULE_COLUMNS} FROM incident_rules
             WHERE tenant_id=$1 AND project_id=$2 AND environment_id=$3 ORDER BY created_at DESC LIMIT 200"
        );
        Ok(sqlx::query_as(&sql)
            .bind(scope.tenant_id.as_str())
            .bind(scope.project_id.as_str())
            .bind(scope.environment_id.as_str())
            .fetch_all(self.pool())
            .await?)
    }

    pub async fn incident_rule(&self, scope: &Scope, id: Uuid) -> Result<Option<IncidentRuleRow>> {
        let sql = format!(
            "SELECT {RULE_COLUMNS} FROM incident_rules WHERE tenant_id=$1 AND project_id=$2 AND environment_id=$3 AND id=$4"
        );
        Ok(sqlx::query_as(&sql)
            .bind(scope.tenant_id.as_str())
            .bind(scope.project_id.as_str())
            .bind(scope.environment_id.as_str())
            .bind(id)
            .fetch_optional(self.pool())
            .await?)
    }

    pub async fn create_incident_rule(&self, input: NewRule<'_>) -> Result<IncidentRuleRow> {
        let mut tx = self.pool().begin().await?;
        let id = Uuid::now_v7();
        let sql = format!(
            "INSERT INTO incident_rules
             (id, tenant_id, project_id, environment_id, service_id, kind, threshold, min_requests, created_by, last_window_end_ns)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) RETURNING {RULE_COLUMNS}"
        );
        let row = sqlx::query_as(&sql)
            .bind(id)
            .bind(input.scope.tenant_id.as_str())
            .bind(input.scope.project_id.as_str())
            .bind(input.scope.environment_id.as_str())
            .bind(input.service_id)
            .bind(input.kind)
            .bind(input.threshold)
            .bind(input.min_requests)
            .bind(input.actor)
            .bind(input.previous_window_end_ns)
            .fetch_one(&mut *tx)
            .await?;
        audit(&mut tx, input.scope, &input.actor.to_string(), "incident_rule.create", id).await?;
        tx.commit().await?;
        Ok(row)
    }

    pub async fn update_incident_rule(&self, input: RuleUpdate<'_>) -> Result<Option<IncidentRuleRow>> {
        let mut tx = self.pool().begin().await?;
        let sql = format!(
            "SELECT {RULE_COLUMNS} FROM incident_rules WHERE tenant_id=$1 AND project_id=$2 AND environment_id=$3 AND id=$4 FOR UPDATE"
        );
        let current: Option<IncidentRuleRow> = sqlx::query_as(&sql)
            .bind(input.scope.tenant_id.as_str())
            .bind(input.scope.project_id.as_str())
            .bind(input.scope.environment_id.as_str())
            .bind(input.id)
            .fetch_optional(&mut *tx)
            .await?;
        let Some(current) = current else { return Ok(None) };
        if current.threshold == input.threshold
            && current.min_requests == input.min_requests
            && current.enabled == input.enabled
        {
            return Ok(Some(current));
        }
        sqlx::query(
            "UPDATE incidents SET state='closed', closed_reason='rule_changed', recovered_at=now()
             WHERE rule_id=$1 AND state='open'",
        )
        .bind(input.id)
        .execute(&mut *tx)
        .await?;
        let sql = format!(
            "UPDATE incident_rules SET threshold=$2, min_requests=$3, enabled=$4, last_window_end_ns=$5,
             bad_streak=0, good_streak=0, observation_state='unknown', updated_at=now()
             WHERE id=$1 RETURNING {RULE_COLUMNS}"
        );
        let row = sqlx::query_as(&sql)
            .bind(input.id)
            .bind(input.threshold)
            .bind(input.min_requests)
            .bind(input.enabled)
            .bind(input.previous_window_end_ns)
            .fetch_one(&mut *tx)
            .await?;
        audit(&mut tx, input.scope, &input.actor.to_string(), "incident_rule.update", input.id).await?;
        tx.commit().await?;
        Ok(Some(row))
    }

    pub async fn due_incident_rules(&self, window_end_ns: i64) -> Result<Vec<IncidentRuleRow>> {
        let sql = format!(
            "SELECT {RULE_COLUMNS} FROM incident_rules WHERE enabled AND last_window_end_ns < $1
             ORDER BY last_window_end_ns, created_at LIMIT 100"
        );
        Ok(sqlx::query_as(&sql).bind(window_end_ns).fetch_all(self.pool()).await?)
    }

    pub async fn incidents(&self, scope: &Scope) -> Result<Vec<IncidentRow>> {
        let sql = format!(
            "SELECT {INCIDENT_COLUMNS} FROM incidents WHERE tenant_id=$1 AND project_id=$2 AND environment_id=$3
             ORDER BY (state='open') DESC, created_at DESC LIMIT 200"
        );
        Ok(sqlx::query_as(&sql)
            .bind(scope.tenant_id.as_str())
            .bind(scope.project_id.as_str())
            .bind(scope.environment_id.as_str())
            .fetch_all(self.pool())
            .await?)
    }

    pub async fn incident(&self, scope: &Scope, id: Uuid) -> Result<Option<IncidentRow>> {
        let sql = format!(
            "SELECT {INCIDENT_COLUMNS} FROM incidents WHERE tenant_id=$1 AND project_id=$2 AND environment_id=$3 AND id=$4"
        );
        Ok(sqlx::query_as(&sql)
            .bind(scope.tenant_id.as_str())
            .bind(scope.project_id.as_str())
            .bind(scope.environment_id.as_str())
            .bind(id)
            .fetch_optional(self.pool())
            .await?)
    }

    pub async fn acknowledge_incident(&self, scope: &Scope, id: Uuid, actor: Uuid) -> Result<Option<IncidentRow>> {
        let mut tx = self.pool().begin().await?;
        let sql = format!(
            "UPDATE incidents SET acknowledged_by=$5, acknowledged_at=now()
             WHERE tenant_id=$1 AND project_id=$2 AND environment_id=$3 AND id=$4 AND acknowledged_at IS NULL
             RETURNING {INCIDENT_COLUMNS}"
        );
        let changed: Option<IncidentRow> = sqlx::query_as(&sql)
            .bind(scope.tenant_id.as_str())
            .bind(scope.project_id.as_str())
            .bind(scope.environment_id.as_str())
            .bind(id)
            .bind(actor)
            .fetch_optional(&mut *tx)
            .await?;
        if changed.is_some() {
            audit(&mut tx, scope, &actor.to_string(), "incident.acknowledge", id).await?;
        }
        tx.commit().await?;
        if changed.is_some() { Ok(changed) } else { self.incident(scope, id).await }
    }

    /// Applies one closed-window observation atomically. A repeated evaluation of the same
    /// minute returns without another transition, even when several API replicas queried CH.
    pub async fn apply_incident_observation(
        &self,
        rule_id: Uuid,
        window_end_ns: i64,
        observation: Observation,
        verdict: Verdict,
    ) -> Result<()> {
        let mut tx = self.pool().begin().await?;
        let sql = format!("SELECT {RULE_COLUMNS} FROM incident_rules WHERE id=$1 FOR UPDATE");
        let rule: Option<IncidentRuleRow> = sqlx::query_as(&sql).bind(rule_id).fetch_optional(&mut *tx).await?;
        let Some(rule) = rule else { return Ok(()) };
        if !rule.enabled || rule.last_window_end_ns >= window_end_ns {
            return Ok(());
        }
        let open: Option<(Uuid,)> =
            sqlx::query_as("SELECT id FROM incidents WHERE rule_id=$1 AND state='open' FOR UPDATE")
                .bind(rule_id)
                .fetch_optional(&mut *tx)
                .await?;
        let previous = consecutive_streak(
            Streak { bad: rule.bad_streak, good: rule.good_streak },
            rule.last_window_end_ns,
            window_end_ns,
        );
        let decision = advance(previous, verdict, open.is_some());
        let observation_state = match verdict {
            Verdict::Bad => "breaching",
            Verdict::Good => "healthy",
            Verdict::Unknown => "unknown",
        };
        sqlx::query(
            "UPDATE incident_rules SET last_window_end_ns=$2, bad_streak=$3, good_streak=$4,
             observation_state=$5, updated_at=now() WHERE id=$1",
        )
        .bind(rule_id)
        .bind(window_end_ns)
        .bind(decision.streak.bad)
        .bind(decision.streak.good)
        .bind(observation_state)
        .execute(&mut *tx)
        .await?;
        let requests = observation.requests.min(i64::MAX as u64) as i64;
        let errors = observation.errors.min(i64::MAX as u64) as i64;
        let p95_ms =
            (observation.requests > 0 && observation.p95_ns.is_finite()).then_some(observation.p95_ns / 1_000_000.0);
        match decision.transition {
            Transition::Open => {
                let incident_id = Uuid::now_v7();
                sqlx::query(
                    "INSERT INTO incidents (id, rule_id, tenant_id, project_id, environment_id, service_id, kind,
                     threshold, state, opened_window_end_ns, last_window_end_ns, requests, errors, p95_ms)
                     VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'open',$9,$9,$10,$11,$12)",
                )
                .bind(incident_id)
                .bind(rule_id)
                .bind(&rule.tenant_id)
                .bind(&rule.project_id)
                .bind(&rule.environment_id)
                .bind(&rule.service_id)
                .bind(&rule.kind)
                .bind(rule.threshold)
                .bind(window_end_ns)
                .bind(requests)
                .bind(errors)
                .bind(p95_ms)
                .execute(&mut *tx)
                .await?;
                audit_evaluation(&mut tx, &rule, "incident.open", incident_id).await?;
            }
            Transition::Recover => {
                if let Some((id,)) = open {
                    sqlx::query(
                        "UPDATE incidents SET state='recovered', recovered_at=now(), last_window_end_ns=$2,
                         requests=$3, errors=$4, p95_ms=$5 WHERE id=$1",
                    )
                    .bind(id)
                    .bind(window_end_ns)
                    .bind(requests)
                    .bind(errors)
                    .bind(p95_ms)
                    .execute(&mut *tx)
                    .await?;
                    audit_evaluation(&mut tx, &rule, "incident.recover", id).await?;
                }
            }
            Transition::None => {
                if let Some((id,)) = open.filter(|_| verdict != Verdict::Unknown) {
                    sqlx::query(
                        "UPDATE incidents SET last_window_end_ns=$2, requests=$3, errors=$4, p95_ms=$5 WHERE id=$1",
                    )
                    .bind(id)
                    .bind(window_end_ns)
                    .bind(requests)
                    .bind(errors)
                    .bind(p95_ms)
                    .execute(&mut *tx)
                    .await?;
                }
            }
        }
        tx.commit().await?;
        Ok(())
    }
}

async fn audit_evaluation(
    tx: &mut Transaction<'_, Postgres>,
    rule: &IncidentRuleRow,
    action: &str,
    incident_id: Uuid,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO audit_log (id, tenant_id, project_id, actor_id, actor_kind, action, target, outcome)
         VALUES ($1,$2,$3,'incident-evaluator','system',$4,$5,'succeeded')",
    )
    .bind(Uuid::now_v7())
    .bind(&rule.tenant_id)
    .bind(&rule.project_id)
    .bind(action)
    .bind(incident_id.to_string())
    .execute(&mut **tx)
    .await?;
    Ok(())
}
