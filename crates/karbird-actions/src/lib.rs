//! Controlled actions: what may be done, to what, by whom, and how success is checked.
//!
//! This crate is pure (no I/O) and shared by the API (propose/approve) and the executor
//! (which re-validates everything before it acts). Only allowlisted action types with typed,
//! bounded parameters exist; there is no generic "run command" action, and nothing here is
//! ever derived from telemetry.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Seconds a proposal waits for a decision, and an approval waits for the executor.
pub const DECISION_WINDOW_SECS: i64 = 30 * 60;
pub const EXECUTION_WINDOW_SECS: i64 = 10 * 60;
pub const REASON_MIN: usize = 5;
pub const REASON_MAX: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Low,
    Medium,
}

/// Demo services whose fault injection can be controlled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DemoService {
    Gateway,
    Orders,
}

impl DemoService {
    pub fn as_str(self) -> &'static str {
        match self {
            DemoService::Gateway => "gateway",
            DemoService::Orders => "orders",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "gateway" => Some(DemoService::Gateway),
            "orders" => Some(DemoService::Orders),
            _ => None,
        }
    }
}

/// Demo containers that can be restarted. The catalog replicas sit behind the gateway, which
/// the demo orchestrator drains before restarting one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DemoContainer {
    Gateway,
    Orders,
    #[serde(rename = "catalog-1")]
    Catalog1,
    #[serde(rename = "catalog-2")]
    Catalog2,
    #[serde(rename = "catalog-3")]
    Catalog3,
}

impl DemoContainer {
    pub const ALL: [DemoContainer; 5] = [
        DemoContainer::Gateway,
        DemoContainer::Orders,
        DemoContainer::Catalog1,
        DemoContainer::Catalog2,
        DemoContainer::Catalog3,
    ];

    /// Container name, which is also the instance ID its service reports (`service.instance.id`).
    pub fn as_str(self) -> &'static str {
        match self {
            DemoContainer::Gateway => "gateway",
            DemoContainer::Orders => "orders",
            DemoContainer::Catalog1 => "catalog-1",
            DemoContainer::Catalog2 => "catalog-2",
            DemoContainer::Catalog3 => "catalog-3",
        }
    }

    /// The service running in the container (`service.name`).
    pub fn service(self) -> &'static str {
        match self {
            DemoContainer::Gateway => "gateway",
            DemoContainer::Orders => "orders",
            DemoContainer::Catalog1 | DemoContainer::Catalog2 | DemoContainer::Catalog3 => "catalog",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == s)
    }
}

/// Demo services that run as several replicas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DemoScalable {
    Catalog,
}

impl DemoScalable {
    pub fn as_str(self) -> &'static str {
        match self {
            DemoScalable::Catalog => "catalog",
        }
    }

    /// Replicas the demo can run (one container each).
    pub fn max_replicas(self) -> u8 {
        match self {
            DemoScalable::Catalog => 3,
        }
    }

    fn parse(s: &str) -> Option<Self> {
        (s == "catalog").then_some(DemoScalable::Catalog)
    }
}

/// Demo feature flags. Each belongs to one service; all start off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DemoFlag {
    /// Gateway caches catalog items for 10 s (fewer gateway → catalog calls).
    CatalogCache,
    /// Orders charges through an endpoint payments does not have: every order write fails.
    CheckoutV2,
}

impl DemoFlag {
    pub const ALL: [DemoFlag; 2] = [DemoFlag::CatalogCache, DemoFlag::CheckoutV2];

    pub fn as_str(self) -> &'static str {
        match self {
            DemoFlag::CatalogCache => "catalog-cache",
            DemoFlag::CheckoutV2 => "checkout-v2",
        }
    }

    pub fn service(self) -> DemoService {
        match self {
            DemoFlag::CatalogCache => DemoService::Gateway,
            DemoFlag::CheckoutV2 => DemoService::Orders,
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.as_str() == s)
    }
}

/// Demo services with releases the orchestrator can switch between.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DemoReleaseService {
    Orders,
}

impl DemoReleaseService {
    pub fn as_str(self) -> &'static str {
        match self {
            DemoReleaseService::Orders => "orders",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        (s == "orders").then_some(DemoReleaseService::Orders)
    }
}

/// Release versions of the demo services. A rollback is only verified against one of these.
pub const DEMO_RELEASES: &[&str] = &["0.1.0", "0.2.0"];

/// The allowlisted release equal to `v`, if any.
pub fn demo_release(v: &str) -> Option<&'static str> {
    DEMO_RELEASES.iter().copied().find(|r| *r == v)
}

/// A validated action with typed parameters. The executor only ever runs one of these.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    /// Reset latency and error injection of a demo service.
    DemoFaultsClear { service: DemoService },
    /// Set latency and error injection of a demo service.
    DemoFaultsInject { service: DemoService, latency_ms: u32, error_rate: f64 },
    /// Restart the demo system's Postgres container.
    DemoRestartDatabase,
    /// Restart one demo service container (a new process with the same configuration).
    DemoContainerRestart { container: DemoContainer },
    /// Run exactly `replicas` replicas of a demo service behind the gateway.
    DemoReplicasScale { service: DemoScalable, replicas: u8 },
    /// Switch a demo feature flag on or off.
    DemoFeatureFlagSet { flag: DemoFlag, enabled: bool },
    /// Return a demo service to the release it ran before the current one.
    DemoReleaseRollback { service: DemoReleaseService },
}

#[derive(Debug, Clone, Serialize)]
pub struct ParamSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub min: f64,
    pub max: f64,
    pub integer: bool,
    /// An on/off switch, sent as the integer 0 or 1.
    pub boolean: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActionSpec {
    pub action_type: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub risk: Risk,
    /// Only allowed in environments marked as demo.
    pub demo_only: bool,
    pub targets: &'static [&'static str],
    pub params: &'static [ParamSpec],
    pub verification: &'static str,
}

