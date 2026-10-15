//! A small, scoped metrics view for the selected service or PostgreSQL dependency.
use axum::Json;
use axum::extract::{Path, Query, State};
use karbird_storage::ch::{METRIC_KIND_CUMULATIVE_SUM, METRIC_KIND_DELTA_SUM, MetricWindowRow};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::auth::{CurrentUser, authorize};
use crate::error::{ApiError, ApiResult};
use crate::permissions::Permission;
use crate::routes::telemetry::{now_ns, parse_ns};
use crate::state::AppState;

const WINDOW_NS: i64 = 300_000_000_000;

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct MetricOverviewParams {
    pub name: String,
    pub at: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MetricOverviewResponse {
    pub at_ns: String,
    pub window_seconds: u32,
    pub http_samples: u64,
    pub http_p95_ms: Option<f64>,
    pub process_rss_bytes: Option<f64>,
    pub process_cpu_pct_of_core: Option<f64>,
    pub postgres_backends: Option<f64>,
    pub postgres_commits_per_second: Option<f64>,
    pub postgres_rollbacks_per_second: Option<f64>,
    pub postgres_deadlocks_per_second: Option<f64>,
}

#[utoipa::path(get, path = "/api/v1/projects/{project_id}/environments/{environment_id}/metrics/overview", tag = "telemetry",
    params(("project_id" = String, Path), ("environment_id" = String, Path), MetricOverviewParams),
    responses((status = 200, body = MetricOverviewResponse), (status = 400, body = crate::error::ErrorBody),
        (status = 401, body = crate::error::ErrorBody), (status = 403, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody)))]
pub async fn metric_overview(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id)): Path<(String, String)>,
    Query(q): Query<MetricOverviewParams>,
) -> ApiResult<Json<MetricOverviewResponse>> {
    let (scope, _) = authorize(&state, &user, &project_id, &environment_id, Permission::TelemetryView).await?;
    if q.name.is_empty() || q.name.len() > 128 || q.name.chars().any(char::is_control) {
        return Err(ApiError::BadRequest("name must be 1–128 printable characters".into()));
    }
    let at = parse_ns(q.at.as_deref(), "at")?.unwrap_or_else(now_ns);
    let rows = state.ch.metric_window(&scope, &q.name, at.saturating_sub(WINDOW_NS), at).await?;
    Ok(Json(summarize(&rows, at)))
}

fn amount(row: &MetricWindowRow) -> f64 {
    match row.kind {
        METRIC_KIND_DELTA_SUM => row.total_value,
        METRIC_KIND_CUMULATIVE_SUM if row.last_time_ns > row.first_time_ns && row.last_value >= row.first_value => {
            row.last_value - row.first_value
        }
        _ => 0.0,
    }
}

fn rate(row: &MetricWindowRow) -> Option<f64> {
    let seconds = (row.last_time_ns - row.first_time_ns) as f64 / 1e9;
    (seconds > 0.0).then(|| amount(row) / seconds)
}

fn summarize(rows: &[MetricWindowRow], at: i64) -> MetricOverviewResponse {
    let mut buckets = Vec::<(f64, f64)>::new();
    let mut rss = 0.0;
    let mut has_rss = false;
    let mut cpu = 0.0;
    let mut has_cpu = false;
    let mut backends = 0.0;
    let mut has_backends = false;
    let mut commits = 0.0;
    let mut has_commits = false;
    let mut rollbacks = 0.0;
    let mut has_rollbacks = false;
    let mut deadlocks = 0.0;
    let mut has_deadlocks = false;
    for row in rows {
        match row.metric_name.as_str() {
            "http.server.request.duration.bucket" => {
                let bound = if row.bucket_bound == "+Inf" {
                    f64::INFINITY
                } else {
                    row.bucket_bound.parse::<f64>().unwrap_or(f64::NAN)
                };
                let unit_scale = if row.unit == "s" { 1_000.0 } else { 1.0 };
                let count = amount(row);
                if !bound.is_nan() && count.is_finite() && count >= 0.0 {
                    buckets.push((bound * unit_scale, count));
                }
            }
            "process.memory.usage" if row.detail == "rss" => {
                rss += row.last_value;
                has_rss = true;
            }
            "process.cpu.time" => {
                if let Some(v) = rate(row) {
                    cpu += v * 100.0;
                    has_cpu = true;
                }
            }
            "postgresql.backends" => {
                backends += row.last_value;
                has_backends = true;
            }
            "postgresql.commits" => {
                if let Some(v) = rate(row) {
                    commits += v;
                    has_commits = true;
                }
            }
            "postgresql.rollbacks" => {
                if let Some(v) = rate(row) {
                    rollbacks += v;
                    has_rollbacks = true;
                }
            }
            "postgresql.deadlocks" => {
                if let Some(v) = rate(row) {
                    deadlocks += v;
                    has_deadlocks = true;
                }
            }
            _ => {}
        }
    }
    buckets.sort_by(|a, b| a.0.total_cmp(&b.0));
    let total: f64 = buckets.iter().map(|(_, count)| count).sum();
    let mut running = 0.0;
    let p95 = buckets.iter().find_map(|(bound, count)| {
        running += count;
        (total > 0.0 && running >= total * 0.95 && bound.is_finite()).then_some(*bound)
    });
    MetricOverviewResponse {
        at_ns: at.to_string(),
        window_seconds: 300,
        http_samples: total.max(0.0) as u64,
        http_p95_ms: p95,
        process_rss_bytes: has_rss.then_some(rss),
        process_cpu_pct_of_core: has_cpu.then_some(cpu),
        postgres_backends: has_backends.then_some(backends),
        postgres_commits_per_second: has_commits.then_some(commits),
        postgres_rollbacks_per_second: has_rollbacks.then_some(rollbacks),
        postgres_deadlocks_per_second: has_deadlocks.then_some(deadlocks),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row(name: &str, bound: &str, first: f64, last: f64, kind: u8) -> MetricWindowRow {
        MetricWindowRow {
            metric_name: name.into(),
            kind,
            unit: "s".into(),
            series_id: 1,
            bucket_bound: bound.into(),
            detail: String::new(),
            first_value: first,
            last_value: last,
            total_value: first + last,
            first_time_ns: 1_000_000_000,
            last_time_ns: 2_000_000_000,
        }
    }
    #[test]
    fn aggregates_histogram_and_dependency_without_inventing_missing_values() {
        let mut rss = row("process.memory.usage", "", 0.0, 1_048_576.0, 1);
        rss.detail = "rss".into();
        let summary = summarize(
            &[
                row("http.server.request.duration.bucket", "0.1", 1.0, 3.0, METRIC_KIND_CUMULATIVE_SUM),
                row("http.server.request.duration.bucket", "0.5", 1.0, 9.0, METRIC_KIND_CUMULATIVE_SUM),
                row("http.server.request.duration.bucket", "+Inf", 0.0, 0.0, METRIC_KIND_CUMULATIVE_SUM),
                rss,
                row("postgresql.backends", "", 2.0, 3.0, 1),
                row("postgresql.commits", "", 5.0, 15.0, METRIC_KIND_CUMULATIVE_SUM),
            ],
            3_000_000_000,
        );
        assert_eq!(summary.http_samples, 10);
        assert_eq!(summary.http_p95_ms, Some(500.0));
        assert_eq!(summary.process_rss_bytes, Some(1_048_576.0));
        assert_eq!(summary.postgres_backends, Some(3.0));
        assert_eq!(summary.postgres_commits_per_second, Some(10.0));
        assert_eq!(summary.process_cpu_pct_of_core, None);
    }
}
