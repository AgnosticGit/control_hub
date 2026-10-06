//! Demo adapter: the allowlisted effects on the demo system.
//!
//! - Fault control and feature flags go to the demo services' loopback-only `/__demo/faults`
//!   and `/__demo/flags` endpoints.
//! - Container restarts, replica scaling and release rollbacks go to the loopback-only demo
//!   orchestrator (`scripts/demo-control.mjs`, ADR-0020).
//! - The database restart uses the Docker CLI with fixed arguments on one container, after
//!   checking its compose labels. No shell is involved and no argument comes from a plan
//!   except through the typed [`Action`] enum.
//!
//! Every step reads the resulting state back and fails if it differs from what was asked.

use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow, bail};
use clap::Args;
use karbird_actions::{Action, DemoContainer, DemoFlag, DemoReleaseService, DemoScalable, DemoService, demo_release};
use serde_json::{Value, json};
use tokio::process::Command;

const COMPOSE_PROJECT: &str = "karbird";
const COMPOSE_SERVICE: &str = "demo-postgres";

#[derive(Debug, Clone, Args)]
pub struct DemoConfig {
    #[arg(long, env = "DEMO_GATEWAY_URL", default_value = "http://127.0.0.1:7001")]
    pub gateway_url: String,
    #[arg(long, env = "DEMO_ORDERS_URL", default_value = "http://127.0.0.1:7002")]
    pub orders_url: String,
    /// Demo orchestrator (restarts, scaling, releases).
    #[arg(long, env = "DEMO_CONTROL_URL", default_value = "http://127.0.0.1:7090")]
    pub control_url: String,
    /// Container of the demo database; must carry the compose labels of `demo-postgres`.
    #[arg(long, env = "KARBIRD_DEMO_DB_CONTAINER", default_value = "karbird-demo-postgres-1")]
    pub db_container: String,
    #[arg(long, env = "KARBIRD_DOCKER_BIN", default_value = "docker")]
    pub docker_bin: String,
}

#[derive(Debug, Clone)]
pub struct Demo {
    cfg: DemoConfig,
    agent: ureq::Agent,
    /// Orchestrator calls wait for processes to start and report healthy.
    control_agent: ureq::Agent,
}

fn loopback(url: &str) -> bool {
    ["http://127.0.0.1:", "http://localhost:", "http://[::1]:"].iter().any(|p| url.starts_with(p)) && !url.contains('@')
}

/// GET (no body) or POST (JSON body) returning the JSON reply. A non-2xx answer is an error
/// carrying the reply's `error` text.
fn request(agent: &ureq::Agent, url: &str, body: Option<&Value>) -> anyhow::Result<Value> {
    let mut res = match body {
        Some(b) => agent.post(url).send_json(b),
        None => agent.get(url).call(),
    }
    .with_context(|| format!("{url} unreachable"))?;
    let status = res.status();
    let text = res.body_mut().with_config().limit(64 * 1024).read_to_string()?;
    let value: Value = serde_json::from_str(&text).with_context(|| format!("{url} answered {status} without JSON"))?;
    if !status.is_success() {
        let detail = value.get("error").and_then(Value::as_str).unwrap_or("no detail");
        bail!("{url} answered {status}: {detail}");
    }
    Ok(value)
}

fn count(v: &Value, key: &str) -> Option<u64> {
    v.get(key).and_then(Value::as_u64)
}

fn len(v: &Value, key: &str) -> Option<usize> {
    v.get(key).and_then(Value::as_array).map(Vec::len)
}

