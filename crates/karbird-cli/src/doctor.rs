//! Configuration and connectivity checks. Never prints token values.

use std::path::Path;

use serde_json::{Value, json};

use crate::client::{self, Client, Secret};
use crate::{Conn, Outcome, discover};

fn check(name: &str, ok: bool, detail: impl Into<String>) -> Value {
    json!({ "check": name, "ok": ok, "detail": detail.into() })
}

pub fn run(path: &Path, conn: &Conn) -> anyhow::Result<Outcome> {
    run_with(path, conn, client::api_token(), client::ingest_token())
}

fn run_with(
    path: &Path,
    conn: &Conn,
    api_token: anyhow::Result<Option<Secret>>,
    ingest_token: anyhow::Result<Option<Secret>>,
) -> anyhow::Result<Outcome> {
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
                if d.karbird_register_in_start {
                    ", SDK preloaded in start script"
                } else {
                    ", not integrated yet (karbird plan)"
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

    match api_token {
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

    match ingest_token {
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::test_http::{self, Seen};

    const API_SECRET: &str = "cha_doctor_secret_value";
    const INGEST_SECRET: &str = "chi_doctor_secret_value";

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), r#"{"name":"shop","scripts":{"start":"node a.js"}}"#).unwrap();
        dir
    }

    fn conn(url: &str) -> Conn {
        Conn { api_url: url.into(), ingest_url: url.into() }
    }

    fn check<'a>(o: &'a Outcome, name: &str) -> &'a Value {
        o.json["checks"].as_array().unwrap().iter().find(|c| c["check"] == name).unwrap()
    }

    fn tokens() -> (anyhow::Result<Option<Secret>>, anyhow::Result<Option<Secret>>) {
        (Ok(Some(Secret::for_test(API_SECRET))), Ok(Some(Secret::for_test(INGEST_SECRET))))
    }

    /// Healthy API + ingestion; `session`/`batch` statuses are configurable.
    fn stack(session_status: u16, batch_status: u16) -> String {
        test_http::serve(move |r: &Seen| match r.target.as_str() {
            "/api/v1/health" | "/health" => (200, "{}".into()),
            "/api/v1/session" => (session_status, test_http::session_json()),
            "/api/v1/events/batch" => (batch_status, "{}".into()),
            _ => (404, "{}".into()),
        })
        .0
    }

    fn assert_no_secrets(o: &Outcome) {
        let s = o.json.to_string();
        assert!(!s.contains(API_SECRET) && !s.contains(INGEST_SECRET), "{s}");
    }

    #[test]
    fn healthy_setup_passes_without_printing_tokens() {
        let url = stack(200, 400);
        let (a, i) = tokens();
        let o = run_with(project().path(), &conn(&url), a, i).unwrap();
        assert_eq!((o.code, &o.json["ok"]), (0, &json!(true)), "{}", o.json);
        assert_eq!(check(&o, "api token")["detail"], "read-only access to demo/local-demo");
        assert_eq!(check(&o, "ingest token")["detail"], "accepted by ingestion");
        assert!(check(&o, "project")["detail"].as_str().unwrap().contains("`shop`"));
        assert_no_secrets(&o);
    }

    #[test]
    fn the_ingest_probe_sends_an_empty_batch_with_the_bearer_token() {
        let (url, log) = test_http::serve(|r: &Seen| match r.target.as_str() {
            "/api/v1/events/batch" => (400, "{}".into()),
            "/api/v1/session" => (200, test_http::session_json()),
            _ => (200, "{}".into()),
        });
        let (a, i) = tokens();
        run_with(project().path(), &conn(&url), a, i).unwrap();
        let log = log.lock().unwrap();
        let probe = log.iter().find(|s| s.target == "/api/v1/events/batch").unwrap();
        assert_eq!(probe.method, "POST");
        assert_eq!(serde_json::from_str::<Value>(&probe.body).unwrap(), json!({ "events": [] }), "writes nothing");
        assert_eq!(probe.authorization.as_deref(), Some(format!("Bearer {INGEST_SECRET}").as_str()));
        let health = log.iter().find(|s| s.target == "/api/v1/health").unwrap();
        assert_eq!(health.authorization, None, "health checks are unauthenticated");
    }

    #[test]
    fn rejected_tokens_fail_the_doctor() {
        let (a, i) = tokens();
        let o = run_with(project().path(), &conn(&stack(200, 401)), a, i).unwrap();
        assert_eq!(o.code, 1);
        assert_eq!(check(&o, "ingest token")["ok"], false);
        assert!(check(&o, "ingest token")["detail"].as_str().unwrap().contains("rejected"));

        let (a, i) = tokens();
        let o = run_with(project().path(), &conn(&stack(401, 400)), a, i).unwrap();
        assert_eq!(o.code, 1);
        assert!(check(&o, "api token")["detail"].as_str().unwrap().contains("API token rejected"));
        assert_no_secrets(&o);
    }

    #[test]
    fn unexpected_ingest_status_is_reported_but_not_fatal() {
        let (a, i) = tokens();
        let o = run_with(project().path(), &conn(&stack(200, 500)), a, i).unwrap();
        assert_eq!(check(&o, "ingest token")["detail"], "unexpected status 500");
        assert_eq!(o.code, 0);
    }

    #[test]
    fn missing_tokens_are_reported_but_optional() {
        let o = run_with(project().path(), &conn(&stack(200, 400)), Ok(None), Ok(None)).unwrap();
        assert_eq!(o.code, 0);
        assert!(check(&o, "api token")["detail"].as_str().unwrap().contains("KARBIRD_API_TOKEN not set"));
        assert!(check(&o, "ingest token")["detail"].as_str().unwrap().contains("KARBIRD_INGEST_TOKEN not set"));
    }

    #[test]
    fn malformed_token_variables_fail_the_doctor() {
        let o = run_with(
            project().path(),
            &conn(&stack(200, 400)),
            Err(anyhow::anyhow!("KARBIRD_API_TOKEN must be a `cha_…` token")),
            Ok(None),
        )
        .unwrap();
        assert_eq!(o.code, 1);
        assert_eq!(check(&o, "api token")["ok"], false);
    }

    #[test]
    fn unreachable_services_fail_and_tokens_are_not_sent_anywhere() {
        let (a, i) = tokens();
        let o = run_with(project().path(), &conn("http://127.0.0.1:1"), a, i).unwrap();
        assert_eq!((o.code, &o.json["ok"]), (1, &json!(false)));
        assert_eq!(check(&o, "api reachable")["ok"], false);
        assert_eq!(check(&o, "ingest reachable")["ok"], false);
        assert_eq!(check(&o, "api token")["detail"], "set, but the API is unreachable");
        assert_eq!(check(&o, "ingest token")["detail"], "set, but ingestion is unreachable");
        assert_no_secrets(&o);
    }

    #[test]
    fn plain_http_to_remote_hosts_stops_before_any_request() {
        let (a, i) = tokens();
        let o = run_with(project().path(), &conn("http://hub.example.com"), a, i).unwrap();
        assert_eq!(o.code, 1);
        assert_eq!(check(&o, "urls")["ok"], false);
        assert!(check(&o, "urls")["detail"].as_str().unwrap().contains("use https"));
        assert!(o.json["checks"].as_array().unwrap().iter().all(|c| c["check"] != "api reachable"));
    }

    #[test]
    fn a_missing_project_directory_is_a_failed_check_not_an_error() {
        let o = run_with(Path::new("definitely/not/here"), &conn(&stack(200, 400)), Ok(None), Ok(None)).unwrap();
        assert_eq!(check(&o, "project")["ok"], false);
    }
}