pub const CATALOG: &[ActionSpec] = &[
    ActionSpec {
        action_type: "demo.faults.clear",
        title: "Clear injected faults",
        description: "Sets latency and error injection of a demo service back to zero.",
        risk: Risk::Low,
        demo_only: true,
        targets: &["gateway", "orders"],
        params: &[],
        verification: "Fault state reads back as zero, and the service's error ratio over the next 15 s is at most 5%.",
    },
    ActionSpec {
        action_type: "demo.faults.inject",
        title: "Inject faults",
        description: "Adds latency and/or a random error rate to a demo service.",
        risk: Risk::Medium,
        demo_only: true,
        targets: &["gateway", "orders"],
        params: &[
            ParamSpec {
                name: "latency_ms",
                description: "Added latency per request",
                min: 0.0,
                max: 5000.0,
                integer: true,
                boolean: false,
            },
            ParamSpec {
                name: "error_rate",
                description: "Share of requests that fail (0–1)",
                min: 0.0,
                max: 1.0,
                integer: false,
                boolean: false,
            },
        ],
        verification: "Fault state reads back as requested, and the service still receives requests over the next 15 s.",
    },
    ActionSpec {
        action_type: "demo.restart_database",
        title: "Restart demo database",
        description: "Restarts the demo system's Postgres container (orders loses its database for a few seconds).",
        risk: Risk::Medium,
        demo_only: true,
        targets: &["demo-postgres"],
        params: &[],
        verification: "The container reports healthy again, and orders' error ratio over the following 15 s is at most 20%.",
    },
    ActionSpec {
        action_type: "demo.container.restart",
        title: "Restart container",
        description: "Replaces the process of one demo service container with a fresh one. Catalog replicas are taken out of the gateway's rotation first.",
        risk: Risk::Medium,
        demo_only: true,
        targets: &["gateway", "orders", "catalog-1", "catalog-2", "catalog-3"],
        params: &[],
        verification: "The container runs a new process that reports healthy, and over the next 15 s the restarted instance itself serves requests with an error ratio of at most 5%.",
    },
    ActionSpec {
        action_type: "demo.replicas.scale",
        title: "Scale replicas",
        description: "Starts or stops catalog replicas until exactly the requested number run, and points the gateway at them.",
        risk: Risk::Medium,
        demo_only: true,
        targets: &["catalog"],
        params: &[ParamSpec {
            name: "replicas",
            description: "Replicas that should run",
            min: 1.0,
            max: 3.0,
            integer: true,
            boolean: false,
        }],
        verification: "Exactly that many replicas run and the gateway routes to all of them; over the next 15 s exactly that many catalog instances serve requests, with an error ratio of at most 5%.",
    },
    ActionSpec {
        action_type: "demo.feature_flag.set",
        title: "Set feature flag",
        description: "Turns a demo feature flag on or off: catalog-cache (gateway caches catalog items for 10 s) or checkout-v2 (orders charges through a new payments endpoint).",
        risk: Risk::Medium,
        demo_only: true,
        targets: &["catalog-cache", "checkout-v2"],
        params: &[ParamSpec {
            name: "enabled",
            description: "Flag on (1) or off (0)",
            min: 0.0,
            max: 1.0,
            integer: true,
            boolean: true,
        }],
        verification: "The flag reads back as requested, and over the next 15 s the service that owns it (gateway or orders) serves requests with an error ratio of at most 5%. A flag that breaks the service fails verification.",
    },
    ActionSpec {
        action_type: "demo.release.rollback",
        title: "Roll back release",
        description: "Restarts the service on the release it ran before the current one (deploy one first with `just demo-release orders 0.2.0`).",
        risk: Risk::Medium,
        demo_only: true,
        targets: &["orders"],
        params: &[],
        verification: "The service runs the earlier release, and over the next 15 s only that release serves requests, with an error ratio of at most 5%.",
    },
];

pub fn spec(action_type: &str) -> Option<&'static ActionSpec> {
    CATALOG.iter().find(|s| s.action_type == action_type)
}

/// Map card of the demo database (`db.system: db.namespace` of the spans that call it).
pub const DEMO_DATABASE_NODE: &str = "postgresql: orders";
/// Container of the demo database in the demo infrastructure layer.
pub const DEMO_DATABASE_CONTAINER: &str = "postgres";

/// Where a target shows up on the System Map: the card of its service (or resource) and, for
/// a container, its name in the infrastructure layer. The UI offers an action on that card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Subject {
    pub service: &'static str,
    pub container: Option<&'static str>,
}

/// The subject of a catalog target, or `None` for unknown types and targets.
pub fn subject(action_type: &str, target: &str) -> Option<Subject> {
    let service = |service: &'static str| Subject { service, container: None };
    match action_type {
        "demo.faults.clear" | "demo.faults.inject" => DemoService::parse(target).map(|s| service(s.as_str())),
        "demo.restart_database" => (target == "demo-postgres")
            .then_some(Subject { service: DEMO_DATABASE_NODE, container: Some(DEMO_DATABASE_CONTAINER) }),
        "demo.container.restart" => {
            DemoContainer::parse(target).map(|c| Subject { service: c.service(), container: Some(c.as_str()) })
        }
        "demo.replicas.scale" => DemoScalable::parse(target).map(|s| service(s.as_str())),
        "demo.feature_flag.set" => DemoFlag::parse(target).map(|f| service(f.service().as_str())),
        "demo.release.rollback" => DemoReleaseService::parse(target).map(|s| service(s.as_str())),
        _ => None,
    }
}

