//! Integration plan: a list of whole-file edits with the SHA-256 of the file they were
//! computed from. `apply` refuses to touch a file that changed since the plan, only writes
//! allowlisted files in the project root, and is idempotent.

use std::path::{Component, Path, PathBuf};

use anyhow::{Context as _, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::discover::{self, Discovery, Ecosystem, SDK_PACKAGE, SDK_REGISTER, read_limited};
use crate::{Outcome, PlanArgs};

pub const PLAN_FORMAT: u32 = 1;
/// The only files `apply` may create or modify.
const ALLOWED_FILES: &[&str] = &["package.json", ".env.example", ".gitignore"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Step {
    pub id: String,
    /// Path relative to the project root.
    pub file: String,
    pub description: Vec<String>,
    /// SHA-256 of the file the step was computed from; `None` when the file did not exist.
    pub before_sha256: Option<String>,
    pub after: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub format: u32,
    pub generated_by: String,
    pub service_name: Option<String>,
    pub environment: String,
    pub steps: Vec<Step>,
    pub manual_steps: Vec<String>,
    pub next_commands: Vec<String>,
    pub notes: Vec<String>,
}

fn sha256(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

/// Service names follow the ingestion rules: printable ASCII, no spaces, at most 128 bytes.
pub fn sanitize_service_name(raw: &str) -> Option<String> {
    let base = raw.rsplit('/').next().unwrap_or(raw);
    let s: String = base
        .chars()
        .map(|c| c.to_ascii_lowercase())
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '-' })
        .collect();
    let s = s.trim_matches('-').chars().take(64).collect::<String>();
    (!s.is_empty()).then_some(s)
}

fn validate_service_name(s: &str) -> anyhow::Result<String> {
    let ok = !s.is_empty() && s.len() <= 128 && s.bytes().all(|b| b.is_ascii_graphic());
    if ok {
        Ok(s.to_owned())
    } else {
        bail!("invalid service name {s:?}: printable ASCII without spaces, at most 128 bytes")
    }
}

fn indent_of(text: &str) -> Vec<u8> {
    text.lines()
        .nth(1)
        .map(|l| l.bytes().take_while(|b| *b == b' ' || *b == b'\t').collect::<Vec<u8>>())
        .filter(|i| !i.is_empty())
        .unwrap_or_else(|| b"  ".to_vec())
}

fn to_pretty(v: &Value, indent: &[u8]) -> anyhow::Result<String> {
    let mut out = Vec::new();
    let mut ser =
        serde_json::Serializer::with_formatter(&mut out, serde_json::ser::PrettyFormatter::with_indent(indent));
    v.serialize(&mut ser)?;
    Ok(String::from_utf8(out)? + "\n")
}

fn package_json_step(
    root: &Path,
    d: &Discovery,
    args: &PlanArgs,
    manual: &mut Vec<String>,
) -> anyhow::Result<Option<Step>> {
    let before = read_limited(&root.join("package.json"))?;
    let mut pkg: Value = serde_json::from_str(&before).context("package.json is not valid JSON")?;
    let obj = pkg.as_object_mut().ok_or_else(|| anyhow!("package.json is not an object"))?;
    let mut description = Vec::new();

    let deps = obj.entry("dependencies").or_insert_with(|| json!({}));
    let deps = deps.as_object_mut().ok_or_else(|| anyhow!("package.json dependencies is not an object"))?;
    if deps.get(SDK_PACKAGE).and_then(Value::as_str) != Some(args.sdk_spec.as_str()) {
        deps.insert(SDK_PACKAGE.into(), Value::String(args.sdk_spec.clone()));
        description.push(format!("add dependency {SDK_PACKAGE}@{}", args.sdk_spec));
    }

    if !d.controlhub_register_in_start {
        let start = d.start_script.clone();
        let new_start = match (&start, &d.entry) {
            (Some(s), Some(_)) if discover::node_entry(s).is_some() => {
                Some(s.replacen("node", &format!("node --import {SDK_REGISTER}"), 1))
            }
            (None, Some(main)) => Some(format!("node --import {SDK_REGISTER} {main}")),
            _ => None,
        };
        match new_start {
            Some(ns) => {
                let scripts = obj.entry("scripts").or_insert_with(|| json!({}));
                let scripts = scripts.as_object_mut().ok_or_else(|| anyhow!("package.json scripts is not an object"))?;
                scripts.insert("start".into(), Value::String(ns.clone()));
                description.push(format!("start script: `{ns}` (preloads the SDK before application code)"));
            }
            None => manual.push(format!(
                "Start the service with NODE_OPTIONS=\"--import {SDK_REGISTER}\" (the start script `{}` is not a plain `node` command)",
                start.unwrap_or_default()
            )),
        }
    }
    if description.is_empty() {
        return Ok(None);
    }
    Ok(Some(Step {
        id: "package-json".into(),
        file: "package.json".into(),
        description,
        before_sha256: Some(sha256(&before)),
        after: to_pretty(&pkg, &indent_of(&before))?,
    }))
}

fn env_example_step(root: &Path, service: &str, environment: &str) -> anyhow::Result<Option<Step>> {
    let path = root.join(".env.example");
    let before = if path.exists() { Some(read_limited(&path)?) } else { None };
    let text = before.clone().unwrap_or_default();
    let has_key =
        |k: &str| text.lines().any(|l| l.trim_start().strip_prefix(k).is_some_and(|r| r.trim_start().starts_with('=')));
    let wanted = [
        ("CONTROLHUB_SERVICE_NAME", service.to_owned()),
        ("CONTROLHUB_ENVIRONMENT", environment.to_owned()),
        ("CONTROLHUB_OTLP_ENDPOINT", "http://127.0.0.1:4318".to_owned()),
        // Never a real value: tokens belong in an untracked .env or a secret store.
        ("CONTROLHUB_INGEST_TOKEN", String::new()),
    ];
    let missing: Vec<&(&str, String)> = wanted.iter().filter(|(k, _)| !has_key(k)).collect();
    if missing.is_empty() {
        return Ok(None);
    }
    let mut after = text.clone();
    if !after.is_empty() && !after.ends_with('\n') {
        after.push('\n');
    }
    after.push_str("# ControlHub (the ingestion token is a secret: keep it in an untracked .env)\n");
    for (k, v) in &missing {
        after.push_str(&format!("{k}={v}\n"));
    }
    Ok(Some(Step {
        id: "env-example".into(),
        file: ".env.example".into(),
        description: vec![format!("document {}", missing.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(", "))],
        before_sha256: before.as_deref().map(sha256),
        after,
    }))
}

fn gitignore_step(root: &Path, d: &Discovery) -> anyhow::Result<Option<Step>> {
    if d.env_ignored_by_git != Some(false) {
        return Ok(None);
    }
    let before = read_limited(&root.join(".gitignore"))?;
    let mut after = before.clone();
    if !after.is_empty() && !after.ends_with('\n') {
        after.push('\n');
    }
    after.push_str(".env\n");
    Ok(Some(Step {
        id: "gitignore".into(),
        file: ".gitignore".into(),
        description: vec!["ignore .env so the ingestion token is not committed".into()],
        before_sha256: Some(sha256(&before)),
        after,
    }))
}

pub fn build(root: &Path, args: &PlanArgs) -> anyhow::Result<Plan> {
    let d = discover::discover(root)?;
    let mut plan = Plan {
        format: PLAN_FORMAT,
        generated_by: format!("controlhub-cli {}", env!("CARGO_PKG_VERSION")),
        service_name: None,
        environment: args.environment.clone(),
        steps: vec![],
        manual_steps: vec![],
        next_commands: vec![],
        notes: d.warnings.clone(),
    };
    if d.ecosystem != Ecosystem::Node {
        plan.manual_steps.push(
            "Configure your OpenTelemetry SDK to export OTLP/HTTP (protobuf) to CONTROLHUB_OTLP_ENDPOINT \
             (/v1/traces, /v1/logs) or OTLP/gRPC to port 4317, with header `authorization: Bearer <ingestion token>`."
                .into(),
        );
        return Ok(plan);
    }
    let service = match (&args.service_name, &d.package_name) {
        (Some(s), _) => validate_service_name(s)?,
        (None, Some(n)) => {
            sanitize_service_name(n).ok_or_else(|| anyhow!("cannot derive a service name; pass --service-name"))?
        }
        (None, None) => bail!("package.json has no name; pass --service-name"),
    };
    plan.service_name = Some(service.clone());
    let mut manual = Vec::new();
    plan.steps.extend(package_json_step(root, &d, args, &mut manual)?);
    plan.steps.extend(env_example_step(root, &service, &args.environment)?);
    plan.steps.extend(gitignore_step(root, &d)?);
    manual.push(
        "Create an ingestion token for the target environment (web UI → Tokens) and put it in an untracked .env \
         or your secret store as CONTROLHUB_INGEST_TOKEN. Node does not load .env automatically: \
         use `node --env-file=.env` (Node ≥ 20.6) or your process manager."
            .into(),
    );
    plan.manual_steps = manual;
    let pm = d.package_manager.as_deref().unwrap_or("npm");
    plan.next_commands = vec![format!("{pm} install"), "controlhub verify".into()];
    if args.sdk_spec.starts_with('^') || args.sdk_spec.starts_with('~') {
        plan.notes.push(format!(
            "{SDK_PACKAGE} is not published to a registry yet; install it with --sdk-spec file:<controlhub checkout>/packages/sdk-node"
        ));
    }
    plan.notes.push(
        "The SDK needs Node.js ≥ 20.6 for `--import` and captures HTTP and pg spans, logs via its logger, and events."
            .into(),
    );
    Ok(plan)
}

pub fn load(path: &Path) -> anyhow::Result<Plan> {
    let plan: Plan = serde_json::from_str(&read_limited(path)?).context("plan file is not a valid plan")?;
    if plan.format != PLAN_FORMAT {
        bail!("unsupported plan format {}", plan.format);
    }
    Ok(plan)
}

/// Resolves a plan file name to a path in the root. Rejects anything but allowlisted names.
fn target(root: &Path, file: &str) -> anyhow::Result<PathBuf> {
    let rel = Path::new(file);
    let single = rel.components().count() == 1 && matches!(rel.components().next(), Some(Component::Normal(_)));
    if !single || !ALLOWED_FILES.contains(&file) {
        bail!("plan step targets {file:?}, which apply is not allowed to modify");
    }
    let path = root.join(rel);
    if let Ok(meta) = std::fs::symlink_metadata(&path)
        && (meta.file_type().is_symlink() || !meta.is_file())
    {
        bail!("{file} is not a regular file");
    }
    Ok(path)
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum StepState {
    Pending,
    AlreadyApplied,
    Applied,
}

fn diff(file: &str, before: &str, after: &str) -> String {
    similar::TextDiff::from_lines(before, after)
        .unified_diff()
        .context_radius(2)
        .header(&format!("a/{file}"), &format!("b/{file}"))
        .to_string()
}

fn write_atomic(path: &Path, content: &str) -> anyhow::Result<()> {
    let name = path.file_name().and_then(|n| n.to_str()).ok_or_else(|| anyhow!("bad file name"))?;
    let tmp = path.with_file_name(format!(".{name}.controlhub-tmp"));
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;
    Ok(())
}

pub fn apply(root: &Path, plan: &Plan, yes: bool) -> anyhow::Result<Outcome> {
    let root = root.canonicalize().with_context(|| format!("{} does not exist", root.display()))?;
    // Validate everything before writing anything.
    let mut checked = Vec::new();
    let mut drift = Vec::new();
    for step in &plan.steps {
        let path = target(&root, &step.file)?;
        let current = if path.exists() { Some(read_limited(&path)?) } else { None };
        let state = if current.as_deref() == Some(step.after.as_str()) {
            StepState::AlreadyApplied
        } else if current.as_deref().map(sha256) != step.before_sha256 {
            drift.push(step.file.clone());
            StepState::Pending
        } else {
            StepState::Pending
        };
        checked.push((step, path, current, state));
    }
    if !drift.is_empty() {
        return Ok(Outcome {
            json: json!({ "applied": false, "error": "files changed since the plan was made; run `controlhub plan` again", "changed_files": drift }),
            code: 2,
        });
    }
    let mut report = Vec::new();
    let pending = checked.iter().any(|c| c.3 == StepState::Pending);
    for (step, path, current, mut state) in checked {
        let d = (state == StepState::Pending).then(|| diff(&step.file, current.as_deref().unwrap_or(""), &step.after));
        if state == StepState::Pending && yes {
            write_atomic(&path, &step.after).with_context(|| format!("writing {}", step.file))?;
            state = StepState::Applied;
        }
        report.push(
            json!({ "id": step.id, "file": step.file, "state": state, "description": step.description, "diff": d }),
        );
    }
    if pending && !yes {
        for r in &report {
            if let Some(d) = r.get("diff").and_then(Value::as_str) {
                eprint!("{d}");
            }
        }
        eprintln!("nothing written; re-run with --yes to apply");
    }
    Ok(Outcome {
        json: json!({
            "applied": yes || !pending,
            "service_name": plan.service_name,
            "steps": report,
            "manual_steps": plan.manual_steps,
            "next_commands": plan.next_commands,
            "notes": plan.notes,
        }),
        code: if pending && !yes { 2 } else { 0 },
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn args() -> PlanArgs {
        PlanArgs { service_name: None, environment: "local".into(), sdk_spec: "file:../sdk".into() }
    }

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            "{\n    \"name\": \"@acme/Shop API\",\n    \"scripts\": {\n        \"start\": \"node src/server.js\"\n    }\n}\n",
        )
        .unwrap();
        std::fs::write(dir.path().join(".gitignore"), "node_modules").unwrap();
        std::fs::write(dir.path().join(".env"), "CONTROLHUB_INGEST_TOKEN=chi_secret").unwrap();
        dir
    }

    #[test]
    fn plan_apply_is_idempotent_and_detects_drift() {
        let dir = fixture();
        let plan = build(dir.path(), &args()).unwrap();
        assert_eq!(plan.service_name.as_deref(), Some("shop-api"));
        assert_eq!(
            plan.steps.iter().map(|s| s.file.as_str()).collect::<Vec<_>>(),
            ["package.json", ".env.example", ".gitignore"]
        );

        let dry = apply(dir.path(), &plan, false).unwrap();
        assert_eq!(dry.code, 2);
        assert!(
            !std::fs::read_to_string(dir.path().join("package.json")).unwrap().contains(SDK_PACKAGE),
            "dry run writes nothing"
        );

        assert_eq!(apply(dir.path(), &plan, true).unwrap().code, 0);
        let pkg = std::fs::read_to_string(dir.path().join("package.json")).unwrap();
        assert!(pkg.contains("\"start\": \"node --import @controlhub/sdk-node/register src/server.js\""));
        assert!(pkg.starts_with("{\n    \"name\""), "keeps 4-space indentation and key order");
        let env = std::fs::read_to_string(dir.path().join(".env.example")).unwrap();
        assert!(env.contains("CONTROLHUB_SERVICE_NAME=shop-api\n") && env.contains("CONTROLHUB_INGEST_TOKEN=\n"));
        assert!(!env.contains("chi_secret"));
        assert_eq!(std::fs::read_to_string(dir.path().join(".gitignore")).unwrap(), "node_modules\n.env\n");

        let again = apply(dir.path(), &plan, true).unwrap();
        assert!(again.json["steps"].as_array().unwrap().iter().all(|s| s["state"] == "already_applied"));
        assert!(build(dir.path(), &args()).unwrap().steps.is_empty(), "nothing left to do");

        let dir2 = fixture();
        let plan2 = build(dir2.path(), &args()).unwrap();
        std::fs::write(dir2.path().join("package.json"), "{\"name\":\"changed\"}").unwrap();
        let drifted = apply(dir2.path(), &plan2, true).unwrap();
        assert_eq!(drifted.code, 2);
        assert!(!dir2.path().join(".env.example").exists(), "nothing is written when any file drifted");
    }

    #[test]
    fn tampered_plans_cannot_escape_the_allowlist() {
        let dir = fixture();
        let mut plan = build(dir.path(), &args()).unwrap();
        for bad in ["../evil.json", "src/server.js", ".env", "/etc/passwd", "C:\\x"] {
            plan.steps[0].file = bad.into();
            assert!(apply(dir.path(), &plan, true).is_err(), "{bad} must be rejected");
        }
    }

    #[test]
    fn service_names_are_sanitized() {
        assert_eq!(sanitize_service_name("@scope/My App").as_deref(), Some("my-app"));
        assert_eq!(sanitize_service_name("@@@").as_deref(), None);
        assert!(validate_service_name("has space").is_err());
    }
}