impl Demo {
    pub fn new(cfg: DemoConfig) -> anyhow::Result<Self> {
        for u in [&cfg.gateway_url, &cfg.orders_url, &cfg.control_url] {
            if !loopback(u) {
                bail!("demo service URL must be a loopback http URL: {u}");
            }
        }
        // Must start alphanumeric so it can never be parsed as a docker flag.
        let valid_name = cfg.db_container.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
            && cfg.db_container.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
        if !valid_name {
            bail!("invalid container name");
        }
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(10)))
            .http_status_as_error(false)
            .build()
            .into();
        let control_agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(150)))
            .http_status_as_error(false)
            .build()
            .into();
        Ok(Self { cfg, agent, control_agent })
    }

    fn service_url(&self, s: DemoService) -> &str {
        match s {
            DemoService::Gateway => &self.cfg.gateway_url,
            DemoService::Orders => &self.cfg.orders_url,
        }
    }

    async fn call(&self, agent: &ureq::Agent, url: String, body: Option<Value>) -> anyhow::Result<Value> {
        let agent = agent.clone();
        tokio::task::spawn_blocking(move || request(&agent, &url, body.as_ref())).await?
    }

    async fn control(&self, path: &str, body: Option<Value>) -> anyhow::Result<Value> {
        let url = format!("{}{path}", self.cfg.control_url);
        self.call(&self.control_agent, url, body).await.context("demo orchestrator (is `just demo` running?)")
    }

    pub async fn run(&self, action: &Action) -> anyhow::Result<Value> {
        match *action {
            Action::DemoFaultsClear { service } => self.set_faults(service, 0, 0.0).await,
            Action::DemoFaultsInject { service, latency_ms, error_rate } => {
                self.set_faults(service, latency_ms, error_rate).await
            }
            Action::DemoRestartDatabase => self.restart_database().await,
            Action::DemoContainerRestart { container } => self.restart_container(container).await,
            Action::DemoReplicasScale { service, replicas } => self.scale(service, replicas).await,
            Action::DemoFeatureFlagSet { flag, enabled } => self.set_flag(flag, enabled).await,
            Action::DemoReleaseRollback { service } => self.rollback(service).await,
        }
    }

    async fn restart_container(&self, container: DemoContainer) -> anyhow::Result<Value> {
        let name = container.as_str();
        let out = self.control(&format!("/instances/{name}/restart"), Some(json!({}))).await?;
        let (previous, pid) = (count(&out, "previous_pid"), count(&out, "pid"));
        if out.get("instance").and_then(Value::as_str) != Some(name) || pid.is_none() || pid == previous {
            bail!("restart of {name} did not report a new process: {out}");
        }
        let listed = self.control("/instances", None).await?;
        let now = listed
            .get("instances")
            .and_then(Value::as_array)
            .and_then(|all| all.iter().find(|i| i.get("name").and_then(Value::as_str) == Some(name)));
        match now {
            Some(i) if i.get("running") == Some(&Value::Bool(true)) && count(i, "pid") == pid => {}
            _ => bail!("{name} is not running as the restarted process: {}", now.unwrap_or(&Value::Null)),
        }
        Ok(json!({
            "container": name,
            "service": container.service(),
            "previous_pid": previous,
            "pid": pid,
            "healthy_after_ms": count(&out, "healthy_after_ms"),
        }))
    }

    async fn scale(&self, service: DemoScalable, replicas: u8) -> anyhow::Result<Value> {
        let name = service.as_str();
        let want = usize::from(replicas);
        self.control(&format!("/replicas/{name}"), Some(json!({ "replicas": replicas }))).await?;
        let state = self.control(&format!("/replicas/{name}"), None).await?;
        if count(&state, "replicas") != Some(u64::from(replicas))
            || len(&state, "running") != Some(want)
            || len(&state, "upstreams") != Some(want)
        {
            bail!("{name} did not settle at {replicas} routed replicas: {state}");
        }
        Ok(json!({ "service": name, "requested": replicas, "state": state }))
    }

    async fn set_flag(&self, flag: DemoFlag, enabled: bool) -> anyhow::Result<Value> {
        let url = format!("{}/__demo/flags", self.service_url(flag.service()));
        let name = flag.as_str();
        self.call(&self.agent, url.clone(), Some(json!({ "flag": name, "enabled": enabled })))
            .await
            .context("flag control (is the demo running with fault injection?)")?;
        let readback = self.call(&self.agent, url, None).await?;
        if readback.get("flags").and_then(|f| f.get(name)).and_then(Value::as_bool) != Some(enabled) {
            bail!("flag {name} did not read back as {enabled}: {readback}");
        }
        Ok(json!({ "flag": name, "service": flag.service().as_str(), "enabled": enabled, "flags": readback["flags"] }))
    }

    async fn rollback(&self, service: DemoReleaseService) -> anyhow::Result<Value> {
        let name = service.as_str();
        let out = self.control(&format!("/releases/{name}/rollback"), Some(json!({}))).await?;
        let version = |k: &str| out.get(k).and_then(Value::as_str).and_then(demo_release);
        let (Some(from), Some(to)) = (version("from_version"), version("to_version")) else {
            bail!("rollback of {name} did not report allowlisted releases: {out}");
        };
        if from == to {
            bail!("rollback of {name} stayed on {to}");
        }
        let now = self.control(&format!("/releases/{name}"), None).await?;
        if now.get("version").and_then(Value::as_str) != Some(to) {
            bail!("{name} does not report release {to} after the rollback: {now}");
        }
        Ok(json!({
            "service": name,
            "from_version": from,
            "to_version": to,
            "previous_pid": count(&out, "previous_pid"),
            "pid": count(&out, "pid"),
            "healthy_after_ms": count(&out, "healthy_after_ms"),
        }))
    }

    async fn set_faults(&self, service: DemoService, latency_ms: u32, error_rate: f64) -> anyhow::Result<Value> {
        let url = format!("{}/__demo/faults", self.service_url(service));
        let agent = self.agent.clone();
        let body = json!({ "latencyMs": latency_ms, "errorRate": error_rate });
        let readback = tokio::task::spawn_blocking(move || -> anyhow::Result<Value> {
            let mut res = agent.post(&url).send_json(&body).with_context(|| format!("{url} unreachable"))?;
            if !res.status().is_success() {
                bail!("fault control returned {} (is the demo running with fault injection?)", res.status());
            }
            let _ = res.body_mut().read_to_string();
            let mut res = agent.get(&url).call().with_context(|| format!("{url} unreachable"))?;
            let text = res.body_mut().with_config().limit(64 * 1024).read_to_string()?;
            Ok(serde_json::from_str(&text)?)
        })
        .await??;
        let got_latency = readback.get("latencyMs").and_then(Value::as_f64);
        let got_rate = readback.get("errorRate").and_then(Value::as_f64);
        if got_latency != Some(f64::from(latency_ms)) || got_rate.is_none_or(|r| (r - error_rate).abs() > 1e-9) {
            bail!("fault state did not read back as requested: {readback}");
        }
        Ok(json!({ "service": service.as_str(), "fault_state": readback }))
    }

    async fn docker(&self, args: &[&str]) -> anyhow::Result<String> {
        let out = Command::new(&self.cfg.docker_bin)
            .args(args)
            .kill_on_drop(true)
            .output()
            .await
            .with_context(|| format!("cannot run {}", self.cfg.docker_bin))?;
        if !out.status.success() {
            bail!("docker {} failed: {}", args.first().unwrap_or(&""), String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    }

    async fn restart_database(&self) -> anyhow::Result<Value> {
        let name = self.cfg.db_container.as_str();
        let labels = self
            .docker(&[
                "inspect",
                "--format",
                "{{index .Config.Labels \"com.docker.compose.project\"}}/{{index .Config.Labels \"com.docker.compose.service\"}}",
                name,
            ])
            .await?;
        if labels != format!("{COMPOSE_PROJECT}/{COMPOSE_SERVICE}") {
            bail!("container {name} is not {COMPOSE_PROJECT}/{COMPOSE_SERVICE} (labels: {labels})");
        }
        let started = Instant::now();
        self.docker(&["restart", "--time", "10", name]).await?;
        let deadline = started + Duration::from_secs(90);
        loop {
            let health = self.docker(&["inspect", "--format", "{{.State.Health.Status}}", name]).await?;
            if health == "healthy" {
                break;
            }
            if Instant::now() >= deadline {
                return Err(anyhow!("container did not become healthy within 90 s (status: {health})"));
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        Ok(json!({ "container": name, "healthy_after_ms": started.elapsed().as_millis() as u64 }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> DemoConfig {
        DemoConfig {
            gateway_url: "http://127.0.0.1:7001".into(),
            orders_url: "http://127.0.0.1:7002".into(),
            control_url: "http://127.0.0.1:7090".into(),
            db_container: "karbird-demo-postgres-1".into(),
            docker_bin: "docker".into(),
        }
    }

    #[test]
    fn only_loopback_targets_and_plain_container_names() {
        assert!(Demo::new(cfg()).is_ok());
        assert!(Demo::new(DemoConfig { orders_url: "http://orders.internal:7002".into(), ..cfg() }).is_err());
        assert!(Demo::new(DemoConfig { control_url: "http://10.0.0.5:7090".into(), ..cfg() }).is_err());
        assert!(Demo::new(DemoConfig { control_url: "https://127.0.0.1:7090".into(), ..cfg() }).is_err());
        assert!(Demo::new(DemoConfig { orders_url: "http://127.0.0.1:1@evil".into(), ..cfg() }).is_err());
        assert!(Demo::new(DemoConfig { db_container: "x; rm -rf /".into(), ..cfg() }).is_err());
        assert!(Demo::new(DemoConfig { db_container: "--privileged".into(), ..cfg() }).is_err());
    }

    #[test]
    fn loopback_urls() {
        for ok in ["http://127.0.0.1:7001", "http://localhost:7002", "http://[::1]:7001"] {
            assert!(loopback(ok), "{ok}");
        }
        for bad in [
            "https://127.0.0.1:7001",
            "http://127.0.0.1",
            "http://127.0.0.2:7001",
            "http://localhost.evil.example:80",
            "http://127.0.0.1:7001@evil.example",
            "http://user@127.0.0.1:7001",
            "HTTP://127.0.0.1:7001",
            "file:///etc/passwd",
            "",
        ] {
            assert!(!loopback(bad), "{bad}");
        }
    }

    #[test]
    fn container_names_can_never_become_flags() {
        for ok in ["karbird-demo-postgres-1", "a", "db_1.x", "0db"] {
            assert!(Demo::new(DemoConfig { db_container: ok.into(), ..cfg() }).is_ok(), "{ok}");
        }
        for bad in ["", "-x", ".hidden", "_x", "a b", "a/b", "a\nb", "a=b", "ä"] {
            assert!(Demo::new(DemoConfig { db_container: bad.into(), ..cfg() }).is_err(), "{bad:?}");
        }
    }

    /// Minimal HTTP/1.1 server on a random loopback port: answers each connection with the next
    /// canned response and records `METHOD PATH` plus the request body.
    fn fake_server(responses: Vec<(u16, &'static str)>) -> (String, std::thread::JoinHandle<Vec<(String, String)>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                let head_end = loop {
                    let n = stream.read(&mut chunk).unwrap();
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break i + 4;
                    }
                    assert!(n > 0, "client closed before sending headers");
                };
                let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                let len = head
                    .lines()
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().to_owned()))
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(0);
                while buf.len() < head_end + len {
                    let n = stream.read(&mut chunk).unwrap();
                    assert!(n > 0);
                    buf.extend_from_slice(&chunk[..n]);
                }
                let line = head.lines().next().unwrap_or_default().rsplit_once(' ').map(|(l, _)| l.to_owned());
                seen.push((
                    line.unwrap_or_default(),
                    String::from_utf8_lossy(&buf[head_end..head_end + len]).to_string(),
                ));
                let res = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(res.as_bytes()).unwrap();
            }
            seen
        });
        (url, handle)
    }

    fn demo_at(url: &str) -> Demo {
        Demo::new(DemoConfig { gateway_url: url.into(), orders_url: url.into(), control_url: url.into(), ..cfg() })
            .unwrap()
    }

    async fn run_against(
        responses: Vec<(u16, &'static str)>,
        action: Action,
    ) -> (anyhow::Result<Value>, Vec<(String, String)>) {
        let (url, server) = fake_server(responses);
        let out = demo_at(&url).run(&action).await;
        (out, server.join().unwrap())
    }

    fn body(seen: &(String, String)) -> Value {
        serde_json::from_str(&seen.1).unwrap()
    }

    #[tokio::test]
    async fn restart_asks_the_orchestrator_and_reads_back_the_new_process() {
        let (out, seen) = run_against(
            vec![
                (200, r#"{"instance":"catalog-2","service":"catalog","previous_pid":10,"pid":11,"healthy_after_ms":900}"#),
                (200, r#"{"instances":[{"name":"gateway","running":true,"pid":3},{"name":"catalog-2","running":true,"pid":11}]}"#),
            ],
            Action::DemoContainerRestart { container: DemoContainer::Catalog2 },
        )
        .await;
        let out = out.unwrap();
        assert_eq!(
            out,
            json!({ "container": "catalog-2", "service": "catalog", "previous_pid": 10, "pid": 11, "healthy_after_ms": 900 })
        );
        assert_eq!(seen[0].0, "POST /instances/catalog-2/restart");
        assert_eq!(seen[1].0, "GET /instances");
    }

    #[tokio::test]
    async fn a_restart_without_a_new_running_process_is_a_failure() {
        let restart = Action::DemoContainerRestart { container: DemoContainer::Orders };
        for reply in [
            r#"{"instance":"orders","previous_pid":10,"pid":10}"#,
            r#"{"instance":"orders","previous_pid":10}"#,
            r#"{"instance":"gateway","previous_pid":10,"pid":11}"#,
        ] {
            let (out, _) = run_against(vec![(200, reply)], restart.clone()).await;
            assert!(format!("{:#}", out.unwrap_err()).contains("did not report a new process"), "{reply}");
        }
        for listed in [
            r#"{"instances":[{"name":"orders","running":true,"pid":12}]}"#,
            r#"{"instances":[{"name":"orders","running":false,"pid":11}]}"#,
            r#"{"instances":[]}"#,
        ] {
            let (out, _) = run_against(
                vec![(200, r#"{"instance":"orders","previous_pid":10,"pid":11}"#), (200, listed)],
                restart.clone(),
            )
            .await;
            assert!(format!("{:#}", out.unwrap_err()).contains("not running as the restarted process"), "{listed}");
        }
    }

    #[tokio::test]
    async fn orchestrator_errors_are_failures_with_their_reason() {
        let (out, _) = run_against(
            vec![(409, r#"{"error":"catalog-3 is not running"}"#)],
            Action::DemoContainerRestart { container: DemoContainer::Catalog3 },
        )
        .await;
        let e = format!("{:#}", out.unwrap_err());
        assert!(e.contains("409") && e.contains("catalog-3 is not running") && e.contains("demo orchestrator"), "{e}");

        let (out, _) = run_against(
            vec![(502, "<html>bad gateway</html>")],
            Action::DemoReleaseRollback { service: DemoReleaseService::Orders },
        )
        .await;
        assert!(format!("{:#}", out.unwrap_err()).contains("without JSON"));

        let e = demo_at("http://127.0.0.1:1")
            .run(&Action::DemoReplicasScale { service: DemoScalable::Catalog, replicas: 1 })
            .await;
        assert!(format!("{:#}", e.unwrap_err()).contains("unreachable"));
    }

    #[tokio::test]
    async fn scaling_reads_back_running_and_routed_replicas() {
        let action = Action::DemoReplicasScale { service: DemoScalable::Catalog, replicas: 2 };
        let state = r#"{"service":"catalog","replicas":2,"max":3,"running":["catalog-1","catalog-2"],"upstreams":["http://127.0.0.1:7006","http://127.0.0.1:7016"]}"#;
        let (out, seen) = run_against(vec![(200, state), (200, state)], action.clone()).await;
        let out = out.unwrap();
        assert_eq!((out["requested"].as_u64(), out["state"]["replicas"].as_u64()), (Some(2), Some(2)));
        assert_eq!(seen[0].0, "POST /replicas/catalog");
        assert_eq!(body(&seen[0]), json!({ "replicas": 2 }));
        assert_eq!(seen[1].0, "GET /replicas/catalog");

        for wrong in [
            r#"{"replicas":3,"running":["a","b","c"],"upstreams":["a","b","c"]}"#,
            r#"{"replicas":2,"running":["a","b"],"upstreams":["a","b","c"]}"#,
            r#"{"replicas":2,"running":["a","b"],"upstreams":null}"#,
        ] {
            let (out, _) = run_against(vec![(200, "{}"), (200, wrong)], action.clone()).await;
            assert!(format!("{:#}", out.unwrap_err()).contains("did not settle at 2"), "{wrong}");
        }
    }

    #[tokio::test]
    async fn flags_are_set_on_the_owning_service_and_read_back() {
        let (out, seen) = run_against(
            vec![(200, r#"{"flags":{"checkout-v2":true}}"#), (200, r#"{"flags":{"checkout-v2":true}}"#)],
            Action::DemoFeatureFlagSet { flag: DemoFlag::CheckoutV2, enabled: true },
        )
        .await;
        let out = out.unwrap();
        assert_eq!((out["service"].as_str(), out["enabled"].as_bool()), (Some("orders"), Some(true)));
        assert_eq!(seen[0].0, "POST /__demo/flags");
        assert_eq!(body(&seen[0]), json!({ "flag": "checkout-v2", "enabled": true }));
        assert_eq!(seen[1].0, "GET /__demo/flags");

        for readback in [r#"{"flags":{"checkout-v2":false}}"#, r#"{"flags":{}}"#, r#"{"flags":{"checkout-v2":"true"}}"#]
        {
            let (out, _) = run_against(
                vec![(200, "{}"), (200, readback)],
                Action::DemoFeatureFlagSet { flag: DemoFlag::CheckoutV2, enabled: true },
            )
            .await;
            assert!(format!("{:#}", out.unwrap_err()).contains("did not read back"), "{readback}");
        }
        let (out, _) = run_against(
            vec![(404, r#"{"error":"not found"}"#)],
            Action::DemoFeatureFlagSet { flag: DemoFlag::CatalogCache, enabled: false },
        )
        .await;
        assert!(format!("{:#}", out.unwrap_err()).contains("fault injection"));
    }

    #[tokio::test]
    async fn rollback_reports_allowlisted_releases_and_reads_back_the_version() {
        let rollback = Action::DemoReleaseRollback { service: DemoReleaseService::Orders };
        let (out, seen) = run_against(
            vec![
                (200, r#"{"service":"orders","from_version":"0.2.0","to_version":"0.1.0","previous_pid":4,"pid":5,"healthy_after_ms":700}"#),
                (200, r#"{"service":"orders","version":"0.1.0","previous":null}"#),
            ],
            rollback.clone(),
        )
        .await;
        let out = out.unwrap();
        assert_eq!((out["from_version"].as_str(), out["to_version"].as_str()), (Some("0.2.0"), Some("0.1.0")));
        assert_eq!(seen[0].0, "POST /releases/orders/rollback");
        assert_eq!(seen[1].0, "GET /releases/orders");

        for reply in [
            r#"{"from_version":"0.2.0","to_version":"9.9.9"}"#,
            r#"{"from_version":"0.2.0","to_version":"0.1.0; rm -rf /"}"#,
            r#"{"to_version":"0.1.0"}"#,
        ] {
            let (out, _) = run_against(vec![(200, reply)], rollback.clone()).await;
            assert!(format!("{:#}", out.unwrap_err()).contains("allowlisted releases"), "{reply}");
        }
        let (out, _) =
            run_against(vec![(200, r#"{"from_version":"0.1.0","to_version":"0.1.0"}"#)], rollback.clone()).await;
        assert!(format!("{:#}", out.unwrap_err()).contains("stayed on 0.1.0"));
        let (out, _) = run_against(
            vec![(200, r#"{"from_version":"0.2.0","to_version":"0.1.0"}"#), (200, r#"{"version":"0.2.0"}"#)],
            rollback,
        )
        .await;
        assert!(format!("{:#}", out.unwrap_err()).contains("does not report release 0.1.0"));
    }

    #[tokio::test]
    async fn inject_posts_typed_values_and_verifies_the_readback() {
        let (url, server) = fake_server(vec![(200, "{}"), (200, r#"{"latencyMs":250,"errorRate":0.25}"#)]);
        let action = Action::DemoFaultsInject { service: DemoService::Orders, latency_ms: 250, error_rate: 0.25 };
        let out = demo_at(&url).run(&action).await.unwrap();
        assert_eq!(out["service"], "orders");
        assert_eq!(out["fault_state"]["latencyMs"], 250);
        let seen = server.join().unwrap();
        assert_eq!(seen[0].0, "POST /__demo/faults");
        assert_eq!(serde_json::from_str::<Value>(&seen[0].1).unwrap(), json!({ "latencyMs": 250, "errorRate": 0.25 }));
        assert_eq!(seen[1].0, "GET /__demo/faults");
    }

    #[tokio::test]
    async fn clear_posts_zeroes() {
        let (url, server) = fake_server(vec![(200, "{}"), (200, r#"{"latencyMs":0,"errorRate":0}"#)]);
        demo_at(&url).run(&Action::DemoFaultsClear { service: DemoService::Gateway }).await.unwrap();
        let seen = server.join().unwrap();
        assert_eq!(serde_json::from_str::<Value>(&seen[0].1).unwrap(), json!({ "latencyMs": 0, "errorRate": 0.0 }));
    }

    #[tokio::test]
    async fn a_readback_that_differs_is_a_failure() {
        for readback in [
            r#"{"latencyMs":0,"errorRate":0}"#,
            r#"{"latencyMs":250}"#,
            r#"{"latencyMs":250,"errorRate":0.2500001}"#,
            r#"{"latencyMs":"250","errorRate":0.25}"#,
        ] {
            let (url, server) = fake_server(vec![(200, "{}"), (200, readback)]);
            let action = Action::DemoFaultsInject { service: DemoService::Orders, latency_ms: 250, error_rate: 0.25 };
            let e = demo_at(&url).run(&action).await.unwrap_err();
            assert!(format!("{e:#}").contains("did not read back"), "{readback}: {e:#}");
            server.join().unwrap();
        }
    }

    #[tokio::test]
    async fn fault_endpoint_errors_are_failures() {
        let (url, server) = fake_server(vec![(500, "{}")]);
        let e = demo_at(&url).run(&Action::DemoFaultsClear { service: DemoService::Orders }).await.unwrap_err();
        assert!(format!("{e:#}").contains("fault control returned 500"), "{e:#}");
        server.join().unwrap();

        let (url, server) = fake_server(vec![(200, "{}"), (200, "<html>not json</html>")]);
        assert!(demo_at(&url).run(&Action::DemoFaultsClear { service: DemoService::Orders }).await.is_err());
        server.join().unwrap();

        // Nothing listens on port 1.
        let e = demo_at("http://127.0.0.1:1").run(&Action::DemoFaultsClear { service: DemoService::Orders }).await;
        assert!(format!("{:#}", e.unwrap_err()).contains("unreachable"));
    }

    #[tokio::test]
    async fn restart_fails_cleanly_without_docker() {
        let demo = Demo::new(DemoConfig { docker_bin: "karbird-test-no-such-docker".into(), ..cfg() }).unwrap();
        let e = demo.run(&Action::DemoRestartDatabase).await.unwrap_err();
        assert!(format!("{e:#}").contains("cannot run karbird-test-no-such-docker"), "{e:#}");
    }

    #[tokio::test]
    async fn restart_refuses_a_container_with_foreign_compose_labels() {
        let dir = std::env::temp_dir().join(format!("karbird-fake-docker-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        #[cfg(windows)]
        let bin = {
            let p = dir.join("docker.cmd");
            std::fs::write(&p, "@echo otherproject/postgres\r\n").unwrap();
            p
        };
        #[cfg(not(windows))]
        let bin = {
            use std::os::unix::fs::PermissionsExt;
            let p = dir.join("docker");
            std::fs::write(&p, "#!/bin/sh\necho otherproject/postgres\n").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        };
        let demo = Demo::new(DemoConfig { docker_bin: bin.to_string_lossy().into_owned(), ..cfg() }).unwrap();
        let e = demo.run(&Action::DemoRestartDatabase).await.unwrap_err();
        let msg = format!("{e:#}");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(msg.contains("is not karbird/demo-postgres") && msg.contains("otherproject/postgres"), "{msg}");
    }
}
