//! Engine input/output contracts. Inputs mirror the REST timeline/topology
//! responses (snake_case JSON, nanosecond timestamps as decimal strings) so
//! the Worker can hand raw response text straight to the engine.

use controlhub_domain::time::{nanos_string, opt_nanos_string};
use controlhub_domain::{EntityKind, HealthStatus, Provenance, RelationKind};
use serde::{Deserialize, Serialize};

#[cfg(feature = "ts")]
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(default)]
pub struct EngineConfig {
    /// Hard cap on stored timeline items (spans + logs + events). Oldest are evicted.
    pub max_items: u32,
    /// Rolling window used for rates / error ratios at the selected time.
    #[serde(with = "nanos_string")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub rolling_window_ns: i64,
    /// Memory budget for reducer checkpoints.
    pub checkpoint_budget_bytes: u32,
    pub degraded_error_ratio: f32,
    pub failing_error_ratio: f32,
    /// Request hops returned per frame cover `[time - hop_before_ns, time + hop_after_ns]`.
    #[serde(with = "nanos_string")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub hop_before_ns: i64,
    #[serde(with = "nanos_string")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub hop_after_ns: i64,
    /// Cap on hops per frame; the summary reports how many were left out.
    pub max_hops: u32,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            max_items: 200_000,
            rolling_window_ns: 30_000_000_000,
            checkpoint_budget_bytes: 16 * 1024 * 1024,
            degraded_error_ratio: 0.05,
            failing_error_ratio: 0.5,
            hop_before_ns: 1_000_000_000,
            hop_after_ns: 1_000_000_000,
            max_hops: 3_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TopologyNode {
    pub id: String,
    pub name: String,
    pub kind: EntityKindDto,
    pub provenance: ProvenanceDto,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TopologyEdge {
    pub source: String,
    pub target: String,
    pub relation: RelationKindDto,
    pub provenance: ProvenanceDto,
    #[serde(default)]
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TopologySnapshot {
    pub nodes: Vec<TopologyNode>,
    pub edges: Vec<TopologyEdge>,
}

/// One recorded item. IDs are the deduplication keys: applying the same item
/// twice has no additional effect.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TimelineItem {
    Span(SpanItem),
    Log(LogItem),
    Event(EventItem),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SpanItem {
    pub trace_id: String,
    pub span_id: String,
    #[serde(default)]
    pub parent_span_id: Option<String>,
    pub service_id: String,
    pub service_name: String,
    /// Resource / dependency entity this span calls (e.g. a database), when known.
    #[serde(default)]
    pub target_entity_id: Option<String>,
    pub name: String,
    /// OTLP SpanKind: 0 unspecified, 1 internal, 2 server, 3 client, 4 producer, 5 consumer.
    pub kind: u8,
    #[serde(with = "nanos_string")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub start_ns: i64,
    #[serde(with = "nanos_string")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub end_ns: i64,
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LogItem {
    pub id: String,
    pub service_id: String,
    pub service_name: String,
    #[serde(with = "nanos_string")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub time_ns: i64,
    /// OTLP severity number (17+ = ERROR).
    pub severity: u8,
    #[serde(default)]
    pub trace_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EventItem {
    pub id: String,
    pub event_type: String,
    #[serde(default)]
    pub service_id: Option<String>,
    #[serde(default)]
    pub service_name: Option<String>,
    #[serde(with = "nanos_string")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub time_ns: i64,
    #[serde(default)]
    pub trace_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Batch {
    pub items: Vec<TimelineItem>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub scale: f32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(default)]
pub struct Filters {
    pub search: String,
    pub hide_inferred: bool,
    pub only_problems: bool,
    pub selected_trace_id: Option<String>,
    /// With `selected_trace_id`: frames carry only that trace's hops, everything off its path is
    /// dimmed, and the timeline tracks count only its spans, logs and events.
    pub trace_only: bool,
    /// Service node IDs switched off by the user: dimmed on the map, left out of the timeline
    /// aggregates. Dependencies whose every caller is muted are dimmed too.
    pub muted_services: Vec<String>,
}

impl Filters {
    /// The trace everything else is hidden for, when `trace_only` is on.
    pub fn isolated_trace(&self) -> Option<&str> {
        if self.trace_only { self.selected_trace_id.as_deref() } else { None }
    }
}

/// Summary accompanying each computed frame.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FrameSummary {
    /// Serialized as a JSON number (values stay far below 2^53).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub generation: u64,
    #[serde(with = "opt_nanos_string")]
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub time_ns: Option<i64>,
    #[serde(with = "opt_nanos_string")]
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub prev_event_ns: Option<i64>,
    #[serde(with = "opt_nanos_string")]
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub next_event_ns: Option<i64>,
    pub total_active: u32,
    pub window_requests: u32,
    pub window_errors: u32,
    pub visible_nodes: u32,
    pub visible_edges: u32,
    /// 0 = overview (no labels), 1 = normal, 2 = detailed.
    pub lod: u8,
    /// Serialized as a JSON number (values stay far below 2^53).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub topology_revision: u64,
    /// Request hops on visible edges overlapping the hop window.
    pub hops_in_window: u32,
    /// Hops actually returned in `hop_data` (less than `hops_in_window` when capped).
    pub hops_shown: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct NodeMetrics {
    pub id: String,
    pub name: String,
    pub status: HealthStatusDto,
    pub status_reason: String,
    pub active: u32,
    pub window_requests: u32,
    pub window_completed: u32,
    pub window_errors: u32,
    pub avg_latency_ms: Option<f64>,
    pub window_logs: u32,
    pub window_error_logs: u32,
    pub window_events: u32,
}

/// Per-bucket aggregates and fine-zoom items for the layered timeline over `[from_ns, to_ns)`.
/// Times inside are milliseconds relative to `from_ns`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export))]
pub struct TimelineTracks {
    #[serde(with = "nanos_string")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub from_ns: i64,
    #[serde(with = "nanos_string")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub to_ns: i64,
    pub buckets: u32,
    /// Server, consumer and root spans by start time (a call is counted once, on the callee).
    pub requests: Vec<u32>,
    /// Failed request spans (same set as `requests`) plus logs with severity >= 17.
    pub errors: Vec<u32>,
    pub events: Vec<u32>,
    /// p95 duration of request spans ending in the bucket; `null` when the bucket has none.
    pub latency_p95_ms: Vec<Option<f32>>,
    pub latency_samples: Vec<u32>,
    pub services: Vec<ServiceTrack>,
    /// Span clips of services under the clip cap.
    pub clips: Vec<SpanClip>,
    /// Events whose type is not `request.*`, capped.
    pub markers: Vec<EventMarker>,
    pub markers_total: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export))]
pub struct ServiceTrack {
    pub node: u32,
    pub name: String,
    /// Spans of this service starting in each bucket.
    pub counts: Vec<u32>,
    /// Spans overlapping the window.
    pub spans_in_view: u32,
    /// True when `spans_in_view` is within the clip cap and every clip is in `clips`.
    pub clipped: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export))]
pub struct SpanClip {
    /// Index into `TimelineTracks::services`.
    pub track: u32,
    pub start_ms: f64,
    pub end_ms: f64,
    /// Nesting depth among spans of the same service in the trace (0 = outermost).
    pub depth: u8,
    pub error: bool,
    pub name: String,
    pub trace_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export))]
