//! End-to-end verification: send one event through ingestion, wait until the API returns it,
//! then check whether the application itself has sent telemetry.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::client::{self, Client, Secret, now_ns};
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
    run_with(path, service_name, timeout_secs, require_app, conn, &api_token, &ingest_token)
}

fn run_with(
    path: &Path,
    service_name: Option<String>,
    timeout_secs: u64,
    require_app: bool,
    conn: &Conn,
    api_token: &Secret,
    ingest_token: &Secret,
) -> anyhow::Result<Outcome> {
    let service = match service_name {
        Some(s) => s,
        None => discover::discover(path)?
            .package_name
            .as_deref()
            .and_then(plan::sanitize_service_name)
            .ok_or_else(|| anyhow!("cannot derive the service name; pass --service-name"))?,
    };
    let client = Client::new(conn)?;
    let scope = client.scope(api_token)?;

    let verify_id = Uuid::new_v4().to_string();
    let sent_at = Instant::now();
    let (status, ack) = client.post_events(
        ingest_token,
        &json!({ "events": [{
            "event_id": verify_id,
            "event_type": "karbird.verify",
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
            api_token,
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
            api_token,
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::test_http::{self, Seen};

    #[derive(Clone, Copy)]
    struct Fake {
        ingest_status: u16,
        durability: &'static str,
        session_status: u16,
        /// Whether the events endpoint returns the posted verification event.
        visible: bool,
        app_requests: u64,
    }

    const OK: Fake = Fake {
        ingest_status: 200,
        durability: "broker_acknowledged",
        session_status: 200,
        visible: true,
        app_requests: 3,
    };

    fn stack(f: Fake) -> (Conn, test_http::Log) {
        let posted: Arc<Mutex<Option<String>>> = Arc::default();
        let (url, log) = test_http::serve(move |r: &Seen| {
            let path = r.target.split('?').next().unwrap_or_default();
            match path {
                "/api/v1/session" => (f.session_status, test_http::session_json()),
                "/api/v1/events/batch" => {
                    let body: Value = serde_json::from_str(&r.body).unwrap_or_default();
                    *posted.lock().unwrap() = body.pointer("/events/0/event_id").and_then(Value::as_str).map(String::from);
                    (f.ingest_status, json!({ "durability": f.durability }).to_string())
                }
                "/api/v1/projects/demo/environments/local-demo/events" => {
                    let id = posted.lock().unwrap().clone().filter(|_| f.visible).unwrap_or_default();
                    (200, json!({ "events": [{ "id": "other" }, { "id": id }] }).to_string())
                }
                "/api/v1/projects/demo/environments/local-demo/services" => (
                    200,
                    json!({ "services": [
                        { "name": "someone-else", "requests": 99, "logs": 99 },
                        { "name": "shop", "requests": f.app_requests, "logs": 0, "signals": ["traces"], "last_seen_ns": 5 },
                    ] })
                    .to_string(),
                ),
                _ => (404, "{}".into()),
            }
        });
        (Conn { api_url: url.clone(), web_url: None, ingest_url: url }, log)
    }

    fn verify(f: Fake, timeout: u64, require_app: bool) -> anyhow::Result<Outcome> {
        let (conn, _) = stack(f);
        run_with(
            Path::new("."),
            Some("shop".into()),
            timeout,
            require_app,
            &conn,
            &Secret::for_test("cha_verify_secret"),
            &Secret::for_test("chi_verify_secret"),
        )
    }

    #[test]
    fn a_visible_event_and_app_traffic_pass() {
        let o = verify(OK, 10, true).unwrap();
        assert_eq!(o.code, 0, "{}", o.json);
        assert_eq!(o.json["pipeline"]["broker_acknowledged"], true);
        assert!(o.json["pipeline"]["visible_after_ms"].is_u64());
        assert_eq!(o.json["app"]["requests"], 3, "counts come from the named service only");
        assert_eq!(o.json["scope"], json!({ "project_id": "demo", "environment_id": "local-demo" }));
        assert!(!o.json.to_string().contains("verify_secret"));
    }

    #[test]
    fn the_verification_event_is_well_formed_and_uses_the_ingest_token() {
        let (conn, log) = stack(OK);
        run_with(
            Path::new("."),
            Some("shop".into()),
            10,
            false,
            &conn,
            &Secret::for_test("cha_a"),
            &Secret::for_test("chi_i"),
        )
        .unwrap();
        let log = log.lock().unwrap();
        let post = log.iter().find(|s| s.method == "POST").unwrap();
        assert_eq!(post.authorization.as_deref(), Some("Bearer chi_i"));
        let ev: Value = serde_json::from_str(&post.body).unwrap();
        let ev = &ev["events"][0];
        assert_eq!(ev["event_type"], "karbird.verify");
        assert_eq!(ev["service"]["name"], "shop");
        assert_eq!(ev["attributes"]["verify_id"], ev["event_id"]);
        assert!(Uuid::parse_str(ev["event_id"].as_str().unwrap()).is_ok());
        assert!(ev["event_time"].as_str().unwrap().parse::<i64>().is_ok(), "nanoseconds as a string");
        assert!(log.iter().filter(|s| s.method == "GET").all(|s| s.authorization.as_deref() == Some("Bearer cha_a")));
    }

    #[test]
    fn missing_app_traffic_only_fails_when_required() {
        let quiet = Fake { app_requests: 0, ..OK };
        let o = verify(quiet, 10, false).unwrap();
        assert_eq!((o.code, &o.json["app"]["observed"]), (0, &json!(false)));
        let o = verify(quiet, 1, true).unwrap();
        assert_eq!(o.code, 1);
        assert_eq!(o.json["ok"], false);
    }

    #[test]
    fn an_event_that_never_becomes_visible_fails() {
        let o = verify(Fake { visible: false, ..OK }, 1, false).unwrap();
        assert_eq!(o.code, 1);
        assert_eq!(o.json["pipeline"]["visible_after_ms"], Value::Null);
        let o = verify(OK, 0, false).unwrap();
        assert_eq!(o.code, 1, "a zero timeout never polls");
    }

    #[test]
    fn ingestion_must_acknowledge_with_broker_durability() {
        for f in [
            Fake { ingest_status: 503, ..OK },
            Fake { durability: "accepted", ..OK },
            Fake { ingest_status: 202, ..OK },
        ] {
            let e = verify(f, 5, false).err().map(|e| e.to_string()).unwrap_or_default();
            assert!(e.contains("did not acknowledge"), "{e}");
        }
    }

    #[test]
    fn a_rejected_api_token_stops_before_sending_anything() {
        let (conn, log) = stack(Fake { session_status: 401, ..OK });
        let e = run_with(
            Path::new("."),
            Some("shop".into()),
            5,
            false,
            &conn,
            &Secret::for_test("cha_a"),
            &Secret::for_test("chi_i"),
        )
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
        assert!(e.contains("API token rejected"), "{e}");
        assert!(log.lock().unwrap().iter().all(|s| s.method != "POST"));
    }

    #[test]
    fn the_service_name_is_derived_from_the_project() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), r#"{"name":"shop"}"#).unwrap();
        let (conn, _) = stack(OK);
        let o = run_with(dir.path(), None, 10, true, &conn, &Secret::for_test("cha_a"), &Secret::for_test("chi_i"))
            .unwrap();
        assert_eq!((o.code, &o.json["service_name"]), (0, &json!("shop")));

        let empty = tempfile::tempdir().unwrap();
        std::fs::write(empty.path().join("package.json"), "{}").unwrap();
        let e = run_with(empty.path(), None, 1, false, &conn, &Secret::for_test("cha_a"), &Secret::for_test("chi_i"))
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(e.contains("--service-name"), "{e}");
    }

    #[test]
    fn plain_http_to_a_remote_host_is_refused() {
        let conn = Conn {
            api_url: "http://hub.example.com".into(),
            web_url: None,
            ingest_url: "http://hub.example.com".into(),
        };
        let r = run_with(
            Path::new("."),
            Some("s".into()),
            1,
            false,
            &conn,
            &Secret::for_test("cha_a"),
            &Secret::for_test("chi_i"),
        );
        assert!(r.is_err());
    }
}
