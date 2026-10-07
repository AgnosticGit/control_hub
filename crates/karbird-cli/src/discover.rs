//! Read-only project inspection. Looks only at the project root; `.env` files are listed by
//! name and never opened.

use std::path::Path;

use anyhow::{Context as _, bail};
use serde::Serialize;
use serde_json::Value;

pub const SDK_PACKAGE: &str = "@karbird/sdk-node";
pub const SDK_REGISTER: &str = "@karbird/sdk-node/register";
pub const SDK_PHP_PACKAGE: &str = "karbird/sdk-php";
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Ecosystem {
    Node,
    Php,
    Unsupported,
}

#[derive(Debug, Clone, Serialize)]
pub struct Discovery {
    pub ecosystem: Ecosystem,
    pub package_name: Option<String>,
    /// `module` or `commonjs`.
    pub module_type: Option<String>,
    pub package_manager: Option<String>,
    pub start_script: Option<String>,
    /// Entry file from the start script or `main`.
    pub entry: Option<String>,
    pub node_engines: Option<String>,
    pub frameworks: Vec<String>,
    /// Datastore clients. Only `pg` is instrumented by the SDK today.
    pub datastores: Vec<String>,
    /// Other telemetry agents that may conflict with the SDK.
    pub existing_telemetry: Vec<String>,
    pub karbird_sdk_installed: bool,
    pub karbird_register_in_start: bool,
    pub env_files: Vec<String>,
    pub env_ignored_by_git: Option<bool>,
    pub containers: Vec<String>,
    pub other_ecosystems: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn read_limited(path: &Path) -> anyhow::Result<String> {
    let meta = std::fs::symlink_metadata(path).with_context(|| format!("cannot read {}", path.display()))?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        bail!("{} is not a regular file", path.display());
    }
    if meta.len() > MAX_MANIFEST_BYTES {
        bail!("{} is larger than 1 MiB", path.display());
    }
    Ok(std::fs::read_to_string(path)?)
}

/// Entry file of a `node …` command, skipping flags and their values.
pub fn node_entry(script: &str) -> Option<String> {
    let mut words = script.split_whitespace();
    if words.next()? != "node" {
        return None;
    }
    let takes_value =
        ["--import", "-r", "--require", "--loader", "--env-file", "--experimental-loader", "--conditions", "-C"];
    let mut skip = false;
    for w in words {
        if skip {
            skip = false;
            continue;
        }
        if takes_value.contains(&w) {
            skip = true;
            continue;
        }
        if w.starts_with('-') {
            continue;
        }
        return Some(w.to_owned());
    }
    None
}

fn names(deps: &serde_json::Map<String, Value>, candidates: &[&str]) -> Vec<String> {
    candidates.iter().filter(|c| deps.contains_key(**c)).map(|c| (*c).to_owned()).collect()
}