pub struct EventMarker {
    pub time_ms: f64,
    pub event_type: String,
    pub service_name: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct NodeDiff {
    pub id: String,
    pub name: String,
    pub before: NodeMetrics,
    pub after: NodeMetrics,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TopologyView {
    /// Serialized as a JSON number (values stay far below 2^53).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub revision: u64,
    pub nodes: Vec<TopologyViewNode>,
    pub edges: Vec<TopologyViewEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TopologyViewNode {
    pub index: u32,
    pub id: String,
    pub name: String,
    pub kind: EntityKindDto,
    pub provenance: ProvenanceDto,
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TopologyViewEdge {
    pub index: u32,
    pub source: u32,
    pub target: u32,
    pub relation: RelationKindDto,
    pub provenance: ProvenanceDto,
    pub reasons: Vec<String>,
    /// Corridor waypoints `[x, y]` (world coordinates) where the edge crosses intermediate
    /// layout columns; empty for edges between adjacent columns.
    pub points: Vec<[f32; 2]>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Diagnostics {
    pub spans: u32,
    pub logs: u32,
    pub events: u32,
    pub deltas: u32,
    pub checkpoints: u32,
    pub checkpoint_interval: u32,
    pub nodes: u32,
    pub edges: u32,
    /// Serialized as a JSON number (values stay far below 2^53).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub approx_memory_bytes: u64,
    /// Serialized as a JSON number (values stay far below 2^53).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub duplicates_ignored: u64,
    /// Serialized as a JSON number (values stay far below 2^53).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub evicted_items: u64,
    #[serde(with = "opt_nanos_string")]
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub evicted_before_ns: Option<i64>,
    /// Server spans whose parent span is not in the loaded window; their caller edge is unknown.
    pub unresolved_parents: u32,
    /// Serialized as a JSON number (values stay far below 2^53).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub stale_seeks_ignored: u64,
    /// Full timeline rebuilds (structural changes). Serialized as a JSON number.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub timeline_rebuilds: u64,
    /// Incremental merges of new items into the timeline. Serialized as a JSON number.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub timeline_merges: u64,
    #[serde(with = "opt_nanos_string")]
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub first_item_ns: Option<i64>,
    #[serde(with = "opt_nanos_string")]
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub last_item_ns: Option<i64>,
}

// DTO mirrors of domain enums so ts-rs can export them without making the
// domain crate depend on ts-rs.
macro_rules! dto_enum {
    ($dto:ident, $domain:ident, { $($v:ident),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
        #[cfg_attr(feature = "ts", derive(TS), ts(export))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
        #[serde(rename_all = "snake_case")]
        pub enum $dto {
            #[default]
            $($v),+
        }
        impl From<$domain> for $dto {
            fn from(v: $domain) -> Self { match v { $($domain::$v => $dto::$v),+ } }
        }
        impl From<$dto> for $domain {
            fn from(v: $dto) -> Self { match v { $($dto::$v => $domain::$v),+ } }
        }
    };
}

dto_enum!(HealthStatusDto, HealthStatus, { Unknown, Healthy, Degraded, Failing });
dto_enum!(ProvenanceDto, Provenance, { Observed, Declared, Inferred });
dto_enum!(RelationKindDto, RelationKind, { Calls, DependsOn, RunsOn, DeployedFrom, Owns, PublishesTo, ConsumesFrom });
dto_enum!(EntityKindDto, EntityKind, { Service, Instance, Repository, Deployment, Database, Cache, Queue, InfrastructureResource, ExternalDependency });
