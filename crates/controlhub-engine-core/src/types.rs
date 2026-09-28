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
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            max_items: 200_000,
            rolling_window_ns: 30_000_000_000,
            checkpoint_budget_bytes: 16 * 1024 * 1024,
            degraded_error_ratio: 0.05,
            failing_error_ratio: 0.5,
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
