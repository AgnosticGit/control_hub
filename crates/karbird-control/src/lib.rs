//! One-container Docker executor. The local journal refuses to repeat delivered commands.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Command {
    pub id: String,
    pub verb: String,
    pub container_id: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultReport {
    pub succeeded: bool,
    pub observed_state: String,
    pub previous_started_at: Option<String>,
    pub started_at: Option<String>,
    pub detail: Option<String>,
}

impl ResultReport {
    fn unknown() -> Self {
        Self {
            succeeded: false,
            observed_state: "unknown".into(),
            previous_started_at: None,
            started_at: None,
            detail: Some("operation outcome unknown after local interruption; inspect Docker before retrying".into()),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct DockerInspect {
    #[serde(rename = "Id")]
    pub id: String,
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "State")]
    pub state: DockerState,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DockerState {
    #[serde(rename = "Running")]
    pub running: bool,
    #[serde(rename = "Status")]
    pub status: String,
    #[serde(rename = "StartedAt")]
    pub started_at: String,
    #[serde(rename = "Health")]
    pub health: Option<DockerHealth>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DockerHealth {
    #[serde(rename = "Status")]
    pub status: String,
}

#[derive(Debug)]
pub struct Journal {
    path: PathBuf,
    entries: BTreeMap<String, ResultReport>,
}

impl Journal {
    pub fn open(path: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let path = path.into();
        let entries = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("invalid Control journal")?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => return Err(error.into()),
        };
        Ok(Self { path, entries })
    }

    pub fn get(&self, id: &str) -> Option<&ResultReport> {
        self.entries.get(id)
    }

    pub fn record(&mut self, id: &str, result: ResultReport) -> anyhow::Result<()> {
        self.entries.insert(id.into(), result);
        let parent = self.path.parent().unwrap_or(Path::new("."));
        fs::create_dir_all(parent)?;
        let pending = self.path.with_extension("tmp");
        fs::write(&pending, serde_json::to_vec(&self.entries)?)?;
        fs::rename(&pending, &self.path)?;
        Ok(())
    }
}

pub fn valid_container_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn check_command(command: &Command, expected_container_id: &str) -> anyhow::Result<()> {
    if !valid_container_id(expected_container_id) || command.container_id != expected_container_id {
        bail!("command target does not match configured container ID");
    }
    if !matches!(command.verb.as_str(), "start" | "stop" | "restart") {
        bail!("unsupported Control verb");
    }
    uuid::Uuid::parse_str(&command.id).context("invalid command ID")?;
    let expires = time::OffsetDateTime::parse(&command.expires_at, &time::format_description::well_known::Rfc3339)
        .context("invalid command expiry")?;
    if expires <= time::OffsetDateTime::now_utc() {
        bail!("command expired");
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct Docker {
    #[cfg(unix)]
    client: reqwest::Client,
}

impl Docker {
    pub fn new(socket: &Path) -> anyhow::Result<Self> {
        #[cfg(unix)]
        {
            let client = reqwest::Client::builder().unix_socket(socket).timeout(Duration::from_secs(20)).build()?;
            Ok(Self { client })
        }
        #[cfg(not(unix))]
        {
            let _ = socket;
            bail!("Karbird Control requires a Linux Docker Unix socket")
        }
    }

    pub async fn inspect(&self, id: &str) -> anyhow::Result<DockerInspect> {
        if !valid_container_id(id) {
            bail!("invalid Docker container ID");
        }
        #[cfg(unix)]
        {
            let response = self
                .client
                .get(format!("http://docker/v1.45/containers/{id}/json"))
                .send()
                .await?
                .error_for_status()?;
            let info: DockerInspect = response.json().await?;
            if info.id != id {
                bail!("Docker returned a different container ID");
            }
            Ok(info)
        }
        #[cfg(not(unix))]
        bail!("Docker Unix socket unavailable on this platform")
    }

    async fn operation(&self, id: &str, verb: &str) -> anyhow::Result<()> {
        if !valid_container_id(id) || !matches!(verb, "start" | "stop" | "restart") {
            bail!("invalid Docker operation");
        }
        #[cfg(unix)]
        {
            let response = self.client.post(format!("http://docker/v1.45/containers/{id}/{verb}")).send().await?;
            if !response.status().is_success() && response.status() != reqwest::StatusCode::NOT_MODIFIED {
                bail!("Docker returned {}", response.status());
            }
            Ok(())
        }
        #[cfg(not(unix))]
        bail!("Docker Unix socket unavailable on this platform")
    }

    pub async fn execute(&self, command: &Command, expected_id: &str) -> anyhow::Result<ResultReport> {
        check_command(command, expected_id)?;
        let before = self.inspect(expected_id).await?;
        self.operation(expected_id, &command.verb).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
        loop {
            let after = self.inspect(expected_id).await?;
            let healthy = after.state.health.as_ref().is_none_or(|health| health.status == "healthy");
            let succeeded = match command.verb.as_str() {
                "stop" => !after.state.running,
                "start" => after.state.running && healthy,
                "restart" => after.state.running && healthy && before.state.started_at != after.state.started_at,
                _ => false,
            };
            if succeeded {
                return Ok(ResultReport {
                    succeeded: true,
                    observed_state: if after.state.running { "running" } else { "stopped" }.into(),
                    previous_started_at: Some(before.state.started_at),
                    started_at: Some(after.state.started_at),
                    detail: None,
                });
            }
            if tokio::time::Instant::now() >= deadline {
                bail!("Docker did not reach the expected state within 90 seconds");
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }
}

pub async fn execute_once(
    journal: &mut Journal,
    docker: &Docker,
    command: &Command,
    expected_id: &str,
) -> anyhow::Result<ResultReport> {
    if let Some(saved) = journal.get(&command.id) {
        return Ok(saved.clone());
    }
    check_command(command, expected_id)?;
    journal.record(&command.id, ResultReport::unknown())?;
    let result = match docker.execute(command, expected_id).await {
        Ok(result) => result,
        Err(error) => {
            ResultReport { detail: Some(format!("{error:#}").chars().take(500).collect()), ..ResultReport::unknown() }
        }
    };
    journal.record(&command.id, result.clone())?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_mismatched_and_expired_commands() {
        let id = "a".repeat(64);
        let command = Command {
            id: uuid::Uuid::new_v4().to_string(),
            verb: "restart".into(),
            container_id: id.clone(),
            expires_at: "2000-01-01T00:00:00Z".into(),
        };
        assert!(check_command(&command, &id).is_err());
        assert!(
            check_command(&Command { expires_at: "2999-01-01T00:00:00Z".into(), ..command }, &"b".repeat(64)).is_err()
        );
    }

    #[test]
    fn journal_preserves_unknown_before_a_command_can_be_retried() {
        let path = std::env::temp_dir().join(format!("karbird-control-{}.json", uuid::Uuid::new_v4()));
        let mut journal = Journal::open(&path).unwrap();
        journal.record("cmd", ResultReport::unknown()).unwrap();
        assert_eq!(Journal::open(&path).unwrap().get("cmd").unwrap().observed_state, "unknown");
        fs::remove_file(path).unwrap();
    }
}
