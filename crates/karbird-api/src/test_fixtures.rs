//! Accounts and scopes that only the integration and e2e suites use. Created right before a
//! test run and removed after it (`scripts/test-with-fixtures.mjs`), so the local database
//! keeps no accounts besides the owner. Passwords and tokens are generated per run and written
//! to the gitignored `.karbird/test-credentials.json`.

use std::path::Path;

use karbird_storage::secrets::{generate_password, hash_password};
use serde_json::json;

use crate::bootstrap::{SeedScope, TEST_EMAIL_DOMAINS, admin_pg, rotate_ingest_token};
use crate::permissions::Builtin;
use crate::system_permissions::SystemBuiltin;

const CREDENTIALS_FILE: &str = "test-credentials.json";
const ISOLATION_TOKEN: &str = "isolation-test";
/// Role templates and projects created by the test suites start with this and are deleted with
/// the accounts.
const TEST_ROLE_PREFIX: &str = "[test] ";

/// A second tenant for isolation tests; `isolation-prod` is not a demo environment, so
/// demo-only actions must be refused there.
const SCOPES: [SeedScope; 2] = [
    SeedScope {
        tenant: "isolation-tenant",
        project: "isolation",
        env: "isolation-test",
        tenant_name: "Isolation test tenant",
        project_name: "Isolation",
        env_name: "isolation-test",
        is_demo: true,
    },
    SeedScope {
        tenant: "isolation-tenant",
        project: "isolation",
        env: "isolation-prod",
        tenant_name: "Isolation test tenant",
        project_name: "Isolation",
        env_name: "isolation-prod",
        is_demo: false,
    },
];

/// (email, display name, project, built-in role). `member-a`/`member-b` are only for the role
/// and member tests, which change their permissions; the others keep theirs for the whole run.
const USERS: [(&str, &str, &str, Builtin); 6] = [
    ("admin@karbird.local", "Test admin", "demo", Builtin::Owner),
    ("viewer@karbird.local", "Test viewer", "demo", Builtin::Viewer),
    ("operator@karbird.local", "Test operator", "demo", Builtin::Operator),
    ("isolation@karbird.local", "Isolation tenant user", "isolation", Builtin::Owner),
    ("member-a@karbird.local", "Member A", "demo", Builtin::Viewer),
    ("member-b@karbird.local", "Member B", "demo", Builtin::Viewer),
];

/// (email, display name, karbird role) for the admin panel tests; no project memberships.
/// `sysmember` starts as a plain account that the tests promote, disable and enable.
const SYSTEM_USERS: [(&str, &str, Option<SystemBuiltin>); 4] = [
    ("superadmin@karbird.local", "Test super admin", Some(SystemBuiltin::SuperAdmin)),
    ("sysadmin@karbird.local", "Test administrator", Some(SystemBuiltin::Admin)),
    ("auditor@karbird.local", "Test auditor", Some(SystemBuiltin::Auditor)),
    ("sysmember@karbird.local", "System member", None),
];

pub async fn create(out_dir: &Path) -> anyhow::Result<()> {
    let pg = admin_pg().await?;
    for s in &SCOPES {
        pg.upsert_scope(&s.scope()?, s.tenant_name, s.project_name, s.env_name, s.is_demo).await?;
    }
    let mut users = Vec::new();
    for (email, name, project, role) in USERS {
        let password = generate_password()?;
        let pw = password.clone();
        let hash = tokio::task::spawn_blocking(move || hash_password(&pw)).await??;
        let id = pg.upsert_user(email, name, &hash).await?;
        pg.upsert_membership(id, project, role.key()).await?;
        users.push(json!({ "email": email, "password": password, "project_id": project, "role": role.key() }));
    }
    for (email, name, system_role) in SYSTEM_USERS {
        let password = generate_password()?;
        let pw = password.clone();
        let hash = tokio::task::spawn_blocking(move || hash_password(&pw)).await??;
        let id = pg.upsert_user(email, name, &hash).await?;
        if let Some(role) = system_role {
            pg.upsert_system_builtin(id, role.key()).await?;
        }
        users.push(json!({
            "email": email, "password": password, "project_id": null, "role": null,
            "system_role": system_role.map(SystemBuiltin::key),
        }));
    }
    let token = rotate_ingest_token(&pg, &SCOPES[0].scope()?, ISOLATION_TOKEN).await?;
    let creds = json!({
        "warning": "Test accounts for one test run; deleted afterwards. Never commit.",
        "users": users,
        "ingest_tokens": { ISOLATION_TOKEN: token },
    });
    std::fs::create_dir_all(out_dir)?;
    std::fs::write(out_dir.join(CREDENTIALS_FILE), serde_json::to_string_pretty(&creds)?)?;
    tracing::info!(users = USERS.len() + SYSTEM_USERS.len(), "test accounts created");
    Ok(())
}

/// Deletes every account in a test domain (the fixtures above and accounts tests signed up)
/// and revokes the isolation ingestion token.
pub async fn remove(out_dir: &Path) -> anyhow::Result<()> {
    let pg = admin_pg().await?;
    let removed = pg.delete_users_in_domains(TEST_EMAIL_DOMAINS).await?;
    let roles = pg.delete_custom_roles_with_prefix(TEST_ROLE_PREFIX).await?
        + pg.delete_system_roles_with_prefix(TEST_ROLE_PREFIX).await?;
    let mut projects = 0;
    for id in pg.project_ids_named_with_prefix(TEST_ROLE_PREFIX).await? {
        projects += u64::from(pg.delete_project(&id).await?);
    }
    pg.revoke_ingest_tokens_named(&SCOPES[0].scope()?, ISOLATION_TOKEN).await?;
    match std::fs::remove_file(out_dir.join(CREDENTIALS_FILE)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
        _ => {}
    }
    tracing::info!(removed, roles, projects, "test accounts, roles and projects removed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_test_account_is_in_a_domain_that_removal_covers() {
        let emails = USERS.iter().map(|u| u.0).chain(SYSTEM_USERS.iter().map(|u| u.0));
        for email in emails {
            let domain = email.rsplit('@').next().unwrap_or_default();
            assert!(TEST_EMAIL_DOMAINS.contains(&domain), "{email} would survive the cleanup");
        }
    }

    #[test]
    fn test_projects_never_match_the_demo_project() {
        assert!(!"Demo system".starts_with(TEST_ROLE_PREFIX));
        assert!(!TEST_ROLE_PREFIX.trim().is_empty(), "an empty prefix would delete every project");
    }

    #[test]
    fn test_scopes_are_valid_and_never_the_demo_project() {
        for s in &SCOPES {
            let scope = s.scope().map_err(|e| e.to_string());
            assert!(scope.is_ok(), "{scope:?}");
            assert_ne!(s.project, "demo");
        }
    }
}
