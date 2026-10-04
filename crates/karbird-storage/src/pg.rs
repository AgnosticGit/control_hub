//! PostgreSQL: tenants/projects, users/sessions, ingestion tokens, consumer
//! offsets, dead-letter records and audit.

use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::{FromRow, Row};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{Result, StorageError};
use karbird_domain::{EnvironmentId, ProjectId, Scope, TenantId};

pub type OffsetDateTimeAlias = OffsetDateTime;

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../infra/migrations/postgres");

#[derive(Debug, Clone)]
pub struct PgStore {
    pool: PgPool,
}

#[derive(Debug, Clone, FromRow)]
pub struct UserRow {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub password_hash: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct SessionRow {
    pub session_id: Uuid,
    pub user_id: Uuid,
    pub email: String,
    pub display_name: String,
    pub csrf_token_hash: Vec<u8>,
    pub expires_at: OffsetDateTime,
}

/// A user's membership in a project with the effective permissions (template or individual set).
#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct MembershipRow {
    pub tenant_id: String,
    pub project_id: String,
    pub project_name: String,
    /// Built-in role key (`owner`, `admin`, …) when the member holds a built-in role.
    pub role_key: Option<String>,
    /// Template name, or `Individual` for an individual permission set.
    pub role_name: String,
    pub role_id: Option<Uuid>,
    pub permissions: Vec<String>,
}

impl MembershipRow {
    pub fn is_owner(&self) -> bool {
        self.role_key.as_deref() == Some("owner")
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct RoleRow {
    pub id: Uuid,
    pub name: String,
    pub permissions: Vec<String>,
    pub builtin: Option<String>,
    pub holders: i64,
}

#[derive(Debug, Clone, FromRow)]
pub struct MemberRow {
    pub user_id: Uuid,
    pub email: String,
    pub display_name: String,
    pub role_id: Option<Uuid>,
    pub role_key: Option<String>,
    pub role_name: String,
    pub based_on: Option<Uuid>,
    pub permissions: Vec<String>,
    pub created_at: OffsetDateTime,
}

impl MemberRow {
    pub fn is_owner(&self) -> bool {
        self.role_key.as_deref() == Some("owner")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleChange {
    Done,
    NotFound,
    Builtin,
    /// Another role of the project already has this name.
    NameTaken,
    /// Still held by this many members (delete only).
    Held(i64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberChange {
    Done,
    /// No such member, or the role is not in this project.
    NotFound,
    /// The change would leave the project without an owner (rolled back).
    LastOwner,
}

/// The signed-in user's own account.
#[derive(Debug, Clone, FromRow)]
pub struct AccountRow {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub password_hash: String,
    pub created_at: OffsetDateTime,
    pub onboarding_completed_at: Option<OffsetDateTime>,
}

/// An active browser session of a user (no IP or user agent is stored).
#[derive(Debug, Clone, FromRow)]
pub struct SessionInfoRow {
    pub id: Uuid,
    pub created_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
}

#[derive(Debug, Clone)]
pub struct NewInvite<'a> {
    pub project_id: &'a str,
    pub role_id: Uuid,
    pub token_hash: &'a [u8],
    pub token_prefix: &'a str,
    pub created_by: Uuid,
    pub expires_at: OffsetDateTime,
    pub max_uses: Option<i32>,
}

/// An invite link as listed to project admins (never the secret).
#[derive(Debug, Clone, FromRow)]
pub struct InviteRow {
    pub id: Uuid,
    pub role_id: Uuid,
    pub role_name: String,
    pub token_prefix: String,
    pub created_by: Uuid,
    pub created_by_name: String,
    pub created_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
    pub max_uses: Option<i32>,
    pub uses: i32,
}

/// An invite found by its secret, with what accepting it would grant.
#[derive(Debug, Clone, FromRow)]
pub struct InviteLookup {
    pub id: Uuid,
    pub tenant_id: String,
    pub project_id: String,
    pub project_name: String,
    pub role_id: Uuid,
    pub role_name: String,
    pub role_key: Option<String>,
    pub role_permissions: Vec<String>,
    pub created_by: Uuid,
    pub inviter_name: String,
    pub expires_at: OffsetDateTime,
    pub max_uses: Option<i32>,
    pub uses: i32,
    pub revoked_at: Option<OffsetDateTime>,
}

impl InviteLookup {
    /// Not revoked, not expired and with uses left at `now`.
    pub fn is_usable(&self, now: OffsetDateTime) -> bool {
        self.revoked_at.is_none() && self.expires_at > now && self.max_uses.is_none_or(|m| self.uses < m)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteAccept {
    Joined,
    /// Already a member: nothing changed and no use was spent.
    AlreadyMember,
    /// Revoked, expired, used up or its role is gone.
    Gone,
}

/// Columns shared by membership queries (`m` = memberships, `r` = its role, if any).
const MEMBERSHIP_GRANT: &str = "r.builtin AS role_key, COALESCE(r.name, 'Individual') AS role_name, m.role_id,
     COALESCE(m.permissions, r.permissions) AS permissions";

#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct EnvironmentRow {
    pub id: String,
    pub project_id: String,
    pub tenant_id: String,
    pub name: String,
    pub is_demo: bool,
}

#[derive(Debug, Clone)]
pub struct IngestCredential {
    pub token_id: Uuid,
    pub scope: Scope,
    pub signals: Vec<String>,
    pub expires_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Ingest,
    Api,
}

impl TokenKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TokenKind::Ingest => "ingest",
            TokenKind::Api => "api",
        }
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct TokenRow {
    pub id: Uuid,
    pub kind: String,
    pub name: String,
    pub token_prefix: String,
    pub signals: Option<Vec<String>>,
    pub created_at: OffsetDateTime,
    pub expires_at: Option<OffsetDateTime>,
    pub revoked_at: Option<OffsetDateTime>,
    pub last_used_at: Option<OffsetDateTime>,
    pub created_by: Option<Uuid>,
}

#[derive(Debug, Clone)]
pub struct ApiTokenPrincipal {
    pub token_id: Uuid,
    pub scope: Scope,
    pub user_id: Uuid,
    pub email: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsumerOffset {
    pub next_offset: i64,
}

#[derive(Debug, Clone, FromRow)]
pub struct WatermarkRow {
    pub partition: i32,
    pub next_offset: i64,
    pub high_watermark: i64,
    pub last_received_time_ns: i64,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone)]
pub struct NewAuditEntry<'a> {
    pub tenant_id: Option<&'a str>,
    pub project_id: Option<&'a str>,
    pub actor_id: &'a str,
    pub actor_kind: &'a str,
    pub action: &'a str,
    pub target: Option<&'a str>,
    pub reason: Option<&'a str>,
    pub before: Option<serde_json::Value>,
    pub after: Option<serde_json::Value>,
    pub outcome: &'a str,
    pub request_id: Option<&'a str>,
}

#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct AuditRow {
    pub id: Uuid,
    #[serde(with = "time::serde::rfc3339")]
    pub occurred_at: OffsetDateTime,
    pub actor_id: String,
    pub actor_kind: String,
    pub action: String,
    pub target: Option<String>,
    pub reason: Option<String>,
    pub outcome: String,
}

#[derive(Debug, Clone)]
pub struct NewAccount<'a> {
    pub email: &'a str,
    pub display_name: &'a str,
    pub password_hash: &'a str,
    pub project_name: &'a str,
    pub environment_name: &'a str,
}

#[derive(Debug, Clone)]
pub struct CreatedAccount {
    pub user_id: Uuid,
    pub scope: Scope,
}

/// Random, unguessable ids for a signed-up tenant, so they never collide with seeded or
/// other customers' ids and reveal nothing about how many accounts exist.
/// `LIKE` patterns for addresses in exactly these domains (lowercased, wildcards escaped;
/// blank domains dropped).
fn email_domain_patterns(domains: &[&str]) -> Vec<String> {
    domains
        .iter()
        .map(|d| d.trim().to_ascii_lowercase())
        .filter(|d| !d.is_empty())
        .map(|d| format!("%@{}", like_escape(&d)))
        .collect()
}

/// Escapes LIKE wildcards so the value only matches itself.
fn like_escape(s: &str) -> String {
    s.replace('\\', r"\\").replace('%', r"\%").replace('_', r"\_")
}

fn new_account_scope() -> Result<Scope> {
    let id = |prefix: &str| format!("{prefix}-{}", Uuid::new_v4().simple());
    scope_from(id("t"), id("p"), id("e"))
}

fn scope_from(t: String, p: String, e: String) -> Result<Scope> {
    let err = |e: karbird_domain::IdError| StorageError::Other(format!("corrupt scope in database: {e}"));
    Ok(Scope {
        tenant_id: TenantId::parse(t).map_err(err)?,
        project_id: ProjectId::parse(p).map_err(err)?,
        environment_id: EnvironmentId::parse(e).map_err(err)?,
    })
}

impl PgStore {
    pub async fn connect(url: &str, max_connections: u32) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(std::time::Duration::from_secs(5))
            .connect(url)
            .await?;
        Ok(Self { pool })
    }

    /// Like [`PgStore::connect`], but connects on first use (no I/O here).
    pub fn connect_lazy(url: &str, max_connections: u32) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(std::time::Duration::from_secs(5))
            .connect_lazy(url)?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn migrate(&self) -> Result<()> {
        MIGRATOR.run(&self.pool).await?;
        Ok(())
    }

    pub async fn ping(&self) -> Result<()> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }

