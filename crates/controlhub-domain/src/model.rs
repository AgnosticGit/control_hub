//! Core graph and event model.

use serde::{Deserialize, Serialize};

/// What kind of object a graph node represents. Distinguishes logical services
/// from concrete instances, repositories, deployments and infrastructure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Service,
    Instance,
    Repository,
    Deployment,
    Database,
    Cache,
    Queue,
    InfrastructureResource,
    ExternalDependency,
}

/// Typed relationships. Only kinds actually produced by the system are listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    Calls,
    DependsOn,
    RunsOn,
    DeployedFrom,
    Owns,
    PublishesTo,
    ConsumesFrom,
}

/// How we know about a relationship or entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    /// Directly seen in telemetry (e.g. parent/child spans across services).
    Observed,
    /// Declared by configuration (e.g. `controlhub.yaml`).
    Declared,
    /// Derived from indirect evidence; always accompanied by reasons.
    Inferred,
}

/// Health as far as ControlHub can tell. `Unknown` is a first-class value:
/// absence of data is never displayed as healthy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Unknown,
    Healthy,
    Degraded,
    Failing,
}

impl HealthStatus {
    pub fn as_u8(self) -> u8 {
        match self {
            HealthStatus::Unknown => 0,
            HealthStatus::Healthy => 1,
            HealthStatus::Degraded => 2,
            HealthStatus::Failing => 3,
        }
    }
}

/// Telemetry signal types accepted by ingestion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Signal {
    Traces,
    Logs,
    Metrics,
    Events,
}

/// Well-known custom event types. Unknown types are still stored verbatim.
pub mod event_types {
    pub const REQUEST_STARTED: &str = "request.started";
    pub const REQUEST_COMPLETED: &str = "request.completed";
    pub const SERVICE_STARTED: &str = "service.started";
    pub const SERVICE_STOPPED: &str = "service.stopped";
    pub const DEPLOYMENT_STARTED: &str = "deployment.started";
    pub const DEPLOYMENT_COMPLETED: &str = "deployment.completed";
    pub const CONFIG_CHANGED: &str = "config.changed";
    pub const ACTION_AUDIT: &str = "action.audit";

    /// Event type must be a dotted lowercase identifier.
    pub fn is_valid(t: &str) -> bool {
        !t.is_empty()
            && t.len() <= 96
            && t.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_'))
    }
}

/// Validation limits for custom events (enforced at ingestion).
pub mod limits {
    pub const MAX_EVENTS_PER_BATCH: usize = 1_000;
    pub const MAX_ATTRIBUTES_PER_ITEM: usize = 64;
    pub const MAX_ATTRIBUTE_KEY_LEN: usize = 128;
    pub const MAX_ATTRIBUTE_VALUE_LEN: usize = 4_096;
    pub const MAX_LOG_BODY_LEN: usize = 16_384;
}
