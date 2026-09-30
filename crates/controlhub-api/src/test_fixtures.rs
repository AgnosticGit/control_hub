//! Accounts and scopes that only the integration and e2e suites use. Created right before a
//! test run and removed after it (`scripts/test-with-fixtures.mjs`), so the local database
//! keeps no accounts besides the owner. Passwords and tokens are generated per run and written
//! to the gitignored `.controlhub/test-credentials.json`.

use std::path::Path;

use controlhub_storage::secrets::{generate_password, hash_password};
use serde_json::json;

use crate::bootstrap::{SeedScope, TEST_EMAIL_DOMAINS, admin_pg, rotate_ingest_token};

const CREDENTIALS_FILE: &str = "test-credentials.json";
const ISOLATION_TOKEN: &str = "isolation-test";

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

/// (email, display name, project, role).
const USERS: [(&str, &str, &str, &str); 4] = [
    ("admin@controlhub.local", "Test admin", "demo", "owner"),
    ("viewer@controlhub.local", "Test viewer", "demo", "viewer"),
    ("operator@controlhub.local", "Test operator", "demo", "operator"),
    ("isolation@controlhub.local", "Isolation tenant user", "isolation", "owner"),
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
        pg.upsert_membership(id, project, role).await?;
        users.push(json!({ "email": email, "password": password, "project_id": project, "role": role }));
    }
    let token = rotate_ingest_token(&pg, &SCOPES[0].scope()?, ISOLATION_TOKEN).await?;
    let creds = json!({
        "warning": "Test accounts for one test run; deleted afterwards. Never commit.",
        "users": users,
        "ingest_tokens": { ISOLATION_TOKEN: token },
    });
    std::fs::create_dir_all(out_dir)?;
    std::fs::write(out_dir.join(CREDENTIALS_FILE), serde_json::to_string_pretty(&creds)?)?;
    tracing::info!(users = USERS.len(), "test accounts created");
    Ok(())
}

/// Deletes every account in a test domain (the fixtures above and accounts tests signed up)
/// and revokes the isolation ingestion token.
pub async fn remove(out_dir: &Path) -> anyhow::Result<()> {
    let pg = admin_pg().await?;
    let removed = pg.delete_users_in_domains(TEST_EMAIL_DOMAINS).await?;
    pg.revoke_ingest_tokens_named(&SCOPES[0].scope()?, ISOLATION_TOKEN).await?;
    match std::fs::remove_file(out_dir.join(CREDENTIALS_FILE)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
        _ => {}
    }
    tracing::info!(removed, "test accounts removed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_test_account_is_in_a_domain_that_removal_covers() {
        for (email, ..) in USERS {
            let domain = email.rsplit('@').next().unwrap_or_default();
            assert!(TEST_EMAIL_DOMAINS.contains(&domain), "{email} would survive the cleanup");
        }
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
