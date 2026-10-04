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
/// At most this many plans per environment per hour (proposed or later).
pub const MAX_PLANS_PER_HOUR: i64 = 30;
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
}

#[derive(Debug, Clone, Serialize)]
pub struct ParamSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub min: f64,
    pub max: f64,
    pub integer: bool,
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
            },
            ParamSpec {
                name: "error_rate",
                description: "Share of requests that fail (0–1)",
                min: 0.0,
                max: 1.0,
                integer: false,
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
];

pub fn spec(action_type: &str) -> Option<&'static ActionSpec> {
    CATALOG.iter().find(|s| s.action_type == action_type)
}

impl Action {
    pub fn action_type(&self) -> &'static str {
        match self {
            Action::DemoFaultsClear { .. } => "demo.faults.clear",
            Action::DemoFaultsInject { .. } => "demo.faults.inject",
            Action::DemoRestartDatabase => "demo.restart_database",
        }
    }

    pub fn target(&self) -> &'static str {
        match self {
            Action::DemoFaultsClear { service } | Action::DemoFaultsInject { service, .. } => service.as_str(),
            Action::DemoRestartDatabase => "demo-postgres",
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
            _ => serde_json::json!({}),
        }
    }

    /// Telemetry check after execution.
    pub fn verification(&self) -> Verification {
        match self {
            Action::DemoFaultsClear { service } => Verification {
                service: service.as_str(),
                observe_secs: 15,
                min_requests: 1,
                max_error_ratio: Some(0.05),
            },
            Action::DemoFaultsInject { service, .. } => {
                Verification { service: service.as_str(), observe_secs: 15, min_requests: 1, max_error_ratio: None }
            }
            Action::DemoRestartDatabase => {
                Verification { service: "orders", observe_secs: 15, min_requests: 1, max_error_ratio: Some(0.2) }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Verification {
    pub service: &'static str,
    pub observe_secs: u64,
    pub min_requests: u64,
    pub max_error_ratio: Option<f64>,
}

impl Verification {
    /// Judges observed request/error counts in the window after the action.
    pub fn judge(&self, requests: u64, errors: u64) -> (bool, String) {
        if requests < self.min_requests {
            return (false, format!("{} received {requests} requests (need ≥ {})", self.service, self.min_requests));
        }
        let ratio = errors as f64 / requests as f64;
        match self.max_error_ratio {
            Some(max) if ratio > max => {
                (false, format!("{} error ratio {:.1}% exceeds {:.0}%", self.service, ratio * 100.0, max * 100.0))
            }
            _ => (true, format!("{} served {requests} requests, error ratio {:.1}%", self.service, ratio * 100.0)),
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
    /// Plans created in this environment during the last hour.
    pub plans_last_hour: i64,
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
    /// Always true: no action runs without a human approval.
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
    let recent = input.plans_last_hour - own;
    checks.push(PolicyCheck {
        name: "rate limit".into(),
        passed: recent < MAX_PLANS_PER_HOUR,
        detail: format!("{recent} plan(s) in the last hour (max {MAX_PLANS_PER_HOUR})"),
    });
    PolicyDecision { allowed: checks.iter().all(|c| c.passed), requires_approval: true, checks }
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

    #[test]
    fn every_action_has_a_catalog_entry() {
        for a in [
            Action::DemoFaultsClear { service: DemoService::Orders },
            Action::DemoFaultsInject { service: DemoService::Gateway, latency_ms: 1, error_rate: 0.0 },
            Action::DemoRestartDatabase,
        ] {
            assert_eq!(a.spec().action_type, a.action_type());
            assert!(a.spec().targets.contains(&a.target()));
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
        let ok = PolicyInput { environment_is_demo: true, open_plans_for_target: 0, plans_last_hour: 0 };
        assert!(evaluate(&a, &ok, false).allowed);
        assert!(evaluate(&a, &ok, false).requires_approval);
        assert!(!evaluate(&a, &PolicyInput { environment_is_demo: false, ..ok }, false).allowed);
        let busy = PolicyInput { open_plans_for_target: 1, ..ok };
        assert!(!evaluate(&a, &busy, false).allowed);
        assert!(evaluate(&a, &busy, true).allowed, "the plan itself does not block its own approval");
        assert!(!evaluate(&a, &PolicyInput { plans_last_hour: MAX_PLANS_PER_HOUR, ..ok }, false).allowed);
    }

    #[test]
    fn verification_judgement() {
        let v = Action::DemoFaultsClear { service: DemoService::Orders }.verification();
        assert!(!v.judge(0, 0).0);
        assert!(v.judge(100, 5).0);
        assert!(!v.judge(100, 6).0);
        let inject =
            Action::DemoFaultsInject { service: DemoService::Orders, latency_ms: 0, error_rate: 1.0 }.verification();
        assert!(inject.judge(10, 10).0, "errors are expected after injecting them");
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
        let base = PolicyInput { environment_is_demo: true, open_plans_for_target: 0, plans_last_hour: 0 };
        let rate = |n| evaluate(&a, &PolicyInput { plans_last_hour: n, ..base }, false).allowed;
        assert!(rate(MAX_PLANS_PER_HOUR - 1));
        assert!(!rate(MAX_PLANS_PER_HOUR));
        // Re-evaluation excludes the plan itself from both counters.
        let own = PolicyInput { open_plans_for_target: 1, plans_last_hour: MAX_PLANS_PER_HOUR, ..base };
        assert!(evaluate(&a, &own, true).allowed);
        assert!(!evaluate(&a, &PolicyInput { open_plans_for_target: 2, ..own }, true).allowed);

        let denied = evaluate(&a, &PolicyInput { environment_is_demo: false, ..base }, false);
        assert!(!denied.allowed && denied.requires_approval);
        let failed: Vec<_> = denied.checks.iter().filter(|c| !c.passed).map(|c| c.name.as_str()).collect();
        assert_eq!(failed, ["demo environment"]);
        assert_eq!(denied.checks.len(), 4);
    }

    proptest! {
        /// Approval is always required and `allowed` is exactly "all checks passed".
        #[test]
        fn policy_decision_is_conjunction_of_checks(
            demo in any::<bool>(), open in -2i64..5, recent in -2i64..100, own in any::<bool>(),
        ) {
            let input = PolicyInput { environment_is_demo: demo, open_plans_for_target: open, plans_last_hour: recent };
            for a in [
                Action::DemoFaultsClear { service: DemoService::Gateway },
                Action::DemoFaultsInject { service: DemoService::Orders, latency_ms: 10, error_rate: 0.1 },
                Action::DemoRestartDatabase,
            ] {
                let d = evaluate(&a, &input, own);
                prop_assert!(d.requires_approval);
                prop_assert_eq!(d.allowed, d.checks.iter().all(|c| c.passed));
                if !demo { prop_assert!(!d.allowed, "demo-only actions never pass outside demo"); }
                let o = i64::from(own);
                prop_assert_eq!(d.allowed, demo && open - o <= 0 && recent - o < MAX_PLANS_PER_HOUR);
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
        assert!(db.judge(10, 2).0, "exactly 20% passes");
        assert!(!db.judge(10, 3).0);
        let (ok, detail) = db.judge(0, 0);
        assert!(!ok && detail.contains("0 requests"), "{detail}");
        let clear = Action::DemoFaultsClear { service: DemoService::Gateway }.verification();
        assert_eq!(clear.service, "gateway");
        assert!(clear.judge(20, 1).0, "exactly 5% passes");
        assert!(!clear.judge(19, 1).0);
        for a in [
            Action::DemoFaultsClear { service: DemoService::Orders },
            Action::DemoFaultsInject { service: DemoService::Orders, latency_ms: 1, error_rate: 0.0 },
            Action::DemoRestartDatabase,
        ] {
            let v = a.verification();
            assert!(v.min_requests >= 1, "a silent service never counts as verified");
            assert!(v.observe_secs > 0);
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
