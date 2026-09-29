//! End-to-end verification: send one event through ingestion, wait until the API returns it,
//! then check whether the application itself has sent telemetry.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::client::{self, Client, now_ns};
use crate::{Conn, Outcome, discover, plan};

const WINDOW_NS: i64 = 15 * 60 * 1_000_000_000;

pub fn run(
    path: &Path,
    service_name: Option<String>,
    timeout_secs: u64,
    require_app: bool,
    conn: &Conn,
) -> anyhow::Result<Outcome> {
    let api_token = client::api_token()?.ok_or_else(|| anyhow!("{} is required", client::API_TOKEN_ENV))?;
    let ingest_token = client::ingest_token()?.ok_or_else(|| anyhow!("{} is required", client::INGEST_TOKEN_ENV))?;
    let service = match service_name {
        Some(s) => s,
        None => discover::discover(path)?
            .package_name
            .as_deref()
            .and_then(plan::sanitize_service_name)
            .ok_or_else(|| anyhow!("cannot derive the service name; pass --service-name"))?,
    };
    let client = Client::new(conn)?;
    let scope = client.scope(&api_token)?;

    let verify_id = Uuid::new_v4().to_string();
    let sent_at = Instant::now();
    let (status, ack) = client.post_events(
        &ingest_token,
        &json!({ "events": [{
            "event_id": verify_id,
            "event_type": "controlhub.verify",
            "service": { "name": service },
            "event_time": now_ns().to_string(),
            "attributes": { "verify_id": verify_id },
        }] }),
    )?;
    let broker_ack = status == 200 && ack.get("durability").and_then(Value::as_str) == Some("broker_acknowledged");
    if !broker_ack {
        bail!("ingestion did not acknowledge the verification event (status {status}): {ack}");
    }
    eprintln!("verification event acknowledged by the broker; waiting for it to become queryable…");

    let deadline = sent_at + Duration::from_secs(timeout_secs);
    let mut visible_after_ms = None;
    while Instant::now() < deadline {
        let now = now_ns();
        let events = client.api_json(
            &api_token,
            &scope.path("events"),
            &[
                ("from", (now - WINDOW_NS).to_string()),
                ("to", (now + 60_000_000_000).to_string()),
                ("limit", "1000".into()),
            ],
        )?;
        let found = events["events"].as_array().is_some_and(|evs| {
            evs.iter().any(|e| {
                e["id"] == verify_id.as_str()
                    || e.pointer("/attributes/verify_id") == Some(&Value::String(verify_id.clone()))
            })
        });
        if found {
            visible_after_ms = Some(sent_at.elapsed().as_millis() as u64);
            break;
        }
        std::thread::sleep(Duration::from_millis(1000));
    }
    let pipeline_ok = visible_after_ms.is_some();

    // Application telemetry is exported in batches, so keep looking until the deadline when it is required.
    let (app, requests, logs) = loop {
        let now = now_ns();
        let services = client.api_json(
            &api_token,
            &scope.path("services"),
            &[("from", (now - WINDOW_NS).to_string()), ("to", now.to_string())],
        )?;
        let app =
            services["services"].as_array().and_then(|s| s.iter().find(|x| x["name"] == service.as_str())).cloned();
        let requests = app.as_ref().and_then(|a| a["requests"].as_u64()).unwrap_or(0);
        let logs = app.as_ref().and_then(|a| a["logs"].as_u64()).unwrap_or(0);
        if requests > 0 || logs > 0 || !require_app || Instant::now() >= deadline {
            break (app, requests, logs);
        }
        std::thread::sleep(Duration::from_millis(1000));
    };
    let app_observed = requests > 0 || logs > 0;
    if !app_observed {
        eprintln!(
            "no spans or logs from `{service}` in the last 15 minutes: start the app with its start script and send it a request"
        );
    }

    let ok = pipeline_ok && (app_observed || !require_app);
    Ok(Outcome {
        json: json!({
            "ok": ok,
            "scope": { "project_id": scope.project_id, "environment_id": scope.environment_id },
            "service_name": service,
            "pipeline": { "broker_acknowledged": broker_ack, "visible_after_ms": visible_after_ms, "timeout_secs": timeout_secs },
            "app": {
                "observed": app_observed,
                "requests": requests,
                "logs": logs,
                "signals": app.as_ref().map(|a| a["signals"].clone()),
                "last_seen_ns": app.as_ref().map(|a| a["last_seen_ns"].clone()),
            },
        }),
        code: if ok { 0 } else { 1 },
    })
}