impl Action {
    pub fn action_type(&self) -> &'static str {
        match self {
            Action::DemoFaultsClear { .. } => "demo.faults.clear",
            Action::DemoFaultsInject { .. } => "demo.faults.inject",
            Action::DemoRestartDatabase => "demo.restart_database",
            Action::DemoContainerRestart { .. } => "demo.container.restart",
            Action::DemoReplicasScale { .. } => "demo.replicas.scale",
            Action::DemoFeatureFlagSet { .. } => "demo.feature_flag.set",
            Action::DemoReleaseRollback { .. } => "demo.release.rollback",
        }
    }

    pub fn target(&self) -> &'static str {
        match self {
            Action::DemoFaultsClear { service } | Action::DemoFaultsInject { service, .. } => service.as_str(),
            Action::DemoRestartDatabase => "demo-postgres",
            Action::DemoContainerRestart { container } => container.as_str(),
            Action::DemoReplicasScale { service, .. } => service.as_str(),
            Action::DemoFeatureFlagSet { flag, .. } => flag.as_str(),
            Action::DemoReleaseRollback { service } => service.as_str(),
        }
    }

    pub fn spec(&self) -> &'static ActionSpec {
        // Every variant has a catalog entry (checked by a unit test).
        spec(self.action_type()).unwrap_or(&CATALOG[0])
    }

    /// Canonical parameters as stored with the plan.
    pub fn params(&self) -> Value {
        match self {
            Action::DemoFaultsInject { latency_ms, error_rate, .. } => {
                serde_json::json!({ "latency_ms": latency_ms, "error_rate": error_rate })
            }
            Action::DemoReplicasScale { replicas, .. } => serde_json::json!({ "replicas": replicas }),
            Action::DemoFeatureFlagSet { enabled, .. } => serde_json::json!({ "enabled": u8::from(*enabled) }),
            Action::DemoFaultsClear { .. }
            | Action::DemoRestartDatabase
            | Action::DemoContainerRestart { .. }
            | Action::DemoReleaseRollback { .. } => serde_json::json!({}),
        }
    }

    /// Telemetry check after execution.
    pub fn verification(&self) -> Verification {
        let base = |service: &'static str, max_error_ratio: Option<f64>| Verification {
            service,
            instance: None,
            observe_secs: 15,
            min_requests: 1,
            max_error_ratio,
            expect_instances: None,
            expect_version: None,
        };
        match self {
            Action::DemoFaultsClear { service } => base(service.as_str(), Some(0.05)),
            Action::DemoFaultsInject { service, .. } => base(service.as_str(), None),
            Action::DemoRestartDatabase => base("orders", Some(0.2)),
            Action::DemoContainerRestart { container } => {
                Verification { instance: Some(container.as_str()), ..base(container.service(), Some(0.05)) }
            }
            Action::DemoReplicasScale { service, replicas } => {
                Verification { expect_instances: Some(u64::from(*replicas)), ..base(service.as_str(), Some(0.05)) }
            }
            Action::DemoFeatureFlagSet { flag, .. } => base(flag.service().as_str(), Some(0.05)),
            Action::DemoReleaseRollback { service } => base(service.as_str(), Some(0.05)),
        }
    }

    /// The check once the step has run. A rollback is verified against the release the adapter
    /// reports it returned to, which must be an allowlisted release.
    pub fn verification_after(&self, result: &Value) -> Result<Verification, String> {
        let mut v = self.verification();
        if let Action::DemoReleaseRollback { .. } = self {
            let to = result.get("to_version").and_then(Value::as_str).and_then(demo_release);
            v.expect_version = Some(to.ok_or("the adapter did not report an allowlisted release to verify")?);
        }
        Ok(v)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Verification {
    pub service: &'static str,
    /// Only requests served by this instance count (a restarted container).
    pub instance: Option<&'static str>,
    pub observe_secs: u64,
    pub min_requests: u64,
    pub max_error_ratio: Option<f64>,
    /// Exactly this many instances must serve requests (scaling).
    pub expect_instances: Option<u64>,
    /// Only requests served by this release count, and no other release may serve any (rollback).
    pub expect_version: Option<&'static str>,
}

/// Server spans of the verified service in the window after the action, as counted by the
/// executor's query. `requests`/`errors` are limited to the instance and release under test;
/// `instances` counts distinct serving instances of that release; `other_version_requests`
/// counts requests served by any other release.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Observed {
    pub requests: u64,
    pub errors: u64,
    pub instances: u64,
    pub other_version_requests: u64,
}

impl Verification {
    fn subject(&self) -> String {
        match (self.instance, self.expect_version) {
            (Some(i), _) => format!("{} instance {i}", self.service),
            (None, Some(v)) => format!("{} {v}", self.service),
            (None, None) => self.service.to_owned(),
        }
    }

    /// Judges what was observed in the window after the action.
    pub fn judge(&self, o: &Observed) -> (bool, String) {
        let who = self.subject();
        if o.requests < self.min_requests {
            return (false, format!("{who} received {} requests (need ≥ {})", o.requests, self.min_requests));
        }
        if self.expect_version.is_some() && o.other_version_requests > 0 {
            return (
                false,
                format!("{} served {} requests from another release", self.service, o.other_version_requests),
            );
        }
        if let Some(n) = self.expect_instances
            && o.instances != n
        {
            return (false, format!("{} instances of {} served requests (expected {n})", o.instances, self.service));
        }
        let ratio = o.errors as f64 / o.requests as f64;
        match self.max_error_ratio {
            Some(max) if ratio > max => {
                (false, format!("{who} error ratio {:.1}% exceeds {:.0}%", ratio * 100.0, max * 100.0))
            }
            _ => {
                let instances = self.expect_instances.map(|_| format!(" across {} instances", o.instances));
                (
                    true,
                    format!(
                        "{who} served {} requests{}, error ratio {:.1}%",
                        o.requests,
                        instances.unwrap_or_default(),
                        ratio * 100.0
                    ),
                )
            }
        }
    }
}

fn number(params: &Value, p: &ParamSpec, errors: &mut Vec<String>) -> Option<f64> {
    match params.get(p.name) {
        None | Some(Value::Null) => {
            errors.push(format!("missing parameter {}", p.name));
            None
        }
        Some(v) => match v.as_f64() {
            Some(n) if n.is_finite() && n >= p.min && n <= p.max && (!p.integer || n.fract() == 0.0) => Some(n),
            _ => {
                errors.push(format!(
                    "{} must be {} in [{}, {}]",
                    p.name,
                    if p.integer { "an integer" } else { "a number" },
                    p.min,
                    p.max
                ));
                None
            }
        },
    }
}

/// Validates an untrusted proposal into a typed action. Unknown types, targets or extra
/// parameters are rejected.
pub fn validate(action_type: &str, target: &str, params: &Value) -> Result<Action, Vec<String>> {
    let Some(spec) = spec(action_type) else {
        return Err(vec![format!("unknown action type {action_type:?}")]);
    };
    let mut errors = Vec::new();
    if !spec.targets.contains(&target) {
        errors.push(format!("target must be one of {}", spec.targets.join(", ")));
    }
    let obj = match params {
        Value::Object(m) => Some(m),
        Value::Null => None,
        _ => {
            errors.push("params must be an object".into());
            None
        }
    };
    if let Some(m) = obj {
        for k in m.keys() {
            if !spec.params.iter().any(|p| p.name == k) {
                errors.push(format!("unknown parameter {k}"));
            }
        }
    }
    let action = match action_type {
        "demo.faults.clear" => DemoService::parse(target).map(|service| Action::DemoFaultsClear { service }),
        "demo.faults.inject" => {
            let latency = number(params, &spec.params[0], &mut errors);
            let rate = number(params, &spec.params[1], &mut errors);
            match (DemoService::parse(target), latency, rate) {
                (Some(service), Some(l), Some(r)) => {
                    if l == 0.0 && r == 0.0 {
                        errors.push(
                            "inject needs latency_ms > 0 or error_rate > 0 (use demo.faults.clear to reset)".into(),
                        );
                    }
                    Some(Action::DemoFaultsInject { service, latency_ms: l as u32, error_rate: r })
                }
                _ => None,
            }
        }
        "demo.restart_database" => Some(Action::DemoRestartDatabase),
        "demo.container.restart" => {
            DemoContainer::parse(target).map(|container| Action::DemoContainerRestart { container })
        }
        "demo.replicas.scale" => {
            let replicas = number(params, &spec.params[0], &mut errors);
            match (DemoScalable::parse(target), replicas) {
                (Some(service), Some(n)) if n <= f64::from(service.max_replicas()) => {
                    Some(Action::DemoReplicasScale { service, replicas: n as u8 })
                }
                (Some(service), Some(_)) => {
                    errors.push(format!("{} runs at most {} replicas", service.as_str(), service.max_replicas()));
                    None
                }
                _ => None,
            }
        }
        "demo.feature_flag.set" => {
            let enabled = number(params, &spec.params[0], &mut errors);
            match (DemoFlag::parse(target), enabled) {
                (Some(flag), Some(e)) => Some(Action::DemoFeatureFlagSet { flag, enabled: e == 1.0 }),
                _ => None,
            }
        }
        "demo.release.rollback" => {
            DemoReleaseService::parse(target).map(|service| Action::DemoReleaseRollback { service })
        }
        _ => None,
    };
    match action {
        Some(a) if errors.is_empty() => Ok(a),
        _ => Err(errors),
    }
}

