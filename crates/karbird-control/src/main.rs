//! Outbound-only Karbird Control loop for one Docker container.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, bail};
use clap::Parser;
use karbird_control::{Command, Docker, Journal, ResultReport, execute_once, valid_container_id};
use reqwest::Url;
use serde::{Deserialize, Serialize};

#[derive(Debug, Parser)]
struct Args {
    #[arg(long, env = "KARBIRD_CONTROL_ORIGIN")]
    origin: String,
    #[arg(long, env = "KARBIRD_CONTROL_CONTAINER_ID")]
    container_id: String,
    #[arg(long, env = "KARBIRD_CONTROL_STATE_DIR", default_value = "/var/lib/karbird-control")]
    state_dir: PathBuf,
    #[arg(long, env = "KARBIRD_CONTROL_ENROLLMENT_FILE")]
    enrollment_file: Option<PathBuf>,
    #[arg(long, env = "KARBIRD_CONTROL_DOCKER_SOCKET", default_value = "/var/run/docker.sock")]
    docker_socket: PathBuf,
}

#[derive(Debug, Deserialize)]
struct Enrollment {
    credential: String,
}

#[derive(Debug, Serialize)]
struct Report<'a> {
    container_id: &'a str,
    container_name: &'a str,
    state: &'a str,
}

fn control_origin(raw: &str) -> anyhow::Result<Url> {
    let url = Url::parse(raw).context("invalid Karbird origin")?;
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        bail!("Karbird origin must not include a path, query or fragment");
    }
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if url.scheme() != "https" && !(url.scheme() == "http" && local) {
        bail!("Karbird Control requires HTTPS except for loopback development");
    }
    Ok(url)
}

fn read_secret(path: &Path) -> anyhow::Result<String> {
    let text = fs::read_to_string(path).context("cannot read Control secret file")?;
    let secret = text.trim();
    if secret.len() < 30 || secret.len() > 100 {
        bail!("invalid Control secret file");
    }
    Ok(secret.to_owned())
}

fn save_secret(path: &Path, secret: &str) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new().create_new(true).write(true).mode(0o600).open(path)?;
        file.write_all(secret.as_bytes())?;
        file.sync_all()?;
    }
    #[cfg(not(unix))]
    fs::write(path, secret)?;
    Ok(())
}

async fn enroll(client: &reqwest::Client, origin: &Url, path: &Path) -> anyhow::Result<String> {
    let code = read_secret(path)?;
    let response = client
        .post(origin.join("api/v1/control/enroll")?)
        .json(&serde_json::json!({ "enrollment_code": code }))
        .send()
        .await?
        .error_for_status()?;
    Ok(response.json::<Enrollment>().await?.credential)
}

async fn report(
    client: &reqwest::Client,
    origin: &Url,
    credential: &str,
    docker: &Docker,
    id: &str,
) -> anyhow::Result<()> {
    let info = docker.inspect(id).await?;
    let state = if !info.state.running {
        "stopped"
    } else if info.state.health.as_ref().is_some_and(|h| h.status == "unhealthy") {
        "unhealthy"
    } else {
        "running"
    };
    client
        .post(origin.join("api/v1/control/report")?)
        .bearer_auth(credential)
        .json(&Report { container_id: &info.id, container_name: &info.name, state })
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn poll(client: &reqwest::Client, origin: &Url, credential: &str) -> anyhow::Result<Option<Command>> {
    let response =
        client.get(origin.join("api/v1/control/commands")?).bearer_auth(credential).send().await?.error_for_status()?;
    Ok(response.json().await?)
}

async fn complete(
    client: &reqwest::Client,
    origin: &Url,
    credential: &str,
    command: &Command,
    result: &ResultReport,
) -> anyhow::Result<()> {
    client
        .post(origin.join(&format!("api/v1/control/commands/{}/complete", command.id))?)
        .bearer_auth(credential)
        .json(result)
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    if !valid_container_id(&args.container_id) {
        bail!("container ID must be the full 64-character Docker ID");
    }
    let origin = control_origin(&args.origin)?;
    fs::create_dir_all(&args.state_dir)?;
    let credential_path = args.state_dir.join("credential");
    let client = reqwest::Client::builder().timeout(Duration::from_secs(30)).build()?;
    let credential = if credential_path.exists() {
        read_secret(&credential_path)?
    } else {
        let enrollment = args.enrollment_file.as_ref().context("enrollment file required on first start")?;
        let credential = enroll(&client, &origin, enrollment).await?;
        save_secret(&credential_path, &credential)?;
        credential
    };
    let docker = Docker::new(&args.docker_socket)?;
    let mut journal = Journal::open(args.state_dir.join("commands.json"))?;
    loop {
        if let Err(error) = report(&client, &origin, &credential, &docker, &args.container_id).await {
            eprintln!("Control status report failed: {error}");
        }
        match poll(&client, &origin, &credential).await {
            Ok(Some(command)) => match execute_once(&mut journal, &docker, &command, &args.container_id).await {
                Ok(result) => {
                    if let Err(error) = complete(&client, &origin, &credential, &command, &result).await {
                        eprintln!("Control result delivery failed: {error}");
                    }
                }
                Err(error) => eprintln!("Control command rejected: {error}"),
            },
            Ok(None) => {}
            Err(error) => eprintln!("Control poll failed: {error}"),
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::control_origin;

    #[test]
    fn remote_origins_require_https() {
        assert!(control_origin("https://karbird.example").is_ok());
        assert!(control_origin("http://127.0.0.1:8080").is_ok());
        assert!(control_origin("http://karbird.example").is_err());
        assert!(control_origin("https://karbird.example/some/path").is_err());
    }
}
