//! Configuration and connectivity checks. Never prints token values.

use std::path::Path;

use serde_json::{Value, json};

use crate::client::{self, Client};
use crate::{Conn, Outcome, discover};

fn check(name: &str, ok: bool, detail: impl Into<String>) -> Value {
    json!({ "check": name, "ok": ok, "detail": detail.into() })
}

pub fn run(path: &Path, conn: &Conn) -> anyhow::Result<Outcome> {
    let mut checks = Vec::new();
    let mut required_ok = true;

    match discover::discover(path) {
        Ok(d) => checks.push(check(
            "project",
            true,
            format!(
                "{:?} project{}{}",
                d.ecosystem,
                d.package_name.map(|n| format!(" `{n}`")).unwrap_or_default(),
                if d.controlhub_register_in_start {
                    ", SDK preloaded in start script"
                } else {
                    ", not integrated yet (controlhub plan)"
                }
            ),
        )),
        Err(e) => checks.push(check("project", false, e.to_string())),
    }

    let client = match Client::new(conn) {
        Ok(c) => c,
        Err(e) => {
            checks.push(check("urls", false, e.to_string()));
            return Ok(Outcome { json: json!({ "ok": false, "checks": checks }), code: 1 });
        }
    };

    let api_health = client.status(&format!("{}/api/v1/health", client.api_url));
    let api_up = matches!(api_health, Ok(200));
    required_ok &= api_up;
    checks.push(check(
        "api reachable",
        api_up,
        format!("{} → {:?}", client.api_url, api_health.map_err(|e| e.to_string())),
    ));

    let ingest_health = client.status(&format!("{}/health", client.ingest_url));
    let ingest_up = matches!(ingest_health, Ok(200));
    required_ok &= ingest_up;
    checks.push(check(
        "ingest reachable",
        ingest_up,
        format!("{} → {:?}", client.ingest_url, ingest_health.map_err(|e| e.to_string())),
    ));

    match client::api_token() {
        Ok(Some(t)) if api_up => match client.scope(&t) {
            Ok(s) => checks.push(check(
                "api token",
                true,
                format!("read-only access to {}/{}", s.project_id, s.environment_id),
            )),
            Err(e) => {
                required_ok = false;
                checks.push(check("api token", false, e.to_string()));
            }
        },
        Ok(Some(_)) => checks.push(check("api token", false, "set, but the API is unreachable")),
        Ok(None) => checks.push(check(
            "api token",
            false,
            format!("{} not set (needed by verify/context)", client::API_TOKEN_ENV),
        )),
        Err(e) => {
            required_ok = false;
            checks.push(check("api token", false, e.to_string()));
        }
    }

    match client::ingest_token() {
        // An empty batch is rejected with 400 after authentication, so nothing is written.
        Ok(Some(t)) if ingest_up => match client.post_events(&t, &json!({ "events": [] })) {
            Ok((400, _)) => checks.push(check("ingest token", true, "accepted by ingestion")),
            Ok((401, _)) => {
                required_ok = false;
                checks.push(check("ingest token", false, "rejected (expired, revoked or invalid)"));
            }
            Ok((s, _)) => checks.push(check("ingest token", false, format!("unexpected status {s}"))),
            Err(e) => checks.push(check("ingest token", false, e.to_string())),
        },
        Ok(Some(_)) => checks.push(check("ingest token", false, "set, but ingestion is unreachable")),
        Ok(None) => checks.push(check(
            "ingest token",
            false,
            format!("{} not set (needed by verify and the SDK)", client::INGEST_TOKEN_ENV),
        )),
        Err(e) => {
            required_ok = false;
            checks.push(check("ingest token", false, e.to_string()));
        }
    }

    Ok(Outcome { json: json!({ "ok": required_ok, "checks": checks }), code: if required_ok { 0 } else { 1 } })
}
