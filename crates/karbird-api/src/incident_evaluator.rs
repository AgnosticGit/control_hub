//! Periodic evaluation of closed service windows. A transaction advisory lock ensures only one
//! API replica evaluates a batch at once; each rule transition is idempotent in PostgreSQL.

use std::collections::HashMap;
use std::time::Duration;

use karbird_domain::incidents::{MetricKind, Observation, RuleThreshold, classify};
use karbird_domain::{EnvironmentId, ProjectId, Scope, TenantId};
use karbird_storage::pg::WatermarkRow;

use crate::routes::incidents::closed_window_end_ns;
use crate::state::AppState;

const MINUTE_NS: i64 = 60_000_000_000;
const WINDOW_NS: i64 = 5 * MINUTE_NS;
const LOCK_KEY: i64 = 0x6b61726269726431; // "karbird1"

fn safe_to_evaluate(watermarks: &[WatermarkRow], window_end_ns: i64) -> bool {
    !watermarks.is_empty()
        && watermarks.iter().all(|w| {
            w.next_offset >= w.high_watermark && w.updated_at.unix_timestamp_nanos() >= i128::from(window_end_ns)
        })
}

async fn evaluate_once(state: &AppState) -> anyhow::Result<()> {
    let mut lock = state.pg.pool().begin().await?;
    let (acquired,): (bool,) =
        sqlx::query_as("SELECT pg_try_advisory_xact_lock($1)").bind(LOCK_KEY).fetch_one(&mut *lock).await?;
    if !acquired {
        return Ok(());
    }
    let now = time::OffsetDateTime::now_utc().unix_timestamp_nanos().clamp(0, i64::MAX as i128) as i64;
    let end = closed_window_end_ns(now);
    let watermarks = state.pg.watermarks("karbird-processor", "karbird.telemetry.v1").await?;
    if !safe_to_evaluate(&watermarks, end) {
        tracing::debug!("incident evaluation waits for processor watermark");
        return Ok(());
    }
    let rules = state.pg.due_incident_rules(end).await?;
    let mut summaries: HashMap<(String, String, String, i64), HashMap<String, karbird_storage::ch::ServiceSummaryRow>> =
        HashMap::new();
    let mut processing_errors: HashMap<String, bool> = HashMap::new();
    for rule in rules {
        let tenant = rule.tenant_id.clone();
        let has_errors = match processing_errors.get(&tenant) {
            Some(value) => *value,
            None => {
                let value = state.pg.count_unresolved_processing_errors(&tenant).await? > 0;
                processing_errors.insert(tenant.clone(), value);
                value
            }
        };
        if has_errors {
            tracing::debug!(tenant_id = %tenant, "incident evaluation waits for unresolved processing errors");
            continue;
        }
        let Some(kind) = MetricKind::parse(&rule.kind) else {
            tracing::error!(rule_id = %rule.id, "incident rule has invalid metric kind");
            continue;
        };
        // Catch up at most ten minutes after downtime. Older evidence expires, and
        // the storage transition resets a streak if this creates a gap.
        let first_end = rule.last_window_end_ns.saturating_add(MINUTE_NS).max(end - 9 * MINUTE_NS);
        let mut window_end = first_end;
        while window_end <= end {
            let key = (tenant.clone(), rule.project_id.clone(), rule.environment_id.clone(), window_end);
            if !summaries.contains_key(&key) {
                let scope = Scope {
                    tenant_id: TenantId::parse(&key.0)?,
                    project_id: ProjectId::parse(&key.1)?,
                    environment_id: EnvironmentId::parse(&key.2)?,
                };
                let rows = state.ch.service_summaries(&scope, window_end - WINDOW_NS, window_end).await?;
                summaries.insert(key.clone(), rows.into_iter().map(|row| (row.service_id.clone(), row)).collect());
            }
            let found = summaries.get(&key).and_then(|rows| rows.get(&rule.service_id));
            let observation = found.map_or(Observation { requests: 0, errors: 0, p95_ns: f64::NAN }, |row| {
                Observation { requests: row.requests, errors: row.errors, p95_ns: row.p95_ns }
            });
            let verdict = classify(
                RuleThreshold { kind, threshold: rule.threshold, min_requests: rule.min_requests as u64 },
                observation,
            );
            state.pg.apply_incident_observation(rule.id, window_end, observation, verdict).await?;
            window_end = window_end.saturating_add(MINUTE_NS);
        }
    }
    lock.commit().await?;
    Ok(())
}

pub async fn run(state: AppState) {
    let mut tick = tokio::time::interval(Duration::from_secs(15));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        if let Err(error) = evaluate_once(&state).await {
            tracing::warn!(%error, "incident evaluation failed; closed window will be retried");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_watermarks_cannot_be_treated_as_complete() {
        assert!(!safe_to_evaluate(&[], 100));
    }

    #[test]
    fn every_partition_must_have_a_fresh_caught_up_watermark() {
        let row = |partition, next_offset, high_watermark, updated_at| WatermarkRow {
            partition,
            next_offset,
            high_watermark,
            last_received_time_ns: 0,
            updated_at: time::OffsetDateTime::from_unix_timestamp(updated_at).unwrap(),
        };
        let idle = row(0, 2, 2, 10);
        let active = row(1, 10, 10, 30);
        assert!(!safe_to_evaluate(&[idle.clone(), active.clone()], 20_000_000_000));
        assert!(safe_to_evaluate(&[row(0, 2, 2, 30), active.clone()], 20_000_000_000));
        assert!(!safe_to_evaluate(&[idle.clone(), active], 40_000_000_000));
        assert!(!safe_to_evaluate(&[idle, row(1, 9, 10, 30)], 20_000_000_000));
    }
}
