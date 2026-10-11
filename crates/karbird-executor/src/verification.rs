use karbird_actions::{Observed, Verification};
use serde_json::{Value, json};

/// Evaluate a verification result and build the durable record written by the executor.
pub fn verification_record(v: &Verification, o: &Observed, from_ns: i64, to_ns: i64) -> (bool, Value) {
    let (passed, detail) = v.judge(o);
    (
        passed,
        json!({
            "service": v.service, "instance": v.instance, "expected_version": v.expect_version,
            "window_from_ns": from_ns.to_string(), "window_to_ns": to_ns.to_string(),
            "requests": o.requests, "errors": o.errors, "max_error_ratio": v.max_error_ratio,
            "instances": o.instances, "expected_instances": v.expect_instances,
            "other_version_requests": o.other_version_requests,
            "passed": passed, "detail": detail,
        }),
    )
}
