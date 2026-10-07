//! Data flowing over one map edge: calls and declared HTTP body bytes per time bucket, the
//! average throughput over the range and its peak bucket. Sizes come from the
//! `http.request.body.size` / `http.response.body.size` span attributes (Content-Length), so calls
//! that do not declare them are counted but carry no bytes; `sized_calls` says how many did.

use axum::Json;
use axum::extract::{Path, Query, State};
use karbird_storage::ch::{EdgeSpans, EdgeTrafficRow};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::auth::{CurrentUser, authorize};
use crate::error::{ApiError, ApiResult};
use crate::permissions::Permission;
use crate::routes::telemetry::{Range, RangeParams, external_clients_id};
use crate::state::AppState;

const NS_PER_SEC: f64 = 1e9;
const DEFAULT_BUCKETS: u32 = 60;
const MAX_BUCKETS: u32 = 600;
const MAX_ID_LEN: usize = 256;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EdgeRelation {
    Calls,
    DependsOn,
}

/// Edge fields are optional here and validated after authorization, so a caller without access
/// gets 404 whatever the query, never a 400 that confirms the environment exists.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct EdgeTrafficParams {
    /// Required. Node ID the edge leaves (a service, or the environment's external-clients node).
    pub source: Option<String>,
    /// Required. Node ID the edge enters (a service, database, queue or external host).
    pub target: Option<String>,
    /// Required. `calls` or `depends_on`.
    pub relation: Option<String>,
    /// Start, unix epoch nanoseconds as a decimal string. Default: `to - 15m`.
    pub from: Option<String>,
    /// End (exclusive), unix epoch nanoseconds as a decimal string. Default: now.
    pub to: Option<String>,
    /// Number of buckets (default 60, max 600).
    pub buckets: Option<u32>,
}

/// Calls and bytes over one edge. Byte rates are `null` when no call in the range declared a size.
#[derive(Debug, Serialize, ToSchema, PartialEq)]
pub struct EdgeTraffic {
    pub source: String,
    pub target: String,
    pub relation: EdgeRelation,
    pub from_ns: String,
    pub to_ns: String,
    pub buckets: u32,
    /// Bucket `i` covers `[from + i * bucket_ns, from + (i + 1) * bucket_ns)`, by call start.
    pub bucket_ns: String,
    pub calls: Vec<u64>,
    pub sized_calls: Vec<u64>,
    pub request_bytes: Vec<u64>,
    pub response_bytes: Vec<u64>,
    pub total_calls: u64,
    pub total_sized_calls: u64,
    pub total_request_sized_calls: u64,
    pub total_response_sized_calls: u64,
    pub total_request_bytes: u64,
    pub total_response_bytes: u64,
    /// `total_sized_calls / total_calls`; `null` without calls.
    pub size_coverage: Option<f64>,
    /// Bytes per second averaged over the whole range.
    pub avg_request_bps: Option<f64>,
    pub avg_response_bps: Option<f64>,
    /// Bytes per second in the busiest bucket and where it starts.
    pub peak_request_bps: Option<f64>,
    pub peak_request_at_ns: Option<String>,
    pub peak_response_bps: Option<f64>,
    pub peak_response_at_ns: Option<String>,
}

#[utoipa::path(get, path = "/api/v1/projects/{project_id}/environments/{environment_id}/edges/traffic", tag = "telemetry",
    params(("project_id" = String, Path), ("environment_id" = String, Path), EdgeTrafficParams),
    responses((status = 200, body = EdgeTraffic), (status = 400, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody)))]
pub async fn edge_traffic(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id)): Path<(String, String)>,
    Query(q): Query<EdgeTrafficParams>,
) -> ApiResult<Json<EdgeTraffic>> {
    let (scope, _) = authorize(&state, &user, &project_id, &environment_id, Permission::TelemetryView).await?;
    let r = RangeParams { from: q.from.clone(), to: q.to.clone() }.resolve()?;
    let edge = Edge::parse(&q)?;
    let spans = edge.spans(&external_clients_id(&scope));
    let g = Geometry::new(r, q.buckets);
    let rows = state.ch.edge_traffic(&scope, &spans, r.from, r.to, g.bucket_ns, g.buckets + 1).await?;
    Ok(Json(build(&edge, &g, &rows)))
}

/// A validated edge from the query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    pub source: String,
    pub target: String,
    pub relation: EdgeRelation,
}