pub fn validate_reason(reason: &str) -> Result<&str, String> {
    let r = reason.trim();
    let n = r.chars().count();
    if (REASON_MIN..=REASON_MAX).contains(&n) && !r.chars().any(|c| c.is_control()) {
        Ok(r)
    } else {
        Err(format!("reason must be {REASON_MIN}-{REASON_MAX} characters without control characters"))
    }
}

/// Facts the policy needs; gathered by the caller from PostgreSQL.
#[derive(Debug, Clone, Copy)]
pub struct PolicyInput {
    pub environment_is_demo: bool,
    /// Plans for the same target that are proposed, approved or running.
    pub open_plans_for_target: i64,
    /// The proposer holds the approve permission, so the plan is approved as it is proposed.
    pub approved_by_proposer: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PolicyCheck {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PolicyDecision {
    pub allowed: bool,
    /// False when the proposer may run actions themselves; otherwise someone with the approve
    /// permission has to approve the plan before it runs.
    pub requires_approval: bool,
    pub checks: Vec<PolicyCheck>,
}

/// `exclude_self` is true when re-evaluating an existing plan (at approval or execution),
/// which is itself counted among the open plans.
pub fn evaluate(action: &Action, input: &PolicyInput, exclude_self: bool) -> PolicyDecision {
    let spec = action.spec();
    let own = i64::from(exclude_self);
    let mut checks = vec![PolicyCheck {
        name: "allowlisted action".into(),
        passed: true,
        detail: format!("{} ({:?} risk)", spec.action_type, spec.risk),
    }];
    checks.push(PolicyCheck {
        name: "demo environment".into(),
        passed: !spec.demo_only || input.environment_is_demo,
        detail: if spec.demo_only { "demo-only action".into() } else { "allowed in any environment".into() },
    });
    let open = input.open_plans_for_target - own;
    checks.push(PolicyCheck {
        name: "one open plan per target".into(),
        passed: open <= 0,
        detail: format!("{open} other open plan(s) for {}", action.target()),
    });
    PolicyDecision { allowed: checks.iter().all(|c| c.passed), requires_approval: !input.approved_by_proposer, checks }
}

/// Lifecycle of a plan. Terminal states never change again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Proposed,
    Approved,
    Rejected,
    Cancelled,
    Expired,
    Running,
    Succeeded,
    Failed,
    VerificationFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Approve,
    Reject,
    Cancel,
    Expire,
    Claim,
    Succeed,
    Fail,
    FailVerification,
    LoseExecutor,
}

impl State {
    pub const ALL: [State; 9] = [
        State::Proposed,
        State::Approved,
        State::Rejected,
        State::Cancelled,
        State::Expired,
        State::Running,
        State::Succeeded,
        State::Failed,
        State::VerificationFailed,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            State::Proposed => "proposed",
            State::Approved => "approved",
            State::Rejected => "rejected",
            State::Cancelled => "cancelled",
            State::Expired => "expired",
            State::Running => "running",
            State::Succeeded => "succeeded",
            State::Failed => "failed",
            State::VerificationFailed => "verification_failed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|x| x.as_str() == s)
    }

    pub fn is_terminal(self) -> bool {
        !matches!(self, State::Proposed | State::Approved | State::Running)
    }