pub fn discover(root: &Path) -> anyhow::Result<Discovery> {
    if !root.is_dir() {
        bail!("{} is not a directory", root.display());
    }
    let mut files = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            files.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    files.sort();
    let has = |n: &str| files.iter().any(|f| f == n);

    let env_files: Vec<String> =
        files.iter().filter(|f| *f == ".env" || f.starts_with(".env.") || f.ends_with(".env")).cloned().collect();
    let containers: Vec<String> = files
        .iter()
        .filter(|f| {
            *f == "Dockerfile"
                || f.starts_with("Dockerfile.")
                || matches!(f.as_str(), "docker-compose.yml" | "docker-compose.yaml" | "compose.yml" | "compose.yaml")
        })
        .cloned()
        .collect();
    let other_ecosystems: Vec<String> = [
        ("Cargo.toml", "rust"),
        ("go.mod", "go"),
        ("pyproject.toml", "python"),
        ("requirements.txt", "python"),
        ("pom.xml", "java"),
        ("build.gradle", "java"),
        ("build.gradle.kts", "java"),
    ]
    .iter()
    .filter(|(f, _)| has(f))
    .map(|(_, e)| (*e).to_owned())
    .collect::<std::collections::BTreeSet<_>>()
    .into_iter()
    .collect();
    let env_ignored_by_git = if has(".gitignore") {
        let gi = read_limited(&root.join(".gitignore"))?;
        Some(gi.lines().map(str::trim).any(|l| matches!(l, ".env" | "/.env" | ".env*" | ".env.*" | "*.env")))
    } else {
        None
    };
    let package_manager = [
        ("pnpm-lock.yaml", "pnpm"),
        ("yarn.lock", "yarn"),
        ("bun.lock", "bun"),
        ("bun.lockb", "bun"),
        ("package-lock.json", "npm"),
    ]
    .iter()
    .find(|(f, _)| has(f))
    .map(|(_, m)| (*m).to_owned());

    let mut d = Discovery {
        ecosystem: Ecosystem::Unsupported,
        package_name: None,
        module_type: None,
        package_manager,
        start_script: None,
        entry: None,
        node_engines: None,
        frameworks: vec![],
        datastores: vec![],
        existing_telemetry: vec![],
        karbird_sdk_installed: false,
        karbird_register_in_start: false,
        env_files,
        env_ignored_by_git,
        containers,
        other_ecosystems,
        warnings: vec![],
    };
    if d.env_ignored_by_git == Some(false) && d.env_files.iter().any(|f| f == ".env") {
        d.warnings.push(".env exists but is not ignored by .gitignore".into());
    }
    if has("composer.json") {
        let composer: Value = serde_json::from_str(&read_limited(&root.join("composer.json"))?)
            .context("composer.json is not valid JSON")?;
        d.ecosystem = Ecosystem::Php;
        d.package_name = composer.pointer("/name").and_then(Value::as_str).map(str::to_owned);
        let mut deps = serde_json::Map::new();
        for key in ["require", "require-dev"] {
            if let Some(obj) = composer.get(key).and_then(Value::as_object) {
                deps.extend(obj.clone());
            }
        }
        d.frameworks = names(&deps, &["laravel/framework", "symfony/framework-bundle", "slim/slim"]);
        d.datastores = names(
            &deps,
            &["doctrine/dbal", "ext-pgsql", "ext-mysqli", "mongodb/mongodb", "predis/predis", "ext-redis"],
        );
        d.existing_telemetry =
            names(&deps, &["open-telemetry/sdk", "ddtrace/dd-trace", "sentry/sentry", "newrelic/newrelic"]);
        d.karbird_sdk_installed = deps.contains_key(SDK_PHP_PACKAGE);
        if !deps.contains_key("ext-opentelemetry") {
            d.warnings.push("the PHP extension ext-opentelemetry is required for automatic instrumentation".into());
        }
        if !d.existing_telemetry.is_empty() && !d.karbird_sdk_installed {
            d.warnings.push(format!(
                "existing telemetry packages ({}): check for duplicate OpenTelemetry providers before enabling Karbird",
                d.existing_telemetry.join(", ")
            ));
        }
        return Ok(d);
    }
    if !has("package.json") {
        d.warnings.push(
            "no supported package manifest found (package.json or composer.json); configure OTLP manually".into(),
        );
        return Ok(d);
    }

    let pkg: Value =
        serde_json::from_str(&read_limited(&root.join("package.json"))?).context("package.json is not valid JSON")?;
    d.ecosystem = Ecosystem::Node;
    d.package_name = pkg.get("name").and_then(Value::as_str).map(str::to_owned);
    d.module_type = Some(pkg.get("type").and_then(Value::as_str).unwrap_or("commonjs").to_owned());
    d.node_engines = pkg.pointer("/engines/node").and_then(Value::as_str).map(str::to_owned);
    d.start_script = pkg.pointer("/scripts/start").and_then(Value::as_str).map(str::to_owned);
    d.entry = d
        .start_script
        .as_deref()
        .and_then(node_entry)
        .or_else(|| pkg.get("main").and_then(Value::as_str).map(str::to_owned));
    d.karbird_register_in_start = d.start_script.as_deref().is_some_and(|s| s.contains(SDK_REGISTER));

    let mut deps = serde_json::Map::new();
    for key in ["dependencies", "devDependencies"] {
        if let Some(obj) = pkg.get(key).and_then(Value::as_object) {
            deps.extend(obj.clone());
        }
    }
    d.frameworks = names(&deps, &["express", "fastify", "koa", "@hapi/hapi", "@nestjs/core", "next", "hono"]);
    d.datastores =
        names(&deps, &["pg", "mysql2", "mongodb", "mongoose", "ioredis", "redis", "@prisma/client", "kafkajs"]);
    d.existing_telemetry = names(
        &deps,
        &[
            "@opentelemetry/sdk-node",
            "@opentelemetry/sdk-trace-node",
            "dd-trace",
            "newrelic",
            "@sentry/node",
            "elastic-apm-node",
        ],
    );
    d.karbird_sdk_installed = deps.contains_key(SDK_PACKAGE);

    if !d.existing_telemetry.is_empty() {
        d.warnings.push(format!(
            "existing telemetry agents ({}): the SDK refuses to start next to another OpenTelemetry provider unless allowExistingOpenTelemetry is set",
            d.existing_telemetry.join(", ")
        ));
    }
    let unsupported: Vec<&String> = d.datastores.iter().filter(|s| *s != "pg").collect();
    if !unsupported.is_empty() {
        d.warnings.push(format!(
            "no automatic spans for {} yet; they appear only as inferred dependencies if at all",
            unsupported.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
        ));
    }
    if d.frameworks.iter().any(|f| f == "next" || f == "@nestjs/core") {
        d.warnings.push("framework CLIs (next/nest) need NODE_OPTIONS=--import @karbird/sdk-node/register".into());
    }
    Ok(d)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn entry_skips_flags() {
        assert_eq!(node_entry("node src/server.js"), Some("src/server.js".into()));
        assert_eq!(
            node_entry("node --import x/register --enable-source-maps dist/main.js --port 1"),
            Some("dist/main.js".into())
        );
        assert_eq!(node_entry("node -r dotenv/config app.js"), Some("app.js".into()));
        assert_eq!(node_entry("next start"), None);
    }

    #[test]
    fn discovers_node_project_without_reading_env_values() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"name":"@acme/shop","scripts":{"start":"node src/server.js"},"dependencies":{"express":"^5","pg":"^8","ioredis":"^5"}}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join(".env"), "SECRET=do-not-read").unwrap();
        std::fs::write(dir.path().join(".gitignore"), "node_modules\n").unwrap();
        std::fs::write(dir.path().join("package-lock.json"), "{}").unwrap();
        let d = discover(dir.path()).unwrap();
        assert_eq!(d.ecosystem, Ecosystem::Node);
        assert_eq!(d.entry.as_deref(), Some("src/server.js"));
        assert_eq!(d.package_manager.as_deref(), Some("npm"));
        assert_eq!(d.frameworks, vec!["express"]);
        assert_eq!(d.env_files, vec![".env"]);
        assert_eq!(d.env_ignored_by_git, Some(false));
        assert!(!serde_json::to_string(&d).unwrap().contains("do-not-read"));
        assert!(d.warnings.iter().any(|w| w.contains("ioredis")));
        assert!(d.warnings.iter().any(|w| w.contains("not ignored")));
    }

    #[test]
    fn non_node_project_is_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module x").unwrap();
        let d = discover(dir.path()).unwrap();
        assert_eq!(d.ecosystem, Ecosystem::Unsupported);
        assert_eq!(d.other_ecosystems, vec!["go"]);
    }

    #[test]
    fn discovers_php_project_without_reading_env_values() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("composer.json"),
            r#"{"name":"acme/shop","require":{"laravel/framework":"^11","ext-pdo":"*","karbird/sdk-php":"^0.1"}}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join(".env"), "KARBIRD_INGEST_TOKEN=do-not-read").unwrap();
        let d = discover(dir.path()).unwrap();
        assert_eq!(d.ecosystem, Ecosystem::Php);
        assert_eq!(d.package_name.as_deref(), Some("acme/shop"));
        assert_eq!(d.frameworks, vec!["laravel/framework"]);
        assert!(d.datastores.is_empty(), "generic PDO is not a specific database client");
        assert!(d.karbird_sdk_installed);
        assert!(!serde_json::to_string(&d).unwrap().contains("do-not-read"));
        assert!(d.warnings.iter().any(|w| w.contains("ext-opentelemetry")));
    }
}
