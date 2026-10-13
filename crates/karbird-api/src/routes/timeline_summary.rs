//! Bounded, scoped timeline summaries for month-scale views.

use axum::Json;
use axum::extract::{Path, Query, State};
use karbird_storage::ch::{TimelineServiceBucketRow, TimelineSignalBucketRow};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::{IntoParams, ToSchema};

use crate::auth::{CurrentUser, authorize};
use crate::error::{ApiError, ApiResult};
use crate::permissions::Permission;
use crate::routes::telemetry::{Range, now_ns, parse_ns};
use crate::state::AppState;

const NS_PER_DAY: i64 = 86_400_000_000_000;
pub const MAX_SUMMARY_RANGE_NS: i64 = 30 * NS_PER_DAY;
pub const DEFAULT_SUMMARY_BUCKETS: u32 = 1_000;
pub const MAX_SUMMARY_BUCKETS: u32 = 1_000;
const MAX_SUMMARY_ROWS: u32 = 200_000;

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct TimelineSummaryParams {
    /// Start, unix epoch nanoseconds as a decimal string. Defaults to 30 days before `to`.
    pub from: Option<String>,
    /// End (exclusive), unix epoch nanoseconds as a decimal string. Defaults to now.
    pub to: Option<String>,
    /// Number of summary buckets (default 1000, max 1000).
    pub buckets: Option<u32>,
}