    // ------------------------------------------------------------ bootstrap

    /// Create (or update the password of) least-privilege service roles.
    /// Passwords are generated by bootstrap and restricted to base64url characters.
    pub async fn ensure_service_role(&self, role: &str, password: &str, grants: &[&str]) -> Result<()> {
        let ok = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        if !ok(role) || !ok(password) {
            return Err(StorageError::Other("invalid role name or password charset".into()));
        }
        let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = $1)")
            .bind(role)
            .fetch_one(&self.pool)
            .await?;
        let verb = if exists { "ALTER" } else { "CREATE" };
        sqlx::query(&format!("{verb} ROLE {role} LOGIN PASSWORD '{password}'")).execute(&self.pool).await?;
        sqlx::query(&format!("GRANT CONNECT ON DATABASE karbird TO {role}")).execute(&self.pool).await?;
        sqlx::query(&format!("GRANT USAGE ON SCHEMA public TO {role}")).execute(&self.pool).await?;
        for g in grants {
            sqlx::query(&format!("GRANT {g} TO {role}")).execute(&self.pool).await?;
        }
        Ok(())
    }

    pub async fn upsert_scope(
        &self,
        scope: &Scope,
        tenant_name: &str,
        project_name: &str,
        env_name: &str,
        is_demo: bool,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("INSERT INTO tenants (id, name) VALUES ($1, $2) ON CONFLICT (id) DO NOTHING")
            .bind(scope.tenant_id.as_str())
            .bind(tenant_name)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO projects (id, tenant_id, name) VALUES ($1, $2, $3) ON CONFLICT (id) DO NOTHING")
            .bind(scope.project_id.as_str())
            .bind(scope.tenant_id.as_str())
            .bind(project_name)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO environments (id, tenant_id, project_id, name, is_demo) VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (id) DO UPDATE SET is_demo = EXCLUDED.is_demo",
        )
        .bind(scope.environment_id.as_str())
        .bind(scope.tenant_id.as_str())
        .bind(scope.project_id.as_str())
        .bind(env_name)
        .bind(is_demo)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Create or reset a user's password. Returns the user id. Seeded accounts skip onboarding.
    pub async fn upsert_user(&self, email: &str, display_name: &str, password_hash: &str) -> Result<Uuid> {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO users (id, email, display_name, password_hash, onboarding_completed_at) VALUES ($1, $2, $3, $4, now())
             ON CONFLICT (email) DO UPDATE SET password_hash = EXCLUDED.password_hash
             RETURNING id",
        )
        .bind(Uuid::now_v7())
        .bind(email)
        .bind(display_name)
        .bind(password_hash)
        .fetch_one(&self.pool)
        .await?;
        Ok(id)
    }

    /// Self-service sign-up: a new user who owns a fresh tenant, project and environment,
    /// created atomically. Returns `None` (and creates nothing) if the email is taken.
    /// Never touches an existing user, unlike [`PgStore::upsert_user`].
    pub async fn create_account(&self, a: &NewAccount<'_>) -> Result<Option<CreatedAccount>> {
        let mut tx = self.pool.begin().await?;
        let user_id: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO users (id, email, display_name, password_hash) VALUES ($1, $2, $3, $4)
             ON CONFLICT (email) DO NOTHING RETURNING id",
        )
        .bind(Uuid::now_v7())
        .bind(a.email)
        .bind(a.display_name)
        .bind(a.password_hash)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(user_id) = user_id else {
            tx.rollback().await?;
            return Ok(None);
        };
        let scope =
            Self::insert_owned_project(&mut tx, user_id, a.display_name, a.project_name, a.environment_name).await?;
        tx.commit().await?;
        Ok(Some(CreatedAccount { user_id, scope }))
    }

    /// A new project in its own new tenant with one environment, owned by `user_id`.
    pub async fn create_project(&self, user_id: Uuid, project_name: &str, environment_name: &str) -> Result<Scope> {
        let mut tx = self.pool.begin().await?;
        let scope = Self::insert_owned_project(&mut tx, user_id, project_name, project_name, environment_name).await?;
        tx.commit().await?;
        Ok(scope)
    }

    async fn insert_owned_project(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        user_id: Uuid,
        tenant_name: &str,
        project_name: &str,
        environment_name: &str,
    ) -> Result<Scope> {
        let scope = new_account_scope()?;
        sqlx::query("INSERT INTO tenants (id, name) VALUES ($1, $2)")
            .bind(scope.tenant_id.as_str())
            .bind(tenant_name)
            .execute(&mut **tx)
            .await?;
        sqlx::query("INSERT INTO projects (id, tenant_id, name) VALUES ($1, $2, $3)")
            .bind(scope.project_id.as_str())
            .bind(scope.tenant_id.as_str())
            .bind(project_name)
            .execute(&mut **tx)
            .await?;
        sqlx::query(
            "INSERT INTO environments (id, tenant_id, project_id, name, is_demo) VALUES ($1, $2, $3, $4, FALSE)",
        )
        .bind(scope.environment_id.as_str())
        .bind(scope.tenant_id.as_str())
        .bind(scope.project_id.as_str())
        .bind(environment_name)
        .execute(&mut **tx)
        .await?;
        // The built-in roles were created by the projects trigger in this transaction.
        sqlx::query(
            "INSERT INTO memberships (user_id, project_id, role_id)
             SELECT $1, $2, id FROM project_roles WHERE project_id = $2 AND builtin = 'owner'",
        )
        .bind(user_id)
        .bind(scope.project_id.as_str())
        .execute(&mut **tx)
        .await?;
        Ok(scope)
    }

    /// Projects in which the user holds the Owner role.
    pub async fn owned_project_count(&self, user_id: Uuid) -> Result<i64> {
        Ok(sqlx::query_scalar(
            "SELECT count(*) FROM memberships m JOIN project_roles r ON r.id = m.role_id
             WHERE m.user_id = $1 AND r.builtin = 'owner'",
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?)
    }

    /// `false` when another project of the tenant already has this name.
    pub async fn rename_project(&self, project_id: &str, name: &str) -> Result<bool> {
        match sqlx::query("UPDATE projects SET name = $2 WHERE id = $1")
            .bind(project_id)
            .bind(name)
            .execute(&self.pool)
            .await
        {
            Ok(_) => Ok(true),
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    /// Deletes every user whose email is in one of `domains`, with their sessions, memberships,
    /// API tokens and action plans. Local test fixtures only (needs the admin connection).
    /// Returns the number of users deleted.
    pub async fn delete_users_in_domains(&self, domains: &[&str]) -> Result<u64> {
        let patterns = email_domain_patterns(domains);
        let mut tx = self.pool.begin().await?;
        let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM users WHERE lower(email) LIKE ANY($1)")
            .bind(&patterns)
            .fetch_all(&mut *tx)
            .await?;
        if ids.is_empty() {
            tx.rollback().await?;
            return Ok(0);
        }
        for sql in [
            "DELETE FROM action_plans WHERE created_by = ANY($1) OR decided_by = ANY($1)",
            "DELETE FROM api_tokens WHERE created_by = ANY($1)",
            "UPDATE ingest_tokens SET created_by = NULL WHERE created_by = ANY($1)",
            "DELETE FROM sessions WHERE user_id = ANY($1)",
            "DELETE FROM memberships WHERE user_id = ANY($1)",
            "DELETE FROM users WHERE id = ANY($1)",
        ] {
            sqlx::query(sql).bind(&ids).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(ids.len() as u64)
    }

    /// Gives a user a built-in role (`owner`, `admin`, `operator`, `viewer`) in a project.
    pub async fn upsert_membership(&self, user_id: Uuid, project_id: &str, builtin: &str) -> Result<()> {
        let r = sqlx::query(
            "INSERT INTO memberships (user_id, project_id, role_id)
             SELECT $1, $2, id FROM project_roles WHERE project_id = $2 AND builtin = $3
             ON CONFLICT (user_id, project_id) DO UPDATE
                SET role_id = EXCLUDED.role_id, permissions = NULL, based_on = NULL",
        )
        .bind(user_id)
        .bind(project_id)
        .bind(builtin)
        .execute(&self.pool)
        .await?;
        if r.rows_affected() == 0 {
            return Err(StorageError::Other(format!("no built-in role {builtin:?} in project {project_id}")));
        }
        Ok(())
    }

    pub async fn create_ingest_token(
        &self,
        scope: &Scope,
        name: &str,
        token_hash: &[u8],
        prefix: &str,
        expires_at: Option<OffsetDateTime>,
        created_by: Option<Uuid>,
    ) -> Result<Uuid> {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO ingest_tokens (id, tenant_id, project_id, environment_id, name, token_prefix, token_hash, expires_at, created_by)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(id)
        .bind(scope.tenant_id.as_str())
        .bind(scope.project_id.as_str())
        .bind(scope.environment_id.as_str())
        .bind(name)
        .bind(prefix)
        .bind(token_hash)
        .bind(expires_at)
        .bind(created_by)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    /// Revoke all active tokens with the given name in scope (used for rotation).
    pub async fn revoke_ingest_tokens_named(&self, scope: &Scope, name: &str) -> Result<u64> {
        let r = sqlx::query(
            "UPDATE ingest_tokens SET revoked_at = now()
             WHERE tenant_id = $1 AND project_id = $2 AND environment_id = $3 AND name = $4 AND revoked_at IS NULL",
        )
        .bind(scope.tenant_id.as_str())
        .bind(scope.project_id.as_str())
        .bind(scope.environment_id.as_str())
        .bind(name)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected())
    }

    // ------------------------------------------------------------ token management

    /// Ingestion and API tokens of one environment (metadata only; hashes are never returned).
    pub async fn list_tokens(&self, scope: &Scope) -> Result<Vec<TokenRow>> {
        Ok(sqlx::query_as::<_, TokenRow>(
            "SELECT id, 'ingest' AS kind, name, token_prefix, signals, created_at, expires_at, revoked_at,
                    NULL::timestamptz AS last_used_at, created_by
             FROM ingest_tokens WHERE tenant_id = $1 AND project_id = $2 AND environment_id = $3
             UNION ALL
             SELECT id, 'api' AS kind, name, token_prefix, NULL::text[] AS signals, created_at, expires_at, revoked_at,
                    last_used_at, created_by
             FROM api_tokens WHERE tenant_id = $1 AND project_id = $2 AND environment_id = $3
             ORDER BY created_at DESC",
        )
        .bind(scope.tenant_id.as_str())
        .bind(scope.project_id.as_str())
        .bind(scope.environment_id.as_str())
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn find_token(&self, scope: &Scope, kind: TokenKind, id: Uuid) -> Result<Option<TokenRow>> {
        Ok(self.list_tokens(scope).await?.into_iter().find(|t| t.id == id && t.kind == kind.as_str()))
    }

    pub async fn create_api_token(
        &self,
        scope: &Scope,
        name: &str,
        token_hash: &[u8],
        prefix: &str,
        expires_at: OffsetDateTime,
        created_by: Uuid,
    ) -> Result<Uuid> {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO api_tokens (id, tenant_id, project_id, environment_id, name, token_prefix, token_hash, expires_at, created_by)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(id)
        .bind(scope.tenant_id.as_str())
        .bind(scope.project_id.as_str())
        .bind(scope.environment_id.as_str())
        .bind(name)
        .bind(prefix)
        .bind(token_hash)
        .bind(expires_at)
        .bind(created_by)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    /// Makes a token unusable after `at` (now for revocation, now + grace for rotation).
    /// Returns false when the token does not exist in scope or is already revoked.
    pub async fn end_token(&self, scope: &Scope, kind: TokenKind, id: Uuid, at: OffsetDateTime) -> Result<bool> {
        let table = match kind {
            TokenKind::Ingest => "ingest_tokens",
            TokenKind::Api => "api_tokens",
        };
        let revoke_now = at <= OffsetDateTime::now_utc();
        let set = if revoke_now { "revoked_at = now()" } else { "expires_at = LEAST(COALESCE(expires_at, $5), $5)" };
        let sql = format!(
            "UPDATE {table} SET {set}
             WHERE id = $1 AND tenant_id = $2 AND project_id = $3 AND environment_id = $4 AND revoked_at IS NULL"
        );
        let mut q = sqlx::query(&sql)
            .bind(id)
            .bind(scope.tenant_id.as_str())
            .bind(scope.project_id.as_str())
            .bind(scope.environment_id.as_str());
        if !revoke_now {
            q = q.bind(at);
        }
        Ok(q.execute(&self.pool).await?.rows_affected() == 1)
    }

    // ------------------------------------------------------------ API token auth

    /// Active API token with its creator. Disabled users' tokens stop working immediately.
    pub async fn find_api_token(&self, token_hash: &[u8]) -> Result<Option<ApiTokenPrincipal>> {
        let row = sqlx::query(
            "SELECT t.id, t.tenant_id, t.project_id, t.environment_id, u.id AS user_id, u.email, u.display_name
             FROM api_tokens t JOIN users u ON u.id = t.created_by
             WHERE t.token_hash = $1 AND t.revoked_at IS NULL AND t.expires_at > now() AND u.disabled_at IS NULL",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else { return Ok(None) };
        let token_id: Uuid = row.try_get("id")?;
        sqlx::query(
            "UPDATE api_tokens SET last_used_at = now()
             WHERE id = $1 AND (last_used_at IS NULL OR last_used_at < now() - interval '1 minute')",
        )
        .bind(token_id)
        .execute(&self.pool)
        .await?;
        Ok(Some(ApiTokenPrincipal {
            token_id,
            scope: scope_from(row.try_get("tenant_id")?, row.try_get("project_id")?, row.try_get("environment_id")?)?,
            user_id: row.try_get("user_id")?,
            email: row.try_get("email")?,
            display_name: row.try_get("display_name")?,
        }))
    }

    pub async fn api_token_is_active(&self, token_id: Uuid) -> Result<bool> {
        let row: Option<(i32,)> = sqlx::query_as(
            "SELECT 1 FROM api_tokens t JOIN users u ON u.id = t.created_by
             WHERE t.id = $1 AND t.revoked_at IS NULL AND t.expires_at > now() AND u.disabled_at IS NULL",
        )
        .bind(token_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    // ------------------------------------------------------------ ingestion auth

    /// Lookup an active (not revoked, not expired) ingestion token by hash.
    pub async fn find_ingest_token(&self, token_hash: &[u8]) -> Result<Option<IngestCredential>> {
        let row = sqlx::query(
            "SELECT id, tenant_id, project_id, environment_id, signals, expires_at FROM ingest_tokens
             WHERE token_hash = $1 AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at > now())",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else { return Ok(None) };
        Ok(Some(IngestCredential {
            token_id: row.try_get("id")?,
            scope: scope_from(row.try_get("tenant_id")?, row.try_get("project_id")?, row.try_get("environment_id")?)?,
            signals: row.try_get("signals")?,
            expires_at: row.try_get("expires_at")?,
        }))
    }

    // ------------------------------------------------------------ users & sessions

    /// The user with this (normalised) email, including disabled ones.
    pub async fn user_id_by_email(&self, email: &str) -> Result<Option<Uuid>> {
        Ok(sqlx::query_scalar("SELECT id FROM users WHERE email = $1").bind(email).fetch_optional(&self.pool).await?)
    }

    pub async fn find_user_by_email(&self, email: &str) -> Result<Option<UserRow>> {
        Ok(sqlx::query_as::<_, UserRow>(
            "SELECT id, email, display_name, password_hash FROM users WHERE email = $1 AND disabled_at IS NULL",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn create_session(
        &self,
        user_id: Uuid,
        token_hash: &[u8],
        csrf_hash: &[u8],
        ttl: time::Duration,
    ) -> Result<Uuid> {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO sessions (id, user_id, token_hash, csrf_token_hash, expires_at) VALUES ($1, $2, $3, $4, now() + $5)",
        )
        .bind(id)
        .bind(user_id)
        .bind(token_hash)
        .bind(csrf_hash)
        .bind(ttl)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    pub async fn find_session(&self, token_hash: &[u8]) -> Result<Option<SessionRow>> {
        Ok(sqlx::query_as::<_, SessionRow>(
            "SELECT s.id AS session_id, s.user_id, u.email, u.display_name, s.csrf_token_hash, s.expires_at
             FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE s.token_hash = $1 AND s.revoked_at IS NULL AND s.expires_at > now() AND u.disabled_at IS NULL",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Long-lived connections (WebSocket) re-check this so logout/expiry closes them.
    pub async fn session_is_active(&self, session_id: Uuid) -> Result<bool> {
        let row: Option<(i32,)> = sqlx::query_as(
            "SELECT 1 FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE s.id = $1 AND s.revoked_at IS NULL AND s.expires_at > now() AND u.disabled_at IS NULL",
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    pub async fn revoke_session(&self, session_id: Uuid) -> Result<()> {
        sqlx::query("UPDATE sessions SET revoked_at = now() WHERE id = $1")
            .bind(session_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn memberships(&self, user_id: Uuid) -> Result<Vec<MembershipRow>> {
        Ok(sqlx::query_as::<_, MembershipRow>(&format!(
            "SELECT p.tenant_id, m.project_id, p.name AS project_name, {MEMBERSHIP_GRANT}
             FROM memberships m JOIN projects p ON p.id = m.project_id
             LEFT JOIN project_roles r ON r.id = m.role_id
             WHERE m.user_id = $1 ORDER BY p.name"
        ))
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?)
    }

    // ------------------------------------------------------------ project roles and members

    pub async fn roles(&self, project_id: &str) -> Result<Vec<RoleRow>> {
        Ok(sqlx::query_as::<_, RoleRow>(
            "SELECT r.id, r.name, r.permissions, r.builtin,
                    (SELECT count(*) FROM memberships m WHERE m.role_id = r.id) AS holders
             FROM project_roles r WHERE r.project_id = $1
             ORDER BY r.builtin IS NULL, array_position(ARRAY['owner','admin','operator','viewer'], r.builtin), lower(r.name)",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn role(&self, project_id: &str, role_id: Uuid) -> Result<Option<RoleRow>> {
        Ok(sqlx::query_as::<_, RoleRow>(
            "SELECT r.id, r.name, r.permissions, r.builtin,
                    (SELECT count(*) FROM memberships m WHERE m.role_id = r.id) AS holders
             FROM project_roles r WHERE r.project_id = $1 AND r.id = $2",
        )
        .bind(project_id)
        .bind(role_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Creates a custom role. `None` when the name is taken in this project.
    pub async fn create_role(&self, project_id: &str, name: &str, permissions: &[String]) -> Result<Option<Uuid>> {
        Ok(sqlx::query_scalar(
            "INSERT INTO project_roles (project_id, name, permissions) VALUES ($1, $2, $3)
             ON CONFLICT DO NOTHING RETURNING id",
        )
        .bind(project_id)
        .bind(name)
        .bind(permissions)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn update_role(
        &self,
        project_id: &str,
        role_id: Uuid,
        name: &str,
        permissions: &[String],
    ) -> Result<RoleChange> {
        let Some(role) = self.role(project_id, role_id).await? else { return Ok(RoleChange::NotFound) };
        if role.builtin.is_some() {
            return Ok(RoleChange::Builtin);
        }
        let r = sqlx::query(
            "UPDATE project_roles SET name = $3, permissions = $4 WHERE project_id = $1 AND id = $2 AND builtin IS NULL",
        )
        .bind(project_id)
        .bind(role_id)
        .bind(name)
        .bind(permissions)
        .execute(&self.pool)
        .await;
        match r {
            Ok(r) if r.rows_affected() == 1 => Ok(RoleChange::Done),
            Ok(_) => Ok(RoleChange::NotFound),
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => Ok(RoleChange::NameTaken),
            Err(e) => Err(e.into()),
        }
    }

    pub async fn delete_role(&self, project_id: &str, role_id: Uuid) -> Result<RoleChange> {
        let mut tx = self.pool.begin().await?;
        let row: Option<(Option<String>, i64)> = sqlx::query_as(
            "SELECT r.builtin, (SELECT count(*) FROM memberships m WHERE m.role_id = r.id)
             FROM project_roles r WHERE r.project_id = $1 AND r.id = $2 FOR UPDATE",
        )
        .bind(project_id)
        .bind(role_id)
        .fetch_optional(&mut *tx)
        .await?;
        let result = match row {
            None => RoleChange::NotFound,
            Some((Some(_), _)) => RoleChange::Builtin,
            Some((None, n)) if n > 0 => RoleChange::Held(n),
            Some((None, _)) => {
                sqlx::query("DELETE FROM project_roles WHERE project_id = $1 AND id = $2")
                    .bind(project_id)
                    .bind(role_id)
                    .execute(&mut *tx)
                    .await?;
                RoleChange::Done
            }
        };
        tx.commit().await?;
        Ok(result)
    }

    /// Sets the permissions of a built-in role in every project (built-ins follow the code).
    pub async fn sync_builtin_role(&self, builtin: &str, permissions: &[String]) -> Result<u64> {
        let r = sqlx::query(
            "UPDATE project_roles SET permissions = $2 WHERE builtin = $1 AND permissions IS DISTINCT FROM $2",
        )
        .bind(builtin)
        .bind(permissions)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected())
    }

    /// Deletes custom roles (any project) whose name starts with `prefix`; members holding one
    /// are switched to an empty individual set first. Used to clean up after test runs.
    pub async fn delete_custom_roles_with_prefix(&self, prefix: &str) -> Result<u64> {
        if prefix.trim().is_empty() {
            return Err(StorageError::Other("an empty prefix would match every custom role".into()));
        }
        let pattern = format!("{}%", like_escape(prefix));
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE memberships SET role_id = NULL, permissions = '{}' WHERE role_id IN
                (SELECT id FROM project_roles WHERE builtin IS NULL AND name LIKE $1)",
        )
        .bind(&pattern)
        .execute(&mut *tx)
        .await?;
        let r = sqlx::query("DELETE FROM project_roles WHERE builtin IS NULL AND name LIKE $1")
            .bind(&pattern)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(r.rows_affected())
    }

    pub async fn members(&self, project_id: &str) -> Result<Vec<MemberRow>> {
        Ok(sqlx::query_as::<_, MemberRow>(&format!(
            "SELECT u.id AS user_id, u.email, u.display_name, m.based_on, m.created_at, {MEMBERSHIP_GRANT}
             FROM memberships m JOIN users u ON u.id = m.user_id
             LEFT JOIN project_roles r ON r.id = m.role_id
             WHERE m.project_id = $1 ORDER BY lower(u.display_name), u.email"
        ))
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn member(&self, project_id: &str, user_id: Uuid) -> Result<Option<MemberRow>> {
        Ok(sqlx::query_as::<_, MemberRow>(&format!(
            "SELECT u.id AS user_id, u.email, u.display_name, m.based_on, m.created_at, {MEMBERSHIP_GRANT}
             FROM memberships m JOIN users u ON u.id = m.user_id
             LEFT JOIN project_roles r ON r.id = m.role_id
             WHERE m.project_id = $1 AND m.user_id = $2"
        ))
        .bind(project_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn count_owners(&self, project_id: &str) -> Result<i64> {
        Ok(sqlx::query_scalar(
            "SELECT count(*) FROM memberships m JOIN project_roles r ON r.id = m.role_id
             WHERE m.project_id = $1 AND r.builtin = 'owner'",
        )
        .bind(project_id)
        .fetch_one(&self.pool)
        .await?)
    }

    /// Serialises member changes of one project and returns its owner count at that point.
    async fn lock_members(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, project_id: &str) -> Result<i64> {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('memberships:' || $1, 0))")
            .bind(project_id)
            .execute(&mut **tx)
            .await?;
        Ok(sqlx::query_scalar(
            "SELECT count(*) FROM memberships m JOIN project_roles r ON r.id = m.role_id
             WHERE m.project_id = $1 AND r.builtin = 'owner'",
        )
        .bind(project_id)
        .fetch_one(&mut **tx)
        .await?)
    }

    /// Commits a member change unless it touched no row or took away the project's last owner.
    async fn finish_member_change(
        mut tx: sqlx::Transaction<'_, sqlx::Postgres>,
        project_id: &str,
        owners_before: i64,
        affected: u64,
    ) -> Result<MemberChange> {
        if affected != 1 {
            return Ok(MemberChange::NotFound);
        }
        let owners_after: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM memberships m JOIN project_roles r ON r.id = m.role_id
             WHERE m.project_id = $1 AND r.builtin = 'owner'",
        )
        .bind(project_id)
        .fetch_one(&mut *tx)
        .await?;
        if owners_before > 0 && owners_after == 0 {
            return Ok(MemberChange::LastOwner);
        }
        tx.commit().await?;
        Ok(MemberChange::Done)
    }

    /// Points a member at a role of the same project.
    pub async fn set_member_role(&self, project_id: &str, user_id: Uuid, role_id: Uuid) -> Result<MemberChange> {
        let mut tx = self.pool.begin().await?;
        let owners = Self::lock_members(&mut tx, project_id).await?;
        let r = sqlx::query(
            "UPDATE memberships SET role_id = $3, permissions = NULL, based_on = NULL
             WHERE project_id = $1 AND user_id = $2
               AND EXISTS (SELECT 1 FROM project_roles WHERE id = $3 AND project_id = $1)",
        )
        .bind(project_id)
        .bind(user_id)
        .bind(role_id)
        .execute(&mut *tx)
        .await?;
        Self::finish_member_change(tx, project_id, owners, r.rows_affected()).await
    }

    /// Gives a member an individual permission set; `based_on` (a role of the project) is for display.
    pub async fn set_member_permissions(
        &self,
        project_id: &str,
        user_id: Uuid,
        permissions: &[String],
        based_on: Option<Uuid>,
    ) -> Result<MemberChange> {
        let mut tx = self.pool.begin().await?;
        let owners = Self::lock_members(&mut tx, project_id).await?;
        let r = sqlx::query(
            "UPDATE memberships SET role_id = NULL, permissions = $3,
                    based_on = (SELECT id FROM project_roles WHERE id = $4 AND project_id = $1)
             WHERE project_id = $1 AND user_id = $2",
        )
        .bind(project_id)
        .bind(user_id)
        .bind(permissions)
        .bind(based_on)
        .execute(&mut *tx)
        .await?;
        Self::finish_member_change(tx, project_id, owners, r.rows_affected()).await
    }

    pub async fn remove_member(&self, project_id: &str, user_id: Uuid) -> Result<MemberChange> {
        let mut tx = self.pool.begin().await?;
        let owners = Self::lock_members(&mut tx, project_id).await?;
        let r = sqlx::query("DELETE FROM memberships WHERE project_id = $1 AND user_id = $2")
            .bind(project_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        Self::finish_member_change(tx, project_id, owners, r.rows_affected()).await
    }

    // ------------------------------------------------------------ invite links

    /// `None` when the role is not a role of this project.
    pub async fn create_invite(&self, i: &NewInvite<'_>) -> Result<Option<Uuid>> {
        Ok(sqlx::query_scalar(
            "INSERT INTO project_invites (id, project_id, role_id, token_hash, token_prefix, created_by, expires_at, max_uses)
             SELECT $1, $2, $3, $4, $5, $6, $7, $8
             WHERE EXISTS (SELECT 1 FROM project_roles WHERE id = $3 AND project_id = $2)
             RETURNING id",
        )
        .bind(Uuid::now_v7())
        .bind(i.project_id)
        .bind(i.role_id)
        .bind(i.token_hash)
        .bind(i.token_prefix)
        .bind(i.created_by)
        .bind(i.expires_at)
        .bind(i.max_uses)
        .fetch_optional(&self.pool)
        .await?)
    }

    const INVITE_COLUMNS: &str = "i.id, i.role_id, r.name AS role_name, i.token_prefix, i.created_by,
        u.display_name AS created_by_name, i.created_at, i.expires_at, i.max_uses, i.uses
        FROM project_invites i JOIN project_roles r ON r.id = i.role_id JOIN users u ON u.id = i.created_by";

    /// Links that can still be used, newest first.
    pub async fn invites(&self, project_id: &str) -> Result<Vec<InviteRow>> {
        Ok(sqlx::query_as::<_, InviteRow>(&format!(
            "SELECT {} WHERE i.project_id = $1 AND i.revoked_at IS NULL AND i.expires_at > now()
               AND (i.max_uses IS NULL OR i.uses < i.max_uses)
             ORDER BY i.created_at DESC LIMIT 200",
            Self::INVITE_COLUMNS
        ))
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn invite(&self, project_id: &str, id: Uuid) -> Result<Option<InviteRow>> {
        Ok(sqlx::query_as::<_, InviteRow>(&format!(
            "SELECT {} WHERE i.project_id = $1 AND i.id = $2",
            Self::INVITE_COLUMNS
        ))
        .bind(project_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// `false` when there is no such active link in the project.
    pub async fn revoke_invite(&self, project_id: &str, id: Uuid) -> Result<bool> {
        let r = sqlx::query(
            "UPDATE project_invites SET revoked_at = now() WHERE project_id = $1 AND id = $2 AND revoked_at IS NULL",
        )
        .bind(project_id)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() == 1)
    }

    pub async fn find_invite(&self, token_hash: &[u8]) -> Result<Option<InviteLookup>> {
        Ok(sqlx::query_as::<_, InviteLookup>(
            "SELECT i.id, p.tenant_id, i.project_id, p.name AS project_name, i.role_id, r.name AS role_name,
                    r.builtin AS role_key, r.permissions AS role_permissions, i.created_by,
                    u.display_name AS inviter_name, i.expires_at, i.max_uses, i.uses, i.revoked_at
             FROM project_invites i JOIN projects p ON p.id = i.project_id
             JOIN project_roles r ON r.id = i.role_id JOIN users u ON u.id = i.created_by
             WHERE i.token_hash = $1",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Adds the user to the invite's project with its role, spending one use. The link is
    /// re-checked under a row lock, so concurrent accepts never exceed `max_uses`.
    pub async fn accept_invite(&self, invite_id: Uuid, user_id: Uuid) -> Result<InviteAccept> {
        let mut tx = self.pool.begin().await?;
        let row: Option<(String, Uuid)> = sqlx::query_as(
            "SELECT project_id, role_id FROM project_invites
             WHERE id = $1 AND revoked_at IS NULL AND expires_at > now() AND (max_uses IS NULL OR uses < max_uses)
             FOR UPDATE",
        )
        .bind(invite_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((project_id, role_id)) = row else { return Ok(InviteAccept::Gone) };
        let already: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM memberships WHERE user_id = $1 AND project_id = $2)")
                .bind(user_id)
                .bind(&project_id)
                .fetch_one(&mut *tx)
                .await?;
        if already {
            return Ok(InviteAccept::AlreadyMember);
        }
        let r = sqlx::query(
            "INSERT INTO memberships (user_id, project_id, role_id)
             SELECT $1, $2, $3 WHERE EXISTS (SELECT 1 FROM project_roles WHERE id = $3 AND project_id = $2)
             ON CONFLICT (user_id, project_id) DO NOTHING",
        )
        .bind(user_id)
        .bind(&project_id)
        .bind(role_id)
        .execute(&mut *tx)
        .await?;
        if r.rows_affected() != 1 {
            return Ok(InviteAccept::Gone);
        }
        sqlx::query("UPDATE project_invites SET uses = uses + 1 WHERE id = $1")
            .bind(invite_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(InviteAccept::Joined)
    }

    // ------------------------------------------------------------ own account

    pub async fn account(&self, user_id: Uuid) -> Result<Option<AccountRow>> {
        Ok(sqlx::query_as::<_, AccountRow>(
            "SELECT id, email, display_name, password_hash, created_at, onboarding_completed_at
             FROM users WHERE id = $1 AND disabled_at IS NULL",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn set_display_name(&self, user_id: Uuid, display_name: &str) -> Result<()> {
        sqlx::query("UPDATE users SET display_name = $2 WHERE id = $1")
            .bind(user_id)
            .bind(display_name)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Marks onboarding finished (now) or not finished.
    pub async fn set_onboarding_done(&self, user_id: Uuid, done: bool) -> Result<()> {
        sqlx::query(
            "UPDATE users SET onboarding_completed_at = CASE WHEN $2 THEN COALESCE(onboarding_completed_at, now()) END
             WHERE id = $1",
        )
        .bind(user_id)
        .bind(done)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Stores a new password hash and signs out every other session. Returns how many were ended.
    pub async fn change_password(&self, user_id: Uuid, password_hash: &str, keep_session: Uuid) -> Result<u64> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE users SET password_hash = $2 WHERE id = $1")
            .bind(user_id)
            .bind(password_hash)
            .execute(&mut *tx)
            .await?;
        let r = sqlx::query(
            "UPDATE sessions SET revoked_at = now() WHERE user_id = $1 AND id <> $2 AND revoked_at IS NULL",
        )
        .bind(user_id)
        .bind(keep_session)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(r.rows_affected())
    }

    pub async fn active_sessions(&self, user_id: Uuid) -> Result<Vec<SessionInfoRow>> {
        Ok(sqlx::query_as::<_, SessionInfoRow>(
            "SELECT id, created_at, expires_at FROM sessions
             WHERE user_id = $1 AND revoked_at IS NULL AND expires_at > now() ORDER BY created_at DESC LIMIT 100",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Ends one of the user's own sessions; `false` if it is not theirs or already ended.
    pub async fn revoke_user_session(&self, user_id: Uuid, session_id: Uuid) -> Result<bool> {
        let r =
            sqlx::query("UPDATE sessions SET revoked_at = now() WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL")
                .bind(session_id)
                .bind(user_id)
                .execute(&self.pool)
                .await?;
        Ok(r.rows_affected() == 1)
    }

    pub async fn revoke_other_sessions(&self, user_id: Uuid, keep_session: Uuid) -> Result<u64> {
        let r = sqlx::query(
            "UPDATE sessions SET revoked_at = now() WHERE user_id = $1 AND id <> $2 AND revoked_at IS NULL",
        )
        .bind(user_id)
        .bind(keep_session)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected())
    }

    pub async fn environments_for_projects(&self, project_ids: &[String]) -> Result<Vec<EnvironmentRow>> {
        Ok(sqlx::query_as::<_, EnvironmentRow>(
            "SELECT id, project_id, tenant_id, name, is_demo FROM environments WHERE project_id = ANY($1) ORDER BY name",
        )
        .bind(project_ids)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn environment(&self, project_id: &str, environment_id: &str) -> Result<Option<EnvironmentRow>> {
        Ok(sqlx::query_as::<_, EnvironmentRow>(
            "SELECT id, project_id, tenant_id, name, is_demo FROM environments WHERE project_id = $1 AND id = $2",
        )
        .bind(project_id)
        .bind(environment_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    // ------------------------------------------------------------ consumer offsets

    pub async fn load_offset(&self, consumer: &str, topic: &str, partition: i32) -> Result<Option<ConsumerOffset>> {
        let v: Option<i64> = sqlx::query_scalar(
            "SELECT next_offset FROM consumer_offsets WHERE consumer = $1 AND topic = $2 AND partition = $3",
        )
        .bind(consumer)
        .bind(topic)
        .bind(partition)
        .fetch_optional(&self.pool)
        .await?;
        Ok(v.map(|next_offset| ConsumerOffset { next_offset }))
    }

    pub async fn commit_offset(
        &self,
        consumer: &str,
        topic: &str,
        partition: i32,
        next_offset: i64,
        high_watermark: i64,
        last_received_time_ns: i64,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO consumer_offsets (consumer, topic, partition, next_offset, high_watermark, last_received_time_ns, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, now())
             ON CONFLICT (consumer, topic, partition) DO UPDATE SET
               next_offset = GREATEST(consumer_offsets.next_offset, EXCLUDED.next_offset),
               high_watermark = EXCLUDED.high_watermark,
               last_received_time_ns = GREATEST(consumer_offsets.last_received_time_ns, EXCLUDED.last_received_time_ns),
               updated_at = now()",
        )
        .bind(consumer)
        .bind(topic)
        .bind(partition)
        .bind(next_offset)
        .bind(high_watermark)
        .bind(last_received_time_ns)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn watermarks(&self, consumer: &str, topic: &str) -> Result<Vec<WatermarkRow>> {
        Ok(sqlx::query_as::<_, WatermarkRow>(
            "SELECT partition, next_offset, high_watermark, last_received_time_ns, updated_at
             FROM consumer_offsets WHERE consumer = $1 AND topic = $2 ORDER BY partition",
        )
        .bind(consumer)
        .bind(topic)
        .fetch_all(&self.pool)
        .await?)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn record_processing_error(
        &self,
        consumer: &str,
        topic: &str,
        partition: i32,
        broker_offset: i64,
        batch_id: Option<&str>,
        tenant_id: Option<&str>,
        kind: &str,
        message: &str,
        payload: Option<&[u8]>,
    ) -> Result<Uuid> {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO processing_errors (id, consumer, topic, partition, broker_offset, batch_id, tenant_id, error_kind, error_message, payload)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             ON CONFLICT (consumer, topic, partition, broker_offset) DO NOTHING",
        )
        .bind(id)
        .bind(consumer)
        .bind(topic)
        .bind(partition)
        .bind(broker_offset)
        .bind(batch_id)
        .bind(tenant_id)
        .bind(kind)
        .bind(message)
        .bind(payload)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    pub async fn count_unresolved_processing_errors(&self, tenant_id: &str) -> Result<i64> {
        Ok(sqlx::query_scalar(
            "SELECT count(*) FROM processing_errors WHERE resolved_at IS NULL AND (tenant_id = $1 OR tenant_id IS NULL)",
        )
        .bind(tenant_id)
        .fetch_one(&self.pool)
        .await?)
    }

    // ------------------------------------------------------------ audit

    pub async fn audit(&self, e: NewAuditEntry<'_>) -> Result<Uuid> {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO audit_log (id, tenant_id, project_id, actor_id, actor_kind, action, target, reason, before, after, outcome, request_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        )
        .bind(id)
        .bind(e.tenant_id)
        .bind(e.project_id)
        .bind(e.actor_id)
        .bind(e.actor_kind)
        .bind(e.action)
        .bind(e.target)
        .bind(e.reason)
        .bind(e.before)
        .bind(e.after)
        .bind(e.outcome)
        .bind(e.request_id)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    pub async fn audit_entries_for_target(
        &self,
        tenant_id: &str,
        project_id: &str,
        target: &str,
    ) -> Result<Vec<AuditRow>> {
        Ok(sqlx::query_as::<_, AuditRow>(
            "SELECT id, occurred_at, actor_id, actor_kind, action, target, reason, outcome FROM audit_log
             WHERE tenant_id = $1 AND project_id = $2 AND target = $3 ORDER BY occurred_at, id LIMIT 200",
        )
        .bind(tenant_id)
        .bind(project_id)
        .bind(target)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn audit_entries(&self, tenant_id: &str, project_id: &str, limit: i64) -> Result<Vec<AuditRow>> {
        Ok(sqlx::query_as::<_, AuditRow>(
            "SELECT id, occurred_at, actor_id, actor_kind, action, target, reason, outcome FROM audit_log
             WHERE tenant_id = $1 AND project_id = $2 ORDER BY occurred_at DESC LIMIT $3",
        )
        .bind(tenant_id)
        .bind(project_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn scope_from_accepts_valid_ids() {
        let s = scope_from("local".into(), "demo".into(), "local-demo".into()).unwrap();
        assert_eq!(
            (s.tenant_id.as_str(), s.project_id.as_str(), s.environment_id.as_str()),
            ("local", "demo", "local-demo")
        );
    }

    #[test]
    fn scope_from_rejects_corrupt_rows_instead_of_widening_scope() {
        for (t, p, e) in [
            ("", "demo", "env"),
            ("local", "", "env"),
            ("local", "demo", ""),
            ("local", "demo", "env' OR '1'='1"),
            ("local/other", "demo", "env"),
            ("local", &"p".repeat(129) as &str, "env"),
        ] {
            match scope_from(t.into(), p.into(), e.into()) {
                Err(StorageError::Other(m)) => assert!(m.contains("corrupt scope"), "{m}"),
                other => panic!("{t:?}/{p:?}/{e:?} must be refused, got {other:?}"),
            }
        }
    }

    #[test]
    fn email_domain_patterns_match_only_that_exact_domain() {
        assert_eq!(email_domain_patterns(&["karbird.local", "Example.TEST"]), ["%@karbird.local", "%@example.test"]);
        // LIKE wildcards in a domain are escaped, so they can never widen a delete.
        assert_eq!(email_domain_patterns(&["%", "a_b.test", r"x\y"]), [r"%@\%", r"%@a\_b.test", r"%@x\\y"]);
        assert!(email_domain_patterns(&["", "  "]).is_empty(), "an empty domain would match every address ending in @");
    }

    #[test]
    fn invites_are_usable_only_while_live_unrevoked_and_with_uses_left() {
        let now = OffsetDateTime::from_unix_timestamp(1_000_000).unwrap();
        let invite = InviteLookup {
            id: Uuid::nil(),
            tenant_id: "t".into(),
            project_id: "p".into(),
            project_name: "P".into(),
            role_id: Uuid::nil(),
            role_name: "Viewer".into(),
            role_key: Some("viewer".into()),
            role_permissions: vec![],
            created_by: Uuid::nil(),
            inviter_name: "A".into(),
            expires_at: now + time::Duration::seconds(1),
            max_uses: None,
            uses: 500,
            revoked_at: None,
        };
        assert!(invite.is_usable(now), "unlimited uses");
        assert!(InviteLookup { max_uses: Some(501), ..invite.clone() }.is_usable(now));
        assert!(!InviteLookup { max_uses: Some(500), ..invite.clone() }.is_usable(now), "used up");
        assert!(!InviteLookup { expires_at: now, ..invite.clone() }.is_usable(now), "expiry is exclusive");
        assert!(!InviteLookup { revoked_at: Some(now), ..invite }.is_usable(now), "revoked");
    }

    #[test]
    fn like_escape_neutralises_wildcards() {
        assert_eq!(like_escape("[test] "), "[test] ");
        assert_eq!(like_escape(r"50%_a\b"), r"50\%\_a\\b");
    }

    #[tokio::test]
    async fn an_empty_role_prefix_is_refused_before_touching_the_database() {
        let pg = PgStore::connect_lazy("postgres://u:p@127.0.0.1:1/x", 1).unwrap();
        for p in ["", "   "] {
            let e = pg.delete_custom_roles_with_prefix(p).await.unwrap_err();
            assert!(e.to_string().contains("empty prefix"), "{e}");
        }
    }

    #[test]
    fn signed_up_scopes_are_valid_random_and_never_collide_with_seeds() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            let s = new_account_scope().unwrap();
            let (t, p, e) = (s.tenant_id.as_str(), s.project_id.as_str(), s.environment_id.as_str());
            assert!(t.starts_with("t-") && p.starts_with("p-") && e.starts_with("e-"), "{t} {p} {e}");
            assert_eq!(p.len(), 34);
            for seeded in ["local", "demo", "local-demo", "isolation-tenant", "isolation", "isolation-test"] {
                assert!(t != seeded && p != seeded && e != seeded);
            }
            assert!(seen.insert(p.to_owned()), "duplicate project id {p}");
        }
    }

    #[test]
    fn token_kinds_map_to_the_sql_kind_column() {
        assert_eq!(TokenKind::Ingest.as_str(), "ingest");
        assert_eq!(TokenKind::Api.as_str(), "api");
    }

    #[tokio::test]
    async fn lazy_store_does_no_io_until_used() {
        // Port 1 is never a PostgreSQL server; construction must still succeed.
        assert!(PgStore::connect_lazy("postgres://u:p@127.0.0.1:1/karbird", 1).is_ok());
        assert!(PgStore::connect_lazy("not a url", 1).is_err());
    }

    #[tokio::test]
    async fn service_roles_refuse_injection_before_any_query() {
        let pg = PgStore::connect_lazy("postgres://u:p@127.0.0.1:1/karbird", 1).unwrap();
        for (role, pw) in [
            ("karbird_api", "x' SUPERUSER; --"),
            ("api; DROP TABLE users", "pw"),
            ("", "pw"),
            ("role", ""),
            ("role name", "pw"),
            ("role", "pw\"q"),
        ] {
            match pg.ensure_service_role(role, pw, &[]).await {
                Err(StorageError::Other(m)) => assert!(m.contains("charset"), "{m}"),
                other => panic!("{role:?}/{pw:?} must be refused before any query, got {other:?}"),
            }
        }
    }
}
