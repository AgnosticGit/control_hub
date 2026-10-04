//! Minimal HTTP client for the karbird API (read-only bearer token) and ingestion.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, anyhow, bail};
use serde_json::Value;

use crate::Conn;

pub const API_TOKEN_ENV: &str = "KARBIRD_API_TOKEN";
pub const INGEST_TOKEN_ENV: &str = "KARBIRD_INGEST_TOKEN";

/// A secret that never appears in `Debug` output.
pub struct Secret(String);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(…)")
    }
}

impl Secret {
    #[cfg(test)]
    pub fn for_test(v: &str) -> Self {
        Self(v.to_owned())
    }

    fn from_env(name: &str, prefix: &str) -> anyhow::Result<Option<Self>> {
        match std::env::var(name) {
            Ok(v) if v.trim().is_empty() => Ok(None),
            Ok(v) => {
                let v = v.trim().to_owned();
                if !v.starts_with(prefix) {
                    bail!("{name} must be a `{prefix}…` token");
                }
                Ok(Some(Self(v)))
            }
            Err(_) => Ok(None),
        }
    }
}

pub fn api_token() -> anyhow::Result<Option<Secret>> {
    Secret::from_env(API_TOKEN_ENV, "cha_")
}

pub fn ingest_token() -> anyhow::Result<Option<Secret>> {
    Secret::from_env(INGEST_TOKEN_ENV, "chi_")
}

/// Tokens must not travel in cleartext outside the local machine.
pub fn check_url(url: &str) -> anyhow::Result<String> {
    let url = url.trim_end_matches('/');
    if let Some(rest) = url.strip_prefix("http://") {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
        let loopback = !authority.contains('@')
            && ["localhost", "127.0.0.1", "[::1]"]
                .iter()
                .any(|h| authority == *h || authority.strip_prefix(h).is_some_and(|p| p.starts_with(':')));
        if !loopback {
            bail!("refusing to send tokens over plain http to {authority}; use https");
        }
    } else if !url.starts_with("https://") {
        bail!("URL must start with http:// or https://: {url}");
    }
    Ok(url.to_owned())
}

pub fn now_ns() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as i64).unwrap_or_default()
}