impl TimelineSummaryParams {
    pub fn resolve(&self) -> ApiResult<Range> {
        let to = parse_ns(self.to.as_deref(), "to")?.unwrap_or_else(now_ns);
        let from = parse_ns(self.from.as_deref(), "from")?.unwrap_or(to.saturating_sub(MAX_SUMMARY_RANGE_NS));
        if from >= to {
            return Err(ApiError::BadRequest("from must be before to".into()));
        }
        if to.checked_sub(from).is_none_or(|span| span > MAX_SUMMARY_RANGE_NS) {
            return Err(ApiError::BadRequest("summary time range may not exceed 30 days".into()));
        }
        Ok(Range { from, to })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SummaryGeometry {
    pub from: i64,
    pub to: i64,
    pub buckets: u32,
    pub bucket_ns: i64,
}

impl SummaryGeometry {
    pub fn new(from: i64, to: i64, buckets: Option<u32>) -> Self {
        let buckets = buckets.unwrap_or(DEFAULT_SUMMARY_BUCKETS).clamp(1, MAX_SUMMARY_BUCKETS);
        let span = to.saturating_sub(from).max(1);
        let bucket_ns = span.saturating_add(i64::from(buckets) - 1) / i64::from(buckets);
        Self { from, to, buckets, bucket_ns }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SummarySignalBucket {
    pub requests: u64,
    pub errors: u64,
    pub p95_ms: Option<f64>,
    pub error_logs: u64,
    pub events: u64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SummaryServiceTrack {
    pub service_id: String,
    pub service_name: String,
    pub spans: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SummaryHostTrack {
    pub host_id: String,
    pub host_name: String,
    pub cpu: Vec<Option<f32>>,
    pub memory: Vec<Option<f32>>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TimelineSummary {
    /// Unix epoch nanoseconds as decimal strings.
    pub from_ns: String,
    /// Exclusive end, Unix epoch nanoseconds as a decimal string.
    pub to_ns: String,
    /// Oldest retained timeline signal for this authorized scope, if any.
    pub oldest_ns: Option<String>,
    pub buckets: u32,
    /// Width of one bucket in nanoseconds as a decimal string.
    pub bucket_ns: String,
    pub signals: Vec<SummarySignalBucket>,
    pub services: Vec<SummaryServiceTrack>,
    pub hosts: Vec<SummaryHostTrack>,
}

#[utoipa::path(get, path = "/api/v1/projects/{project_id}/environments/{environment_id}/timeline/summary", tag = "timeline",
    params(("project_id" = String, Path), ("environment_id" = String, Path), TimelineSummaryParams),
    responses((status = 200, body = TimelineSummary), (status = 400, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody))) ]
pub async fn timeline_summary(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id)): Path<(String, String)>,
    Query(q): Query<TimelineSummaryParams>,
) -> ApiResult<Json<TimelineSummary>> {
    let (scope, _) = authorize(&state, &user, &project_id, &environment_id, Permission::TelemetryView).await?;
    let range = q.resolve()?;
    let geometry = SummaryGeometry::new(range.from, range.to, q.buckets);
    let (signals, error_logs, events, services) = tokio::try_join!(
        state.ch.timeline_signal_buckets(&scope, geometry.from, geometry.to, geometry.bucket_ns, geometry.buckets,),
        state.ch.timeline_error_log_buckets(&scope, geometry.from, geometry.to, geometry.bucket_ns, geometry.buckets,),
        state.ch.timeline_event_buckets(&scope, geometry.from, geometry.to, geometry.bucket_ns, geometry.buckets,),
        state.ch.timeline_service_buckets(&scope, geometry.from, geometry.to, geometry.bucket_ns, MAX_SUMMARY_ROWS,),
    )?;
    let infra_geometry = super::infra_timeline::Geometry::new(range, Some(geometry.buckets));
    let infra_rows =
        state.ch.infra_buckets(&scope, infra_geometry.base(), range.to, geometry.bucket_ns, MAX_SUMMARY_ROWS).await?;
    let infrastructure =
        super::infra_timeline::build(&infra_geometry, &infra_rows, infra_rows.len() >= MAX_SUMMARY_ROWS as usize);

    let oldest_ns = super::telemetry::earliest_timeline_time_ns(&state, &scope).await?.map(|time| time.to_string());
    let mut summary = build_summary(geometry, signals, error_logs, events, services, infrastructure.hosts);
    summary.oldest_ns = oldest_ns;
    Ok(Json(summary))
}

fn build_summary(
    geometry: SummaryGeometry,
    signals: Vec<TimelineSignalBucketRow>,
    error_logs: Vec<karbird_storage::ch::TimelineCountBucketRow>,
    events: Vec<karbird_storage::ch::TimelineCountBucketRow>,
    services: Vec<TimelineServiceBucketRow>,
    hosts: Vec<super::infra_timeline::HostTrack>,
) -> TimelineSummary {
    let mut signal_buckets = vec![
        SummarySignalBucket { requests: 0, errors: 0, p95_ms: None, error_logs: 0, events: 0 };
        geometry.buckets as usize
    ];
    for TimelineSignalBucketRow { bucket, requests, errors, p95_ns } in signals {
        if let Ok(index) = usize::try_from(bucket)
            && let Some(item) = signal_buckets.get_mut(index)
        {
            item.requests = requests;
            item.errors = errors;
            item.p95_ms = (requests > 0).then(|| p95_ns / 1_000_000.0);
        }
    }
    for row in error_logs {
        if let Ok(index) = usize::try_from(row.bucket)
            && let Some(item) = signal_buckets.get_mut(index)
        {
            item.error_logs = row.count;
        }
    }
    for row in events {
        if let Ok(index) = usize::try_from(row.bucket)
            && let Some(item) = signal_buckets.get_mut(index)
        {
            item.events = row.count;
        }
    }

    let mut service_buckets = HashMap::<String, SummaryServiceTrack>::new();
    for row in services {
        let bucket_index = usize::try_from(row.bucket);
        let Some(index) = bucket_index.ok().filter(|index| *index < geometry.buckets as usize) else { continue };
        let track = service_buckets.entry(row.service_id.clone()).or_insert_with(|| SummaryServiceTrack {
            service_id: row.service_id,
            service_name: row.service_name,
            spans: vec![0; geometry.buckets as usize],
        });
        track.spans[index] = row.count;
    }
    let mut service_tracks = service_buckets.into_values().collect::<Vec<_>>();
    service_tracks.sort_by(|left, right| left.service_name.cmp(&right.service_name));

    TimelineSummary {
        from_ns: geometry.from.to_string(),
        to_ns: geometry.to.to_string(),
        oldest_ns: None,
        buckets: geometry.buckets,
        bucket_ns: geometry.bucket_ns.to_string(),
        signals: signal_buckets,
        services: service_tracks,
        hosts: hosts
            .into_iter()
            .map(|host| SummaryHostTrack { host_id: host.id, host_name: host.name, cpu: host.cpu, memory: host.memory })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_range_accepts_thirty_days_and_rejects_longer_ranges() {
        let to = 1_800_000_000_000_000_000_i64;
        let params = TimelineSummaryParams {
            from: Some((to - MAX_SUMMARY_RANGE_NS).to_string()),
            to: Some(to.to_string()),
            buckets: None,
        };
        assert_eq!(params.resolve().unwrap().to, to);

        let too_wide = TimelineSummaryParams {
            from: Some((to - MAX_SUMMARY_RANGE_NS - 1).to_string()),
            to: Some(to.to_string()),
            buckets: None,
        };
        assert!(matches!(too_wide.resolve(), Err(ApiError::BadRequest(_))));
    }

    #[test]
    fn summary_bucket_geometry_is_bounded_and_covers_the_requested_span() {
        let geometry = SummaryGeometry::new(1_000, 1_000 + 30 * NS_PER_DAY, Some(u32::MAX));
        assert_eq!(geometry.buckets, MAX_SUMMARY_BUCKETS);
        assert!(geometry.bucket_ns > 0);
        assert!(geometry.bucket_ns * i64::from(geometry.buckets) >= geometry.to - geometry.from);
    }

    #[test]
    fn summary_builder_keeps_sparse_buckets_and_decimal_nanosecond_boundaries() {
        let geometry = SummaryGeometry::new(1_000_000_000, 1_000_000_020, Some(2));
        let summary = build_summary(
            geometry,
            vec![TimelineSignalBucketRow { bucket: 1, requests: 4, errors: 1, p95_ns: 2_000_000.0 }],
            vec![karbird_storage::ch::TimelineCountBucketRow { bucket: 1, count: 2 }],
            vec![karbird_storage::ch::TimelineCountBucketRow { bucket: 0, count: 3 }],
            vec![TimelineServiceBucketRow {
                bucket: 1,
                service_id: "svc-1".into(),
                service_name: "orders".into(),
                count: 8,
            }],
            Vec::new(),
        );

        assert_eq!(summary.from_ns, "1000000000");
        assert_eq!(summary.to_ns, "1000000020");
        assert_eq!(summary.bucket_ns, "10");
        assert_eq!(summary.signals[0].events, 3);
        assert_eq!(summary.signals[0].requests, 0);
        assert_eq!(summary.signals[1].requests, 4);
        assert_eq!(summary.signals[1].errors, 1);
        assert_eq!(summary.signals[1].p95_ms, Some(2.0));
        assert_eq!(summary.signals[1].error_logs, 2);
        assert_eq!(summary.services[0].spans, [0, 8]);
    }
}