    /// The only allowed transitions. Running plans cannot be cancelled (the effect may already
    /// be under way); a lost executor makes the outcome `failed` (unknown), never a retry.
    pub fn next(self, event: Event) -> Option<State> {
        use {Event as E, State as S};
        match (self, event) {
            (S::Proposed, E::Approve) => Some(S::Approved),
            (S::Proposed, E::Reject) => Some(S::Rejected),
            (S::Proposed | S::Approved, E::Cancel) => Some(S::Cancelled),
            (S::Proposed | S::Approved, E::Expire) => Some(S::Expired),
            (S::Approved, E::Claim) => Some(S::Running),
            (S::Running, E::Succeed) => Some(S::Succeeded),
            (S::Running, E::Fail | E::LoseExecutor) => Some(S::Failed),
            (S::Running, E::FailVerification) => Some(S::VerificationFailed),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use proptest::prelude::*;
    use serde_json::json;

    /// One of every variant, for every target it can have.
    fn all_actions() -> Vec<Action> {
        let mut all = vec![
            Action::DemoFaultsClear { service: DemoService::Orders },
            Action::DemoFaultsClear { service: DemoService::Gateway },
            Action::DemoFaultsInject { service: DemoService::Gateway, latency_ms: 1, error_rate: 0.0 },
            Action::DemoFaultsInject { service: DemoService::Orders, latency_ms: 10, error_rate: 0.1 },
            Action::DemoRestartDatabase,
            Action::DemoReplicasScale { service: DemoScalable::Catalog, replicas: 2 },
            Action::DemoReleaseRollback { service: DemoReleaseService::Orders },
        ];
        all.extend(DemoContainer::ALL.map(|container| Action::DemoContainerRestart { container }));
        all.extend(DemoFlag::ALL.map(|flag| Action::DemoFeatureFlagSet { flag, enabled: true }));
        all
    }

    fn obs(requests: u64, errors: u64) -> Observed {
        Observed { requests, errors, ..Observed::default() }
    }

    #[test]
    fn every_action_has_a_catalog_entry() {
        for a in all_actions() {
            assert_eq!(a.spec().action_type, a.action_type());
            assert!(a.spec().targets.contains(&a.target()), "{a:?}");
            assert_eq!(validate(a.action_type(), a.target(), &a.params()).unwrap(), a, "stored plans re-validate");
        }
        let mut targets: Vec<_> =
            CATALOG.iter().flat_map(|s| s.targets.iter().map(move |t| (s.action_type, *t))).collect();
        let covered: Vec<_> = all_actions().iter().map(|a| (a.action_type(), a.target())).collect();
        targets.retain(|t| !covered.contains(t));
        assert!(targets.is_empty(), "catalog targets without a typed action: {targets:?}");
    }

    #[test]
    fn every_catalog_target_has_a_map_subject() {
        for s in CATALOG {
            for t in s.targets {
                assert!(subject(s.action_type, t).is_some(), "{} {t}", s.action_type);
            }
        }
        let restart = subject("demo.container.restart", "catalog-2").unwrap();
        assert_eq!((restart.service, restart.container), ("catalog", Some("catalog-2")));
        assert_eq!(subject("demo.feature_flag.set", "checkout-v2").unwrap().service, "orders");
        assert_eq!(subject("demo.feature_flag.set", "catalog-cache").unwrap().service, "gateway");
        assert_eq!(subject("demo.replicas.scale", "catalog").unwrap(), Subject { service: "catalog", container: None });
        let db = subject("demo.restart_database", "demo-postgres").unwrap();
        assert_eq!((db.service, db.container), ("postgresql: orders", Some("postgres")));
        for (t, target) in [
            ("demo.container.restart", "postgres"),
            ("demo.restart_database", "orders"),
            ("demo.faults.clear", "catalog"),
            ("shell.exec", "orders"),
            ("demo.feature_flag.set", "gateway"),
        ] {
            assert_eq!(subject(t, target), None, "{t} {target}");
        }
    }

    #[test]
    fn container_subjects_match_the_verified_service() {
        for container in DemoContainer::ALL {
            let a = Action::DemoContainerRestart { container };
            let s = subject(a.action_type(), a.target()).unwrap();
            assert_eq!(s.service, a.verification().service);
            assert_eq!(s.container, a.verification().instance);
        }
    }

    #[test]
    fn validation_rejects_unknown_and_out_of_range_input() {
        assert!(validate("shell.exec", "orders", &json!({})).is_err());
        assert!(validate("demo.faults.clear", "postgres", &json!({})).is_err());
        assert!(validate("demo.faults.clear", "orders", &json!({ "cmd": "rm -rf /" })).is_err());
        assert!(validate("demo.faults.inject", "orders", &json!({ "latency_ms": 10.5, "error_rate": 0.1 })).is_err());
        assert!(validate("demo.faults.inject", "orders", &json!({ "latency_ms": 100, "error_rate": 2 })).is_err());
        assert!(validate("demo.faults.inject", "orders", &json!({ "latency_ms": 0, "error_rate": 0 })).is_err());
        assert!(validate("demo.faults.inject", "orders", &json!({ "latency_ms": 100 })).is_err());
        assert_eq!(
            validate("demo.faults.inject", "orders", &json!({ "latency_ms": 100, "error_rate": 0.5 })).unwrap(),
            Action::DemoFaultsInject { service: DemoService::Orders, latency_ms: 100, error_rate: 0.5 }
        );
        assert_eq!(
            validate("demo.restart_database", "demo-postgres", &Value::Null).unwrap(),
            Action::DemoRestartDatabase
        );
    }

    #[test]
    fn policy_blocks_non_demo_environments_and_concurrent_plans() {
        let a = Action::DemoFaultsClear { service: DemoService::Orders };
        let ok = PolicyInput { environment_is_demo: true, open_plans_for_target: 0, approved_by_proposer: false };
        assert!(evaluate(&a, &ok, false).allowed);
        assert!(evaluate(&a, &ok, false).requires_approval);
        assert!(!evaluate(&a, &PolicyInput { environment_is_demo: false, ..ok }, false).allowed);
        let busy = PolicyInput { open_plans_for_target: 1, ..ok };
        assert!(!evaluate(&a, &busy, false).allowed);
        assert!(evaluate(&a, &busy, true).allowed, "the plan itself does not block its own approval");
    }

    #[test]
    fn proposers_with_the_approve_permission_need_no_second_approval() {
        let a = Action::DemoContainerRestart { container: DemoContainer::Gateway };
        let input = PolicyInput { environment_is_demo: true, open_plans_for_target: 0, approved_by_proposer: true };
        let d = evaluate(&a, &input, false);
        assert!(d.allowed && !d.requires_approval);
        // Running it yourself does not lift the other checks.
        assert!(!evaluate(&a, &PolicyInput { environment_is_demo: false, ..input }, false).allowed);
        assert!(!evaluate(&a, &PolicyInput { open_plans_for_target: 1, ..input }, false).allowed);
        let names: Vec<_> = d.checks.iter().map(|c| c.name.as_str()).collect();
        assert!(!names.iter().any(|n| n.contains("rate")), "there is no plan quota: {names:?}");
    }

    #[test]
    fn verification_judgement() {
        let v = Action::DemoFaultsClear { service: DemoService::Orders }.verification();
        assert!(!v.judge(&obs(0, 0)).0);
        assert!(v.judge(&obs(100, 5)).0);
        assert!(!v.judge(&obs(100, 6)).0);
        let inject =
            Action::DemoFaultsInject { service: DemoService::Orders, latency_ms: 0, error_rate: 1.0 }.verification();
        assert!(inject.judge(&obs(10, 10)).0, "errors are expected after injecting them");
    }

    fn event() -> impl Strategy<Value = Event> {
        prop_oneof![
            Just(Event::Approve),
            Just(Event::Reject),
            Just(Event::Cancel),
            Just(Event::Expire),
            Just(Event::Claim),
            Just(Event::Succeed),
            Just(Event::Fail),
            Just(Event::FailVerification),
            Just(Event::LoseExecutor),
        ]
    }

    proptest! {
        /// Terminal states are final, and nothing reaches Running without an approval.
        #[test]
        fn lifecycle_invariants(events in proptest::collection::vec(event(), 0..20)) {
            let mut s = State::Proposed;
            let mut approved = false;
            for e in events {
                if let Some(n) = s.next(e) {
                    prop_assert!(!s.is_terminal());
                    if n == State::Approved { approved = true; }
                    if n == State::Running { prop_assert!(approved); }
                    s = n;
                }
                prop_assert_eq!(State::parse(s.as_str()), Some(s));
            }
        }
    }

    const ALL_EVENTS: [Event; 9] = [
        Event::Approve,
        Event::Reject,
        Event::Cancel,
        Event::Expire,
        Event::Claim,
        Event::Succeed,
        Event::Fail,
        Event::FailVerification,
        Event::LoseExecutor,
    ];

    /// The complete transition table from ADR-0012. Anything not listed must be refused.
    fn allowed(s: State, e: Event) -> Option<State> {
        use {Event as E, State as S};
        [
            (S::Proposed, E::Approve, S::Approved),
            (S::Proposed, E::Reject, S::Rejected),
            (S::Proposed, E::Cancel, S::Cancelled),
            (S::Proposed, E::Expire, S::Expired),
            (S::Approved, E::Cancel, S::Cancelled),
            (S::Approved, E::Expire, S::Expired),
            (S::Approved, E::Claim, S::Running),
            (S::Running, E::Succeed, S::Succeeded),
            (S::Running, E::Fail, S::Failed),
            (S::Running, E::LoseExecutor, S::Failed),
            (S::Running, E::FailVerification, S::VerificationFailed),
        ]
        .into_iter()
        .find(|(from, ev, _)| *from == s && *ev == e)
        .map(|(_, _, to)| to)
    }

    #[test]
    fn transition_table_is_exactly_the_documented_one() {
        let mut accepted = 0;
        for s in State::ALL {
            for e in ALL_EVENTS {
                assert_eq!(s.next(e), allowed(s, e), "{s:?} + {e:?}");
                accepted += usize::from(s.next(e).is_some());
            }
        }
        assert_eq!(accepted, 11);
    }

    #[test]
    fn terminal_states_accept_no_event() {
        for s in State::ALL.into_iter().filter(|s| s.is_terminal()) {
            for e in ALL_EVENTS {
                assert_eq!(s.next(e), None, "{s:?} + {e:?}");
            }
        }
        let non_terminal: Vec<_> = State::ALL.into_iter().filter(|s| !s.is_terminal()).collect();
        assert_eq!(non_terminal, [State::Proposed, State::Approved, State::Running]);
    }

    #[test]
    fn running_plans_cannot_be_cancelled_expired_or_reclaimed() {
        for e in [Event::Cancel, Event::Expire, Event::Claim, Event::Approve, Event::Reject] {
            assert_eq!(State::Running.next(e), None, "{e:?}");
        }
        assert_eq!(State::Proposed.next(Event::Claim), None, "no execution without approval");
        assert_eq!(State::Approved.next(Event::Approve), None, "no double approval");
        assert_eq!(State::Approved.next(Event::Reject), None, "approved plans are cancelled, not rejected");
    }

    #[test]
    fn state_strings_round_trip_and_match_serde() {
        for s in State::ALL {
            assert_eq!(State::parse(s.as_str()), Some(s));
            assert_eq!(serde_json::to_value(s).unwrap(), json!(s.as_str()));
            assert_eq!(serde_json::from_value::<State>(json!(s.as_str())).unwrap(), s);
        }
        for bad in ["", "Proposed", "PROPOSED", " proposed", "running;", "deleted"] {
            assert_eq!(State::parse(bad), None, "{bad:?}");
        }
    }

    fn state() -> impl Strategy<Value = State> {
        proptest::sample::select(State::ALL.to_vec())
    }

    proptest! {
        /// No illegal transition is ever accepted, from any state.
        #[test]
        fn no_illegal_transition_is_accepted(s in state(), e in event()) {
            prop_assert_eq!(s.next(e), allowed(s, e));
            if let Some(n) = s.next(e) {
                prop_assert_ne!(n, s, "every transition changes the state");
                prop_assert_ne!(n, State::Proposed, "nothing returns to proposed");
            }
        }

        /// Any accepted run is at most proposed → approved → running → terminal (≤ 3 steps).
        #[test]
        fn lifecycles_are_short_and_monotonic(events in proptest::collection::vec(event(), 0..40)) {
            let mut s = State::Proposed;
            let mut steps = 0;
            for e in events {
                if let Some(n) = s.next(e) {
                    s = n;
                    steps += 1;
                }
            }
            prop_assert!(steps <= 3);
        }
    }

    #[test]
    fn catalog_is_bounded_and_consistent() {
        let mut types: Vec<_> = CATALOG.iter().map(|s| s.action_type).collect();
        types.sort_unstable();
        types.dedup();
        assert_eq!(types.len(), CATALOG.len(), "action types are unique");
        for s in CATALOG {
            assert!(s.demo_only, "{} only demo actions exist", s.action_type);
            assert!(!s.targets.is_empty() && !s.verification.is_empty());
            assert_eq!(spec(s.action_type).map(|x| x.action_type), Some(s.action_type));
            for p in s.params {
                assert!(p.min.is_finite() && p.max.is_finite() && p.min <= p.max, "{}", p.name);
            }
        }
        assert!(spec("shell.exec").is_none());
        assert!(spec("DEMO.FAULTS.CLEAR").is_none());
        assert!(spec("").is_none());
    }

    #[test]
    fn validation_rejects_wrong_param_shapes() {
        for params in [json!([]), json!("latency_ms=1"), json!(1), json!(true)] {
            let e = validate("demo.faults.clear", "orders", &params).unwrap_err();
            assert!(e.iter().any(|m| m.contains("params must be an object")), "{params}");
        }
        for (latency, rate) in [
            (json!("100"), json!(0.1)),
            (json!(null), json!(0.1)),
            (json!(-1), json!(0.1)),
            (json!(5001), json!(0.1)),
            (json!(100), json!(-0.01)),
            (json!(100), json!(1.0001)),
            (json!(100), json!("0.5")),
            (json!(1e300), json!(0.5)),
        ] {
            let p = json!({ "latency_ms": latency, "error_rate": rate });
            assert!(validate("demo.faults.inject", "orders", &p).is_err(), "{p}");
        }
        // Every problem is reported, not just the first.
        let e = validate("demo.faults.inject", "db", &json!({ "latency_ms": -1, "x": 1 })).unwrap_err();
        assert!(e.len() >= 4, "{e:?}");
    }

    #[test]
    fn validation_accepts_bounds_and_canonicalises_params() {
        for (l, r) in [(5000, 0.0), (0, 1.0), (1, 0.0), (0, 0.000_001)] {
            let a = validate("demo.faults.inject", "gateway", &json!({ "latency_ms": l, "error_rate": r })).unwrap();
            assert_eq!(a.target(), "gateway");
            assert_eq!(a.params(), json!({ "latency_ms": l, "error_rate": r }));
            // A stored plan re-validates to the same action (API approval and executor rely on it).
            assert_eq!(validate(a.action_type(), a.target(), &a.params()).unwrap(), a);
        }
        let clear = validate("demo.faults.clear", "gateway", &json!({})).unwrap();
        assert_eq!(clear.params(), json!({}));
        assert!(validate("demo.restart_database", "orders", &json!({})).is_err(), "fixed target only");
        assert!(validate("demo.restart_database", "demo-postgres", &json!({ "force": true })).is_err());
    }

    #[test]
    fn reasons_are_bounded_and_single_line() {
        assert_eq!(validate_reason("  error rate spike  ").unwrap(), "error rate spike");
        assert!(validate_reason("abcd").is_err(), "below minimum");
        assert!(validate_reason("     abcd     ").is_err(), "trimmed before counting");
        assert!(validate_reason(&"x".repeat(REASON_MAX)).is_ok());
        assert!(validate_reason(&"x".repeat(REASON_MAX + 1)).is_err());
        assert!(validate_reason(&"é".repeat(REASON_MAX)).is_ok(), "counted in characters, not bytes");
        assert!(validate_reason("line one\nline two").is_err());
        assert!(validate_reason("bell\u{7} char").is_err());
        assert!(validate_reason("esc \u{1b}[31m red").is_err());
    }

    #[test]
    fn policy_limits_at_their_boundaries() {
        let a = Action::DemoRestartDatabase;
        let base = PolicyInput { environment_is_demo: true, open_plans_for_target: 0, approved_by_proposer: false };
        // Re-evaluation excludes the plan itself from the open plans.
        let own = PolicyInput { open_plans_for_target: 1, ..base };
        assert!(evaluate(&a, &own, true).allowed);
        assert!(!evaluate(&a, &PolicyInput { open_plans_for_target: 2, ..own }, true).allowed);

        let denied = evaluate(&a, &PolicyInput { environment_is_demo: false, ..base }, false);
        assert!(!denied.allowed && denied.requires_approval);
        let failed: Vec<_> = denied.checks.iter().filter(|c| !c.passed).map(|c| c.name.as_str()).collect();
        assert_eq!(failed, ["demo environment"]);
        assert_eq!(denied.checks.len(), 3);
    }

    proptest! {
        /// Approval is required unless the proposer may approve, and `allowed` is exactly
        /// "all checks passed".
        #[test]
        fn policy_decision_is_conjunction_of_checks(
            demo in any::<bool>(), open in -2i64..5, self_approved in any::<bool>(), own in any::<bool>(),
        ) {
            let input = PolicyInput { environment_is_demo: demo, open_plans_for_target: open, approved_by_proposer: self_approved };
            for a in all_actions() {
                let d = evaluate(&a, &input, own);
                prop_assert_eq!(d.requires_approval, !self_approved);
                prop_assert_eq!(d.allowed, d.checks.iter().all(|c| c.passed));
                if !demo { prop_assert!(!d.allowed, "demo-only actions never pass outside demo"); }
                let o = i64::from(own);
                prop_assert_eq!(d.allowed, demo && open - o <= 0);
            }
        }

        /// Arbitrary inject parameters either validate to in-range typed values or are refused.
        #[test]
        fn inject_validation_never_produces_out_of_range_values(l in -10_000.0f64..10_000.0, r in -2.0f64..2.0) {
            match validate("demo.faults.inject", "orders", &json!({ "latency_ms": l, "error_rate": r })) {
                Ok(Action::DemoFaultsInject { latency_ms, error_rate, .. }) => {
                    prop_assert!(latency_ms <= 5000 && f64::from(latency_ms) == l);
                    prop_assert!((0.0..=1.0).contains(&error_rate));
                    prop_assert!(latency_ms > 0 || error_rate > 0.0);
                }
                Ok(other) => prop_assert!(false, "unexpected {:?}", other),
                Err(e) => prop_assert!(!e.is_empty()),
            }
        }

        /// Arbitrary scale and flag parameters either validate to in-range typed values or are refused.
        #[test]
        fn scale_and_flag_validation_never_produces_out_of_range_values(n in -10.0f64..10.0, e in -2.0f64..3.0) {
            match validate("demo.replicas.scale", "catalog", &json!({ "replicas": n })) {
                Ok(Action::DemoReplicasScale { replicas, .. }) => {
                    prop_assert!((1..=3).contains(&replicas) && f64::from(replicas) == n);
                }
                Ok(other) => prop_assert!(false, "unexpected {:?}", other),
                Err(errs) => prop_assert!(!errs.is_empty()),
            }
            match validate("demo.feature_flag.set", "catalog-cache", &json!({ "enabled": e })) {
                Ok(Action::DemoFeatureFlagSet { enabled, .. }) => prop_assert_eq!(f64::from(u8::from(enabled)), e),
                Ok(other) => prop_assert!(false, "unexpected {:?}", other),
                Err(errs) => prop_assert!(!errs.is_empty()),
            }
        }

        /// Unknown action types and targets never validate.
        #[test]
        fn unknown_types_and_targets_are_refused(t in "[a-z._;/ -]{0,24}", target in "[a-z._;/ -]{0,24}") {
            let known_type = CATALOG.iter().any(|s| s.action_type == t);
            let r = validate(&t, &target, &json!({}));
            if !known_type {
                prop_assert!(r.is_err());
            } else if let Ok(a) = r {
                prop_assert_eq!(a.target(), target.as_str());
            }
        }
    }

    #[test]
    fn verification_judgement_boundaries() {
        let db = Action::DemoRestartDatabase.verification();
        assert_eq!(db.service, "orders");
        assert!(db.judge(&obs(10, 2)).0, "exactly 20% passes");
        assert!(!db.judge(&obs(10, 3)).0);
        let (ok, detail) = db.judge(&obs(0, 0));
        assert!(!ok && detail.contains("0 requests"), "{detail}");
        let clear = Action::DemoFaultsClear { service: DemoService::Gateway }.verification();
        assert_eq!(clear.service, "gateway");
        assert!(clear.judge(&obs(20, 1)).0, "exactly 5% passes");
        assert!(!clear.judge(&obs(19, 1)).0);
        for a in all_actions() {
            let v = a.verification();
            assert!(v.min_requests >= 1, "a silent service never counts as verified");
            assert!(v.observe_secs > 0);
            assert!(
                matches!(a, Action::DemoFaultsInject { .. }) || v.max_error_ratio.is_some(),
                "{a:?} must bound the error ratio"
            );
        }
    }

    #[test]
    fn new_actions_validate_to_typed_values() {
        assert_eq!(
            validate("demo.container.restart", "catalog-2", &json!({})).unwrap(),
            Action::DemoContainerRestart { container: DemoContainer::Catalog2 }
        );
        assert_eq!(
            validate("demo.replicas.scale", "catalog", &json!({ "replicas": 1 })).unwrap(),
            Action::DemoReplicasScale { service: DemoScalable::Catalog, replicas: 1 }
        );
        assert_eq!(
            validate("demo.feature_flag.set", "checkout-v2", &json!({ "enabled": 0 })).unwrap(),
            Action::DemoFeatureFlagSet { flag: DemoFlag::CheckoutV2, enabled: false }
        );
        assert_eq!(
            validate("demo.release.rollback", "orders", &Value::Null).unwrap(),
            Action::DemoReleaseRollback { service: DemoReleaseService::Orders }
        );
        let flag = validate("demo.feature_flag.set", "catalog-cache", &json!({ "enabled": 1 })).unwrap();
        assert_eq!(flag.params(), json!({ "enabled": 1 }));
        assert_eq!(flag.target(), "catalog-cache");
    }

    #[test]
    fn new_actions_reject_foreign_targets_and_out_of_range_params() {
        for (t, target, params) in [
            ("demo.container.restart", "demo-postgres", json!({})),
            ("demo.container.restart", "inventory", json!({})),
            ("demo.container.restart", "catalog-4", json!({})),
            ("demo.container.restart", "--privileged", json!({})),
            ("demo.container.restart", "gateway", json!({ "force": 1 })),
            ("demo.replicas.scale", "catalog", json!({})),
            ("demo.replicas.scale", "catalog", json!({ "replicas": 0 })),
            ("demo.replicas.scale", "catalog", json!({ "replicas": 4 })),
            ("demo.replicas.scale", "catalog", json!({ "replicas": 1.5 })),
            ("demo.replicas.scale", "catalog", json!({ "replicas": "2" })),
            ("demo.replicas.scale", "orders", json!({ "replicas": 2 })),
            ("demo.feature_flag.set", "checkout-v2", json!({})),
            ("demo.feature_flag.set", "checkout-v2", json!({ "enabled": 0.5 })),
            ("demo.feature_flag.set", "checkout-v2", json!({ "enabled": 2 })),
            ("demo.feature_flag.set", "checkout-v2", json!({ "enabled": true })),
            ("demo.feature_flag.set", "checkout-v3", json!({ "enabled": 1 })),
            ("demo.feature_flag.set", "", json!({ "enabled": 1 })),
            ("demo.release.rollback", "gateway", json!({})),
            ("demo.release.rollback", "orders", json!({ "version": "0.0.1" })),
        ] {
            assert!(validate(t, target, &params).is_err(), "{t} {target} {params}");
        }
    }

    #[test]
    fn restart_is_verified_on_the_restarted_instance() {
        let v = Action::DemoContainerRestart { container: DemoContainer::Catalog3 }.verification();
        assert_eq!((v.service, v.instance), ("catalog", Some("catalog-3")));
        let (ok, detail) = v.judge(&obs(0, 0));
        assert!(!ok && detail.contains("catalog instance catalog-3 received 0 requests"), "{detail}");
        assert!(v.judge(&obs(20, 1)).0);
        assert!(!v.judge(&obs(20, 2)).0);
        let gw = Action::DemoContainerRestart { container: DemoContainer::Gateway }.verification();
        assert_eq!((gw.service, gw.instance), ("gateway", Some("gateway")));
    }

    #[test]
    fn scaling_is_verified_by_the_number_of_serving_instances() {
        let v = Action::DemoReplicasScale { service: DemoScalable::Catalog, replicas: 2 }.verification();
        assert_eq!(v.expect_instances, Some(2));
        let o = |instances| Observed { requests: 30, errors: 0, instances, other_version_requests: 0 };
        let (ok, detail) = v.judge(&o(2));
        assert!(ok && detail.contains("across 2 instances"), "{detail}");
        for wrong in [0, 1, 3] {
            let (ok, detail) = v.judge(&o(wrong));
            assert!(!ok && detail.contains("expected 2"), "{wrong}: {detail}");
        }
        assert!(!v.judge(&Observed { errors: 2, ..o(2) }).0, "error ratio still applies");
    }

    #[test]
    fn flags_are_verified_on_the_service_that_owns_them() {
        let on = |flag| Action::DemoFeatureFlagSet { flag, enabled: true }.verification();
        assert_eq!(on(DemoFlag::CatalogCache).service, "gateway");
        assert_eq!(on(DemoFlag::CheckoutV2).service, "orders");
        let (ok, detail) = on(DemoFlag::CheckoutV2).judge(&obs(50, 10));
        assert!(!ok && detail.contains("error ratio 20.0% exceeds 5%"), "a breaking flag fails: {detail}");
    }

    #[test]
    fn rollback_is_verified_against_the_reported_allowlisted_release() {
        let a = Action::DemoReleaseRollback { service: DemoReleaseService::Orders };
        assert_eq!(a.verification().expect_version, None);
        let v = a.verification_after(&json!({ "from_version": "0.2.0", "to_version": "0.1.0" })).unwrap();
        assert_eq!(v.expect_version, Some("0.1.0"));
        let clean = Observed { requests: 40, errors: 1, instances: 1, other_version_requests: 0 };
        let (ok, detail) = v.judge(&clean);
        assert!(ok && detail.starts_with("orders 0.1.0 served 40 requests"), "{detail}");
        let (ok, detail) = v.judge(&Observed { other_version_requests: 3, ..clean });
        assert!(!ok && detail.contains("3 requests from another release"), "{detail}");
        assert!(!v.judge(&Observed { requests: 0, ..clean }).0);

        for bad in
            [json!({}), json!({ "to_version": "9.9.9" }), json!({ "to_version": 1 }), json!({ "to_version": "0.1.0 " })]
        {
            assert!(a.verification_after(&bad).is_err(), "{bad}");
        }
        let other = Action::DemoContainerRestart { container: DemoContainer::Orders };
        assert_eq!(other.verification_after(&json!({ "to_version": "0.2.0" })).unwrap(), other.verification());
    }

    #[test]
    fn demo_releases_are_an_exact_allowlist() {
        assert_eq!(demo_release("0.1.0"), Some("0.1.0"));
        for bad in ["", "0.1", "0.1.0-rc1", "v0.1.0", "0.2.0\n"] {
            assert_eq!(demo_release(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn container_and_flag_names_serialize_as_their_targets() {
        for c in DemoContainer::ALL {
            assert_eq!(serde_json::to_value(c).unwrap(), json!(c.as_str()));
        }
        for f in DemoFlag::ALL {
            assert_eq!(serde_json::to_value(f).unwrap(), json!(f.as_str()));
        }
    }

    #[test]
    fn boolean_params_are_integer_switches() {
        for s in CATALOG {
            for p in s.params.iter().filter(|p| p.boolean) {
                assert!(p.integer && p.min == 0.0 && p.max == 1.0, "{} {}", s.action_type, p.name);
            }
        }
    }

    #[test]
    fn actions_serialize_with_a_type_tag() {
        let a = Action::DemoFaultsInject { service: DemoService::Gateway, latency_ms: 5, error_rate: 0.5 };
        let v = serde_json::to_value(&a).unwrap();
        assert_eq!(v["type"], "demo_faults_inject");
        assert_eq!(v["service"], "gateway");
        assert_eq!(serde_json::from_value::<Action>(v).unwrap(), a);
        assert!(serde_json::from_value::<Action>(json!({ "type": "shell_exec", "cmd": "id" })).is_err());
        assert!(serde_json::from_value::<Action>(json!({ "type": "demo_faults_clear", "service": "db" })).is_err());
    }
}