/// Percent-encodes a path segment (IDs are validated server-side; this is defence in depth).
pub fn segment(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[derive(Debug)]
pub struct Client {
    agent: ureq::Agent,
    pub api_url: String,
    pub ingest_url: String,
}

#[derive(Debug, Clone)]
pub struct Scope {
    pub project_id: String,
    pub environment_id: String,
}

impl Scope {
    pub fn path(&self, resource: &str) -> String {
        format!(
            "/api/v1/projects/{}/environments/{}/{resource}",
            segment(&self.project_id),
            segment(&self.environment_id)
        )
    }
}

impl Client {
    pub fn new(conn: &Conn) -> anyhow::Result<Self> {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(15)))
            .http_status_as_error(false)
            .build()
            .into();
        Ok(Self { agent, api_url: check_url(&conn.api_url)?, ingest_url: check_url(&conn.ingest_url)? })
    }

    fn read(mut res: ureq::http::Response<ureq::Body>) -> anyhow::Result<(u16, Value)> {
        let status = res.status().as_u16();
        let body = res.body_mut().with_config().limit(16 * 1024 * 1024).read_to_string().unwrap_or_default();
        Ok((status, serde_json::from_str(&body).unwrap_or(Value::Null)))
    }

    /// Unauthenticated GET returning only the status (health checks).
    pub fn status(&self, url: &str) -> anyhow::Result<u16> {
        let res = self.agent.get(url).call().with_context(|| format!("cannot reach {url}"))?;
        Ok(res.status().as_u16())
    }

    pub fn api_get(&self, token: &Secret, path: &str, query: &[(&str, String)]) -> anyhow::Result<(u16, Value)> {
        let url = format!("{}{path}", self.api_url);
        let mut req = self.agent.get(&url).header("authorization", format!("Bearer {}", token.0));
        for (k, v) in query {
            req = req.query(*k, v);
        }
        Self::read(req.call().with_context(|| format!("cannot reach {}", self.api_url))?)
    }

    /// GET that must succeed with 200.
    pub fn api_json(&self, token: &Secret, path: &str, query: &[(&str, String)]) -> anyhow::Result<Value> {
        match self.api_get(token, path, query)? {
            (200, v) => Ok(v),
            (401, _) => Err(anyhow!("API token rejected (expired, revoked or invalid)")),
            (s, v) => {
                Err(anyhow!("GET {path} failed with {s}: {}", v.get("message").and_then(Value::as_str).unwrap_or("")))
            }
        }
    }

    pub fn post_events(&self, token: &Secret, body: &Value) -> anyhow::Result<(u16, Value)> {
        let url = format!("{}/api/v1/events/batch", self.ingest_url);
        let res = self
            .agent
            .post(&url)
            .header("authorization", format!("Bearer {}", token.0))
            .send_json(body)
            .with_context(|| format!("cannot reach {}", self.ingest_url))?;
        Self::read(res)
    }

    /// The environment an API token is bound to.
    pub fn scope(&self, token: &Secret) -> anyhow::Result<Scope> {
        let session = self.api_json(token, "/api/v1/session", &[])?;
        let project = session.pointer("/projects/0").ok_or_else(|| anyhow!("API token has no project access"))?;
        let env = project.pointer("/environments/0/id").and_then(Value::as_str);
        match (project.get("id").and_then(Value::as_str), env) {
            (Some(p), Some(e)) => Ok(Scope { project_id: p.to_owned(), environment_id: e.to_owned() }),
            _ => Err(anyhow!("API token has no environment access")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_http_only_for_loopback() {
        assert!(check_url("http://127.0.0.1:8080").is_ok());
        assert!(check_url("http://localhost:8080/").is_ok());
        assert!(check_url("https://hub.example.com").is_ok());
        assert!(check_url("http://hub.example.com").is_err());
        assert!(check_url("http://127.0.0.1.evil.example").is_err());
        assert!(check_url("http://127.0.0.1@evil.example").is_err());
        assert!(check_url("http://127.0.0.1:80@evil.example").is_err());
        assert!(check_url("http://[::1]:8080").is_ok());
        assert!(check_url("http://[2001:db8::1]:8080").is_err());
        assert!(check_url("ftp://x").is_err());
    }

    #[test]
    fn segments_are_encoded() {
        assert_eq!(segment("local-demo"), "local-demo");
        assert_eq!(segment("a/b?c"), "a%2Fb%3Fc");
    }

    #[test]
    fn segments_encode_traversal_and_non_ascii() {
        assert_eq!(segment(".."), "..", "dots are unreserved; the server rejects `..` ids");
        assert_eq!(segment("../x"), "..%2Fx");
        assert_eq!(segment("a b#c%d"), "a%20b%23c%25d");
        assert_eq!(segment("é"), "%C3%A9");
        assert_eq!(segment(""), "");
    }

    #[test]
    fn scope_paths_encode_both_ids() {
        let s = Scope { project_id: "p/1".into(), environment_id: "e?x".into() };
        assert_eq!(s.path("traces"), "/api/v1/projects/p%2F1/environments/e%3Fx/traces");
    }

    #[test]
    fn secrets_never_show_in_debug_output() {
        let s = Secret::for_test("cha_super_secret");
        assert_eq!(format!("{s:?}"), "Secret(…)");
        assert!(!format!("{:?}", Some(&s)).contains("super_secret"));
    }

    #[test]
    fn urls_are_trimmed_and_schemes_checked() {
        assert_eq!(check_url("http://127.0.0.1:8080///").unwrap_or_default(), "http://127.0.0.1:8080");
        assert!(check_url("http://localhost.evil.example").is_err());
        assert!(check_url("http://localhostx:1").is_err());
        assert!(check_url("HTTP://127.0.0.1").is_err(), "no case-folding tricks");
        assert!(check_url("").is_err());
        assert!(check_url("http://evil.example/127.0.0.1").is_err());
        assert!(check_url("http://evil.example?h=localhost").is_err());
    }

    #[test]
    fn client_new_rejects_bad_urls() {
        let bad = Conn { api_url: "http://hub.example.com".into(), ingest_url: "http://127.0.0.1:1".into() };
        assert!(Client::new(&bad).is_err());
        let bad = Conn { api_url: "http://127.0.0.1:1".into(), ingest_url: "ftp://x".into() };
        assert!(Client::new(&bad).is_err());
    }

    fn client(url: &str) -> Client {
        Client::new(&Conn { api_url: url.into(), ingest_url: url.into() }).unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn api_json_maps_statuses_without_leaking_the_token() {
        let (url, _) = crate::test_http::serve(|r| match r.target.as_str() {
            "/ok" => (200, r#"{"a":1}"#.into()),
            "/unauth" => (401, "{}".into()),
            "/bad" => (400, r#"{"message":"range too large"}"#.into()),
            _ => (500, "not json".into()),
        });
        let c = client(&url);
        let t = Secret::for_test("cha_do_not_print");
        assert_eq!(c.api_json(&t, "/ok", &[]).ok(), Some(serde_json::json!({"a": 1})));
        let errs = ["/unauth", "/bad", "/boom"].map(|p| c.api_json(&t, p, &[]).err().map(|e| e.to_string()));
        let errs = errs.map(Option::unwrap_or_default);
        assert!(errs[0].contains("API token rejected"), "{}", errs[0]);
        assert_eq!(errs[1], "GET /bad failed with 400: range too large");
        assert_eq!(errs[2], "GET /boom failed with 500: ", "non-JSON bodies are not echoed");
        assert!(errs.iter().all(|e| !e.contains("do_not_print")));
    }

    #[test]
    fn api_get_sends_the_bearer_and_encodes_query_values() {
        let (url, log) = crate::test_http::serve(|_| (200, "{}".into()));
        let t = Secret::for_test("cha_q");
        let got = client(&url).api_get(&t, "/x", &[("service", "a b&c=d".into())]);
        assert_eq!(got.ok().map(|g| g.0), Some(200));
        let log = log.lock().map(|l| l.clone()).unwrap_or_default();
        assert_eq!(log[0].authorization.as_deref(), Some("Bearer cha_q"));
        assert!(log[0].target.starts_with("/x?service="), "{}", log[0].target);
        assert!(!log[0].target.contains("&c=d"), "query values are encoded: {}", log[0].target);
    }

    #[test]
    fn scope_requires_a_project_and_an_environment() {
        let (url, _) = crate::test_http::serve(|r| match r.authorization.as_deref() {
            Some("Bearer cha_none") => (200, r#"{"projects":[]}"#.into()),
            Some("Bearer cha_noenv") => (200, r#"{"projects":[{"id":"p","environments":[]}]}"#.into()),
            _ => (200, crate::test_http::session_json()),
        });
        let c = client(&url);
        let err = |t: &str| c.scope(&Secret::for_test(t)).err().map(|e| e.to_string()).unwrap_or_default();
        assert_eq!(err("cha_none"), "API token has no project access");
        assert_eq!(err("cha_noenv"), "API token has no environment access");
        let s = c.scope(&Secret::for_test("cha_ok")).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((s.project_id.as_str(), s.environment_id.as_str()), ("demo", "local-demo"));
    }

    #[test]
    fn unreachable_hosts_are_errors_not_statuses() {
        let c = client("http://127.0.0.1:1");
        assert!(c.status("http://127.0.0.1:1/health").is_err());
        assert!(c.post_events(&Secret::for_test("chi_x"), &serde_json::json!({})).is_err());
    }
}
