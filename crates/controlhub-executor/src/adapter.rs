//! Demo adapter: the allowlisted effects on the demo system.
//!
//! - Fault control goes to the demo services' loopback-only `/__demo/faults` endpoint.
//! - The database restart uses the Docker CLI with fixed arguments on one container, after
//!   checking its compose labels. No shell is involved and no argument comes from a plan
//!   except through the typed [`Action`] enum.

use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow, bail};
use clap::Args;
use controlhub_actions::{Action, DemoService};
use serde_json::{Value, json};
use tokio::process::Command;

const COMPOSE_PROJECT: &str = "controlhub";
const COMPOSE_SERVICE: &str = "demo-postgres";

#[derive(Debug, Clone, Args)]
pub struct DemoConfig {
    #[arg(long, env = "DEMO_GATEWAY_URL", default_value = "http://127.0.0.1:7001")]
    pub gateway_url: String,
    #[arg(long, env = "DEMO_ORDERS_URL", default_value = "http://127.0.0.1:7002")]
    pub orders_url: String,
    /// Container of the demo database; must carry the compose labels of `demo-postgres`.
    #[arg(long, env = "CONTROLHUB_DEMO_DB_CONTAINER", default_value = "controlhub-demo-postgres-1")]
    pub db_container: String,
    #[arg(long, env = "CONTROLHUB_DOCKER_BIN", default_value = "docker")]
    pub docker_bin: String,
}

#[derive(Debug, Clone)]
pub struct Demo {
    cfg: DemoConfig,
    agent: ureq::Agent,
}

fn loopback(url: &str) -> bool {
    ["http://127.0.0.1:", "http://localhost:", "http://[::1]:"].iter().any(|p| url.starts_with(p)) && !url.contains('@')
}

impl Demo {
    pub fn new(cfg: DemoConfig) -> anyhow::Result<Self> {
        for u in [&cfg.gateway_url, &cfg.orders_url] {
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
        Ok(Self { cfg, agent })
    }

    fn service_url(&self, s: DemoService) -> &str {
        match s {
            DemoService::Gateway => &self.cfg.gateway_url,
            DemoService::Orders => &self.cfg.orders_url,
        }
    }

    pub async fn run(&self, action: &Action) -> anyhow::Result<Value> {
        match *action {
            Action::DemoFaultsClear { service } => self.set_faults(service, 0, 0.0).await,
            Action::DemoFaultsInject { service, latency_ms, error_rate } => {
                self.set_faults(service, latency_ms, error_rate).await
            }
            Action::DemoRestartDatabase => self.restart_database().await,
        }
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
            db_container: "controlhub-demo-postgres-1".into(),
            docker_bin: "docker".into(),
        }
    }

    #[test]
    fn only_loopback_targets_and_plain_container_names() {
        assert!(Demo::new(cfg()).is_ok());
        assert!(Demo::new(DemoConfig { orders_url: "http://orders.internal:7002".into(), ..cfg() }).is_err());
        assert!(Demo::new(DemoConfig { orders_url: "http://127.0.0.1:1@evil".into(), ..cfg() }).is_err());
        assert!(Demo::new(DemoConfig { db_container: "x; rm -rf /".into(), ..cfg() }).is_err());
        assert!(Demo::new(DemoConfig { db_container: "--privileged".into(), ..cfg() }).is_err());
    }
}