impl Edge {
    /// Rejects missing, oversized or impossible edges.
    pub fn parse(q: &EdgeTrafficParams) -> ApiResult<Self> {
        let id = |name: &str, v: &Option<String>| -> ApiResult<String> {
            match v.as_deref() {
                Some(s) if !s.is_empty() && s.len() <= MAX_ID_LEN => Ok(s.to_owned()),
                _ => Err(ApiError::BadRequest(format!("{name} must be a node ID of 1..={MAX_ID_LEN} bytes"))),
            }
        };
        let (source, target) = (id("source", &q.source)?, id("target", &q.target)?);
        if source == target {
            return Err(ApiError::BadRequest("source and target must differ".into()));
        }
        let relation = match q.relation.as_deref() {
            Some("calls") => EdgeRelation::Calls,
            Some("depends_on") => EdgeRelation::DependsOn,
            _ => return Err(ApiError::BadRequest("relation must be calls or depends_on".into())),
        };
        Ok(Self { source, target, relation })
    }

    /// Which spans make up the edge (see [`EdgeSpans`]).
    pub fn spans(&self, clients_id: &str) -> EdgeSpans {
        let (source, target) = (self.source.clone(), self.target.clone());
        match self.relation {
            EdgeRelation::DependsOn => EdgeSpans::Dependency { source, target },
            EdgeRelation::Calls if source == clients_id => EdgeSpans::ExternalClients { target },
            EdgeRelation::Calls => EdgeSpans::Call { source, target },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    pub from: i64,
    pub to: i64,
    pub buckets: u32,
    pub bucket_ns: i64,
}

impl Geometry {
    pub fn new(r: Range, buckets: Option<u32>) -> Self {
        let buckets = buckets.unwrap_or(DEFAULT_BUCKETS).clamp(1, MAX_BUCKETS);
        let span = r.to.saturating_sub(r.from).max(1);
        let bucket_ns = (span + i64::from(buckets) - 1) / i64::from(buckets);
        Self { from: r.from, to: r.to, buckets, bucket_ns }
    }

    fn start(&self, i: usize) -> i64 {
        self.from.saturating_add(self.bucket_ns.saturating_mul(i as i64))
    }

    /// Length of bucket `i`; the last one may end early at `to`.
    fn len_ns(&self, i: usize) -> i64 {
        let start = self.start(i);
        self.bucket_ns.min(self.to.saturating_sub(start)).max(1)
    }
}

pub fn build(q: &Edge, g: &Geometry, rows: &[EdgeTrafficRow]) -> EdgeTraffic {
    let n = g.buckets as usize;
    let (mut calls, mut sized, mut req, mut resp) = (vec![0u64; n], vec![0u64; n], vec![0u64; n], vec![0u64; n]);
    for r in rows {
        let Some(i) = usize::try_from(r.bucket).ok().filter(|&i| i < n) else { continue };
        for (slot, v) in [
            (&mut calls, r.calls),
            (&mut sized, r.sized_calls),
            (&mut req, r.request_bytes),
            (&mut resp, r.response_bytes),
        ] {
            if let Some(s) = slot.get_mut(i) {
                *s = s.saturating_add(v);
            }
        }
    }
    let sum = |v: &[u64]| v.iter().fold(0u64, |a, &b| a.saturating_add(b));
    let (total_calls, total_sized) = (sum(&calls), sum(&sized));
    let (total_request_sized, total_response_sized) = rows.iter().fold((0u64, 0u64), |(req, resp), row| {
        (req.saturating_add(row.request_sized_calls), resp.saturating_add(row.response_sized_calls))
    });
    let (total_req, total_resp) = (sum(&req), sum(&resp));
    let range_s = (g.to.saturating_sub(g.from).max(1)) as f64 / NS_PER_SEC;
    let peak = |v: &[u64], known: bool| -> Option<(f64, String)> {
        if !known {
            return None;
        }
        v.iter()
            .enumerate()
            .map(|(i, &b)| (b as f64 * NS_PER_SEC / g.len_ns(i) as f64, i))
            .max_by(|a, b| a.0.total_cmp(&b.0).then(b.1.cmp(&a.1)))
            .map(|(bps, i)| (bps, g.start(i).to_string()))
    };
    let (peak_req, peak_resp) = (peak(&req, total_request_sized > 0), peak(&resp, total_response_sized > 0));
    EdgeTraffic {
        source: q.source.clone(),
        target: q.target.clone(),
        relation: q.relation,
        from_ns: g.from.to_string(),
        to_ns: g.to.to_string(),
        buckets: g.buckets,
        bucket_ns: g.bucket_ns.to_string(),
        total_calls,
        total_sized_calls: total_sized,
        total_request_sized_calls: total_request_sized,
        total_response_sized_calls: total_response_sized,
        total_request_bytes: total_req,
        total_response_bytes: total_resp,
        size_coverage: (total_calls > 0).then(|| total_sized as f64 / total_calls as f64),
        avg_request_bps: (total_request_sized > 0).then(|| total_req as f64 / range_s),
        avg_response_bps: (total_response_sized > 0).then(|| total_resp as f64 / range_s),
        peak_request_bps: peak_req.as_ref().map(|p| p.0),
        peak_request_at_ns: peak_req.map(|p| p.1),
        peak_response_bps: peak_resp.as_ref().map(|p| p.0),
        peak_response_at_ns: peak_resp.map(|p| p.1),
        calls,
        sized_calls: sized,
        request_bytes: req,
        response_bytes: resp,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    const SEC: i64 = 1_000_000_000;

    fn query(source: Option<&str>, target: Option<&str>, relation: Option<&str>) -> EdgeTrafficParams {
        EdgeTrafficParams {
            source: source.map(Into::into),
            target: target.map(Into::into),
            relation: relation.map(Into::into),
            from: None,
            to: None,
            buckets: None,
        }
    }

    fn params(source: &str, target: &str, relation: EdgeRelation) -> Edge {
        Edge { source: source.into(), target: target.into(), relation }
    }

    fn row(bucket: i64, calls: u64, sized: u64, req: u64, resp: u64) -> EdgeTrafficRow {
        EdgeTrafficRow {
            bucket,
            calls,
            sized_calls: sized,
            request_sized_calls: sized,
            response_sized_calls: sized,
            request_bytes: req,
            response_bytes: resp,
        }
    }

    #[test]
    fn edge_kind_follows_relation_and_the_external_clients_node() {
        let spans =
            |s: &str, t: &str, r: &str| Edge::parse(&query(Some(s), Some(t), Some(r))).unwrap().spans("clients");
        assert_eq!(
            spans("svc_a", "svc_b", "calls"),
            EdgeSpans::Call { source: "svc_a".into(), target: "svc_b".into() }
        );
        assert_eq!(
            spans("svc_a", "db", "depends_on"),
            EdgeSpans::Dependency { source: "svc_a".into(), target: "db".into() }
        );
        assert_eq!(spans("clients", "svc_b", "calls"), EdgeSpans::ExternalClients { target: "svc_b".into() });
    }

    #[test]
    fn malformed_edges_are_rejected() {
        let long = "x".repeat(MAX_ID_LEN + 1);
        let cases = [
            query(Some(""), Some("b"), Some("calls")),
            query(Some("a"), Some(""), Some("calls")),
            query(Some("a"), Some("a"), Some("calls")),
            query(Some(&long), Some("b"), Some("calls")),
            query(Some("a"), Some(&long), Some("calls")),
            query(None, Some("b"), Some("calls")),
            query(Some("a"), None, Some("calls")),
            query(Some("a"), Some("b"), None),
            query(Some("a"), Some("b"), Some("owns")),
            query(Some("a"), Some("b"), Some("CALLS")),
        ];
        for q in cases {
            let err = Edge::parse(&q).unwrap_err();
            assert!(matches!(err, ApiError::BadRequest(_)), "{q:?}");
        }
    }

    #[test]
    fn a_query_without_edge_fields_still_reaches_authorization() {
        // Extraction must accept an empty query; only `Edge::parse`, after `authorize`, rejects it.
        let parse = |uri: &str| Query::<EdgeTrafficParams>::try_from_uri(&uri.parse().unwrap()).unwrap().0;
        let q = parse("/edges/traffic");
        assert_eq!((q.source, q.target, q.relation), (None, None, None));
        let q = parse("/edges/traffic?relation=owns&source=a");
        assert_eq!(q.relation.as_deref(), Some("owns"));
    }

    #[test]
    fn averages_and_peaks_over_the_range() {
        // 60 s in 6 buckets of 10 s.
        let g = Geometry::new(Range { from: 100 * SEC, to: 160 * SEC }, Some(6));
        let q = params("a", "b", EdgeRelation::Calls);
        let t = build(&q, &g, &[row(0, 10, 10, 10_000, 50_000), row(3, 4, 2, 40_000, 2_000)]);
        assert_eq!((t.buckets, t.bucket_ns.as_str()), (6, "10000000000"));
        assert_eq!(t.calls, vec![10, 0, 0, 4, 0, 0]);
        assert_eq!((t.total_calls, t.total_sized_calls), (14, 12));
        assert_eq!((t.total_request_bytes, t.total_response_bytes), (50_000, 52_000));
        assert!((t.size_coverage.unwrap() - 12.0 / 14.0).abs() < 1e-12);
        assert!((t.avg_request_bps.unwrap() - 50_000.0 / 60.0).abs() < 1e-9);
        assert!((t.avg_response_bps.unwrap() - 52_000.0 / 60.0).abs() < 1e-9);
        assert_eq!(t.peak_request_bps, Some(4_000.0));
        assert_eq!(t.peak_request_at_ns.as_deref(), Some("130000000000"));
        assert_eq!(t.peak_response_bps, Some(5_000.0));
        assert_eq!(t.peak_response_at_ns.as_deref(), Some("100000000000"));
    }

    #[test]
    fn without_declared_sizes_rates_are_unknown_not_zero() {
        let g = Geometry::new(Range { from: 0, to: 60 * SEC }, Some(6));
        let t = build(&params("a", "db", EdgeRelation::DependsOn), &g, &[row(1, 7, 0, 0, 0)]);
        assert_eq!((t.total_calls, t.size_coverage), (7, Some(0.0)));
        assert_eq!((t.avg_request_bps, t.peak_request_bps, t.peak_request_at_ns), (None, None, None));
        let empty = build(&params("a", "db", EdgeRelation::DependsOn), &g, &[]);
        assert_eq!((empty.total_calls, empty.size_coverage, empty.avg_response_bps), (0, None, None));
        assert_eq!(empty.calls, vec![0; 6]);
    }

    #[test]
    fn an_unreported_request_size_is_not_reported_as_zero_when_only_response_size_is_known() {
        let g = Geometry::new(Range { from: 0, to: 60 * SEC }, Some(6));
        let t = build(
            &params("a", "b", EdgeRelation::Calls),
            &g,
            &[EdgeTrafficRow {
                bucket: 0,
                calls: 7,
                sized_calls: 7,
                request_sized_calls: 0,
                response_sized_calls: 7,
                request_bytes: 0,
                response_bytes: 350,
            }],
        );
        assert_eq!((t.avg_request_bps, t.peak_request_bps, t.peak_request_at_ns), (None, None, None));
        assert_eq!(t.total_response_sized_calls, 7);
        assert_eq!(t.avg_response_bps, Some(350.0 / 60.0));
        assert!(t.peak_response_bps.is_some_and(|rate| rate > 0.0));
    }

    #[test]
    fn the_short_last_bucket_is_rated_by_its_real_length_and_stray_rows_are_ignored() {
        // 25 s in 3 buckets of ⌈25/3⌉ s: the last one is 2 ns shorter.
        let g = Geometry::new(Range { from: 0, to: 25 * SEC }, Some(3));
        assert_eq!(g.bucket_ns, 8_333_333_334);
        let t = build(
            &params("a", "b", EdgeRelation::Calls),
            &g,
            &[row(2, 1, 1, 7_000, 0), row(-1, 5, 5, 1, 1), row(3, 5, 5, 1, 1)],
        );
        assert_eq!(t.total_calls, 1);
        let last_len = 25 * SEC - 2 * 8_333_333_334;
        assert_eq!(t.peak_request_bps, Some(7_000.0 * 1e9 / last_len as f64));
        assert_eq!(t.peak_request_at_ns.as_deref(), Some("16666666668"));
    }

    #[test]
    fn bucket_count_is_clamped() {
        let r = Range { from: 0, to: 60 * SEC };
        assert_eq!(Geometry::new(r, Some(0)).buckets, 1);
        assert_eq!(Geometry::new(r, Some(100_000)).buckets, MAX_BUCKETS);
        assert_eq!(Geometry::new(r, None).buckets, DEFAULT_BUCKETS);
    }
}
