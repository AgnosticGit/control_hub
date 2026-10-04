//! karbird administration: installation-wide roles and grants (`system_roles`,
//! `system_memberships`) and the cross-project views and changes of the admin panel. Permission
//! checks happen in the API; these queries only keep the data consistent (at least one active
//! Super Admin, see [`MemberChange::LastOwner`]).

use sqlx::{FromRow, Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::pg::{EnvironmentRow, MemberChange, MemberRow, PgStore, RoleChange, RoleRow};
use crate::{Result, StorageError};

/// The top built-in system role; the one that must never disappear.
pub const SUPER_ADMIN: &str = "super_admin";

/// A user's karbird administrator grant with its effective permissions.
#[derive(Debug, Clone, FromRow)]
pub struct SystemGrantRow {
    pub role_key: Option<String>,
    pub role_name: String,
    pub role_id: Option<Uuid>,
    pub permissions: Vec<String>,
}

impl SystemGrantRow {
    pub fn is_super_admin(&self) -> bool {
        self.role_key.as_deref() == Some(SUPER_ADMIN)
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct AdminProjectRow {
    pub id: String,
    pub tenant_id: String,
    pub name: String,
    pub created_at: OffsetDateTime,
    pub members: i64,
    pub owners: Vec<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct AdminUserRow {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub created_at: OffsetDateTime,
    pub disabled_at: Option<OffsetDateTime>,
    pub projects: i64,
    pub active_sessions: i64,
    pub system_role: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct AdminCounts {
    pub projects: i64,
    pub environments: i64,
    pub users: i64,
    pub disabled_users: i64,
    pub active_sessions: i64,
    pub administrators: i64,
}

#[derive(Debug, Clone, FromRow)]
pub struct AdminAuditRow {
    pub id: Uuid,
    pub occurred_at: OffsetDateTime,
    pub tenant_id: Option<String>,
    pub project_id: Option<String>,
    pub actor_id: String,
    pub actor_email: Option<String>,
    pub actor_kind: String,
    pub action: String,
    pub target: Option<String>,
    pub reason: Option<String>,
    pub outcome: String,
}

/// Columns of a system grant (`m` = system_memberships, `r` = its role, if any).
const SYSTEM_GRANT: &str = "r.builtin AS role_key, COALESCE(r.name, 'Individual') AS role_name, m.role_id,
     COALESCE(m.permissions, r.permissions) AS permissions";

const ROLE_ORDER: &str =
    "r.builtin IS NULL, array_position(ARRAY['super_admin','admin','auditor'], r.builtin), lower(r.name)";

const PROJECT_COLUMNS: &str = "p.id, p.tenant_id, p.name, p.created_at,
     (SELECT count(*) FROM memberships m WHERE m.project_id = p.id) AS members,
     ARRAY(SELECT u.email FROM memberships m JOIN project_roles r ON r.id = m.role_id JOIN users u ON u.id = m.user_id
           WHERE m.project_id = p.id AND r.builtin = 'owner' ORDER BY u.email) AS owners";

const USER_COLUMNS: &str = "u.id, u.email, u.display_name, u.created_at, u.disabled_at,
     (SELECT count(*) FROM memberships m WHERE m.user_id = u.id) AS projects,
     (SELECT count(*) FROM sessions s WHERE s.user_id = u.id AND s.revoked_at IS NULL AND s.expires_at > now()) AS active_sessions,
     (SELECT COALESCE(r.name, 'Individual') FROM system_memberships m LEFT JOIN system_roles r ON r.id = m.role_id
      WHERE m.user_id = u.id) AS system_role";

/// Active Super Admins (disabled accounts do not count).
const COUNT_SUPER_ADMINS: &str = "SELECT count(*) FROM system_memberships m JOIN system_roles r ON r.id = m.role_id
     JOIN users u ON u.id = m.user_id WHERE r.builtin = 'super_admin' AND u.disabled_at IS NULL";

impl PgStore {
    // ------------------------------------------------------------ system grants and roles

    pub async fn system_grant(&self, user_id: Uuid) -> Result<Option<SystemGrantRow>> {
        Ok(sqlx::query_as::<_, SystemGrantRow>(&format!(
            "SELECT {SYSTEM_GRANT} FROM system_memberships m LEFT JOIN system_roles r ON r.id = m.role_id
             WHERE m.user_id = $1"
        ))
        .bind(user_id)
        .fetch_optional(self.pool())
        .await?)
    }

    pub async fn system_roles(&self) -> Result<Vec<RoleRow>> {
        Ok(sqlx::query_as::<_, RoleRow>(&format!(
            "SELECT r.id, r.name, r.permissions, r.builtin,
                    (SELECT count(*) FROM system_memberships m WHERE m.role_id = r.id) AS holders
             FROM system_roles r ORDER BY {ROLE_ORDER}"
        ))
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn system_role(&self, role_id: Uuid) -> Result<Option<RoleRow>> {
        Ok(sqlx::query_as::<_, RoleRow>(
            "SELECT r.id, r.name, r.permissions, r.builtin,
                    (SELECT count(*) FROM system_memberships m WHERE m.role_id = r.id) AS holders
             FROM system_roles r WHERE r.id = $1",
        )
        .bind(role_id)
        .fetch_optional(self.pool())
        .await?)
    }

    /// `None` when the name is taken.
    pub async fn create_system_role(&self, name: &str, permissions: &[String]) -> Result<Option<Uuid>> {
        Ok(sqlx::query_scalar(
            "INSERT INTO system_roles (name, permissions) VALUES ($1, $2) ON CONFLICT DO NOTHING RETURNING id",
        )
        .bind(name)
        .bind(permissions)
        .fetch_optional(self.pool())
        .await?)
    }

    pub async fn update_system_role(&self, role_id: Uuid, name: &str, permissions: &[String]) -> Result<RoleChange> {
        let Some(role) = self.system_role(role_id).await? else { return Ok(RoleChange::NotFound) };
        if role.builtin.is_some() {
            return Ok(RoleChange::Builtin);
        }
        let r = sqlx::query("UPDATE system_roles SET name = $2, permissions = $3 WHERE id = $1 AND builtin IS NULL")
            .bind(role_id)
            .bind(name)
            .bind(permissions)
            .execute(self.pool())
            .await;
        match r {
            Ok(r) if r.rows_affected() == 1 => Ok(RoleChange::Done),
            Ok(_) => Ok(RoleChange::NotFound),
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => Ok(RoleChange::NameTaken),
            Err(e) => Err(e.into()),
        }
    }

    pub async fn delete_system_role(&self, role_id: Uuid) -> Result<RoleChange> {
        let mut tx = self.pool().begin().await?;
        let row: Option<(Option<String>, i64)> = sqlx::query_as(
            "SELECT r.builtin, (SELECT count(*) FROM system_memberships m WHERE m.role_id = r.id)
             FROM system_roles r WHERE r.id = $1 FOR UPDATE",
        )
        .bind(role_id)
        .fetch_optional(&mut *tx)
        .await?;
        let result = match row {
            None => RoleChange::NotFound,
            Some((Some(_), _)) => RoleChange::Builtin,
            Some((None, n)) if n > 0 => RoleChange::Held(n),
            Some((None, _)) => {
                sqlx::query("DELETE FROM system_roles WHERE id = $1").bind(role_id).execute(&mut *tx).await?;
                RoleChange::Done
            }
        };
        tx.commit().await?;
        Ok(result)
    }

    /// Built-in system roles follow the code (like project built-ins).
    pub async fn sync_system_builtin_role(&self, builtin: &str, permissions: &[String]) -> Result<u64> {
        let r = sqlx::query(
            "UPDATE system_roles SET permissions = $2 WHERE builtin = $1 AND permissions IS DISTINCT FROM $2",
        )
        .bind(builtin)
        .bind(permissions)
        .execute(self.pool())
        .await?;
        Ok(r.rows_affected())
    }

    /// Deletes custom system roles whose name starts with `prefix` (test clean-up); holders get
    /// an empty individual set first.
    pub async fn delete_system_roles_with_prefix(&self, prefix: &str) -> Result<u64> {
        if prefix.trim().is_empty() {
            return Err(StorageError::Other("an empty prefix would match every custom role".into()));
        }
        let pattern = format!("{}%", prefix.replace('\\', r"\\").replace('%', r"\%").replace('_', r"\_"));
        let mut tx = self.pool().begin().await?;
        sqlx::query(
            "UPDATE system_memberships SET role_id = NULL, permissions = '{}' WHERE role_id IN
                (SELECT id FROM system_roles WHERE builtin IS NULL AND name LIKE $1)",
        )
        .bind(&pattern)
        .execute(&mut *tx)
        .await?;
        let r = sqlx::query("DELETE FROM system_roles WHERE builtin IS NULL AND name LIKE $1")
            .bind(&pattern)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(r.rows_affected())
    }

    // ------------------------------------------------------------ administrators

    pub async fn system_members(&self) -> Result<Vec<MemberRow>> {
        Ok(sqlx::query_as::<_, MemberRow>(&format!(
            "SELECT u.id AS user_id, u.email, u.display_name, m.based_on, m.created_at, {SYSTEM_GRANT}
             FROM system_memberships m JOIN users u ON u.id = m.user_id
             LEFT JOIN system_roles r ON r.id = m.role_id
             ORDER BY lower(u.display_name), u.email"
        ))
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn system_member(&self, user_id: Uuid) -> Result<Option<MemberRow>> {
        Ok(sqlx::query_as::<_, MemberRow>(&format!(
            "SELECT u.id AS user_id, u.email, u.display_name, m.based_on, m.created_at, {SYSTEM_GRANT}
             FROM system_memberships m JOIN users u ON u.id = m.user_id
             LEFT JOIN system_roles r ON r.id = m.role_id
             WHERE m.user_id = $1"
        ))
        .bind(user_id)
        .fetch_optional(self.pool())
        .await?)
    }

    pub async fn count_super_admins(&self) -> Result<i64> {
        Ok(sqlx::query_scalar(COUNT_SUPER_ADMINS).fetch_one(self.pool()).await?)
    }

    /// Serialises administrator changes and returns the number of active Super Admins.
    async fn lock_system(tx: &mut Transaction<'_, Postgres>) -> Result<i64> {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('system_memberships', 0))")
            .execute(&mut **tx)
            .await?;
        Ok(sqlx::query_scalar(COUNT_SUPER_ADMINS).fetch_one(&mut **tx).await?)
    }

    /// Commits unless no row was touched or no active Super Admin would remain.
    async fn finish_system_change(
        mut tx: Transaction<'_, Postgres>,
        before: i64,
        affected: u64,
    ) -> Result<MemberChange> {
        if affected != 1 {
            return Ok(MemberChange::NotFound);
        }
        let after: i64 = sqlx::query_scalar(COUNT_SUPER_ADMINS).fetch_one(&mut *tx).await?;
        if before > 0 && after == 0 {
            return Ok(MemberChange::LastOwner);
        }
        tx.commit().await?;
        Ok(MemberChange::Done)
    }

    /// Makes an active user an administrator with a role. `NotFound`: no such active user or
    /// role, or the user already is one.
    pub async fn add_system_member(&self, user_id: Uuid, role_id: Uuid) -> Result<MemberChange> {
        let mut tx = self.pool().begin().await?;
        let before = Self::lock_system(&mut tx).await?;
        let r = sqlx::query(
            "INSERT INTO system_memberships (user_id, role_id)
             SELECT u.id, r.id FROM users u, system_roles r WHERE u.id = $1 AND u.disabled_at IS NULL AND r.id = $2
             ON CONFLICT (user_id) DO NOTHING",
        )
        .bind(user_id)
        .bind(role_id)
        .execute(&mut *tx)
        .await?;
        Self::finish_system_change(tx, before, r.rows_affected()).await
    }

    pub async fn set_system_member_role(&self, user_id: Uuid, role_id: Uuid) -> Result<MemberChange> {
        let mut tx = self.pool().begin().await?;
        let before = Self::lock_system(&mut tx).await?;
        let r = sqlx::query(
            "UPDATE system_memberships SET role_id = $2, permissions = NULL, based_on = NULL
             WHERE user_id = $1 AND EXISTS (SELECT 1 FROM system_roles WHERE id = $2)",
        )
        .bind(user_id)
        .bind(role_id)
        .execute(&mut *tx)
        .await?;
        Self::finish_system_change(tx, before, r.rows_affected()).await
    }

    pub async fn set_system_member_permissions(
        &self,
        user_id: Uuid,
        permissions: &[String],
        based_on: Option<Uuid>,
    ) -> Result<MemberChange> {
        let mut tx = self.pool().begin().await?;
        let before = Self::lock_system(&mut tx).await?;
        let r = sqlx::query(
            "UPDATE system_memberships SET role_id = NULL, permissions = $2,
                    based_on = (SELECT id FROM system_roles WHERE id = $3)
             WHERE user_id = $1",
        )
        .bind(user_id)
        .bind(permissions)
        .bind(based_on)
        .execute(&mut *tx)
        .await?;
        Self::finish_system_change(tx, before, r.rows_affected()).await
    }

    pub async fn remove_system_member(&self, user_id: Uuid) -> Result<MemberChange> {
        let mut tx = self.pool().begin().await?;
        let before = Self::lock_system(&mut tx).await?;
        let r =
            sqlx::query("DELETE FROM system_memberships WHERE user_id = $1").bind(user_id).execute(&mut *tx).await?;
        Self::finish_system_change(tx, before, r.rows_affected()).await
    }

    /// Gives a user a built-in system role (bootstrap and test fixtures).
    pub async fn upsert_system_builtin(&self, user_id: Uuid, builtin: &str) -> Result<()> {
        let r = sqlx::query(
            "INSERT INTO system_memberships (user_id, role_id)
             SELECT $1, id FROM system_roles WHERE builtin = $2
             ON CONFLICT (user_id) DO UPDATE SET role_id = EXCLUDED.role_id, permissions = NULL, based_on = NULL",
        )
        .bind(user_id)
        .bind(builtin)
        .execute(self.pool())
        .await?;
        if r.rows_affected() == 0 {
            return Err(StorageError::Other(format!("no built-in system role {builtin:?}")));
        }
        Ok(())
    }

    // ------------------------------------------------------------ projects

    pub async fn admin_projects(&self) -> Result<Vec<AdminProjectRow>> {
        Ok(sqlx::query_as::<_, AdminProjectRow>(&format!(
            "SELECT {PROJECT_COLUMNS} FROM projects p ORDER BY lower(p.name), p.id LIMIT 1000"
        ))
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn admin_project(&self, project_id: &str) -> Result<Option<AdminProjectRow>> {
        Ok(sqlx::query_as::<_, AdminProjectRow>(&format!("SELECT {PROJECT_COLUMNS} FROM projects p WHERE p.id = $1"))
            .bind(project_id)
            .fetch_optional(self.pool())
            .await?)
    }

    /// A new environment of the project with a random id; `None` when the project does not
    /// exist or already has an environment with this name.
    pub async fn create_environment(&self, project_id: &str, name: &str) -> Result<Option<EnvironmentRow>> {
        let id = format!("e-{}", Uuid::new_v4().simple());
        let r = sqlx::query_as::<_, EnvironmentRow>(
            "INSERT INTO environments (id, tenant_id, project_id, name, is_demo)
             SELECT $1, p.tenant_id, p.id, $3, FALSE FROM projects p WHERE p.id = $2
             RETURNING id, project_id, tenant_id, name, is_demo",
        )
        .bind(&id)
        .bind(project_id)
        .bind(name)
        .fetch_optional(self.pool())
        .await;
        match r {
            Ok(row) => Ok(row),
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Projects whose name starts with `prefix` (test clean-up).
    pub async fn project_ids_named_with_prefix(&self, prefix: &str) -> Result<Vec<String>> {
        if prefix.trim().is_empty() {
            return Err(StorageError::Other("an empty prefix would match every project".into()));
        }
        let pattern = format!("{}%", prefix.replace('\\', r"\\").replace('%', r"\%").replace('_', r"\_"));
        Ok(sqlx::query_scalar("SELECT id FROM projects WHERE name LIKE $1 AND id <> 'demo'")
            .bind(pattern)
            .fetch_all(self.pool())
            .await?)
    }

    /// Deletes the project with its environments, members, roles, invites, tokens and plans
    /// (see `karbird_delete_project`). `false` when it does not exist.
    pub async fn delete_project(&self, project_id: &str) -> Result<bool> {
        Ok(sqlx::query_scalar("SELECT karbird_delete_project($1)").bind(project_id).fetch_one(self.pool()).await?)
    }

    // ------------------------------------------------------------ users

    pub async fn admin_users(&self) -> Result<Vec<AdminUserRow>> {
        Ok(sqlx::query_as::<_, AdminUserRow>(&format!(
            "SELECT {USER_COLUMNS} FROM users u ORDER BY lower(u.email) LIMIT 1000"
        ))
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn admin_user(&self, user_id: Uuid) -> Result<Option<AdminUserRow>> {
        Ok(sqlx::query_as::<_, AdminUserRow>(&format!("SELECT {USER_COLUMNS} FROM users u WHERE u.id = $1"))
            .bind(user_id)
            .fetch_optional(self.pool())
            .await?)
    }

    /// Disables (ending every session) or re-enables an account. `LastOwner` when it would
    /// leave no active Super Admin; `NotFound` when nothing changed.
    pub async fn set_user_disabled(&self, user_id: Uuid, disabled: bool) -> Result<MemberChange> {
        let mut tx = self.pool().begin().await?;
        let before = Self::lock_system(&mut tx).await?;
        let r = if disabled {
            sqlx::query("UPDATE users SET disabled_at = now() WHERE id = $1 AND disabled_at IS NULL")
        } else {
            sqlx::query("UPDATE users SET disabled_at = NULL WHERE id = $1 AND disabled_at IS NOT NULL")
        }
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        if disabled && r.rows_affected() == 1 {
            sqlx::query("UPDATE sessions SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL")
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
        }
        Self::finish_system_change(tx, before, r.rows_affected()).await
    }

    /// Ends every active session of a user. Returns how many were ended.
    pub async fn revoke_all_sessions(&self, user_id: Uuid) -> Result<u64> {
        let r = sqlx::query("UPDATE sessions SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL")
            .bind(user_id)
            .execute(self.pool())
            .await?;
        Ok(r.rows_affected())
    }

    // ------------------------------------------------------------ overview and audit

    pub async fn admin_counts(&self) -> Result<AdminCounts> {
        Ok(sqlx::query_as::<_, AdminCounts>(
            "SELECT (SELECT count(*) FROM projects) AS projects,
                    (SELECT count(*) FROM environments) AS environments,
                    (SELECT count(*) FROM users) AS users,
                    (SELECT count(*) FROM users WHERE disabled_at IS NOT NULL) AS disabled_users,
                    (SELECT count(*) FROM sessions WHERE revoked_at IS NULL AND expires_at > now()) AS active_sessions,
                    (SELECT count(*) FROM system_memberships) AS administrators",
        )
        .fetch_one(self.pool())
        .await?)
    }

    /// Newest audit entries of the whole installation, optionally of one project.
    pub async fn admin_audit(&self, project_id: Option<&str>, limit: i64) -> Result<Vec<AdminAuditRow>> {
        Ok(sqlx::query_as::<_, AdminAuditRow>(
            "SELECT a.id, a.occurred_at, a.tenant_id, a.project_id, a.actor_id, u.email AS actor_email, a.actor_kind,
                    a.action, a.target, a.reason, a.outcome
             FROM audit_log a LEFT JOIN users u ON a.actor_kind = 'user' AND u.id::text = a.actor_id
             WHERE $1::text IS NULL OR a.project_id = $1
             ORDER BY a.occurred_at DESC, a.id DESC LIMIT $2",
        )
        .bind(project_id)
        .bind(limit.clamp(1, 500))
        .fetch_all(self.pool())
        .await?)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[tokio::test]
    async fn an_empty_system_role_prefix_is_refused_before_touching_the_database() {
        let pg = PgStore::connect_lazy("postgres://u:p@127.0.0.1:1/x", 1).unwrap();
        for p in ["", "  "] {
            let e = pg.delete_system_roles_with_prefix(p).await.unwrap_err();
            assert!(e.to_string().contains("empty prefix"), "{e}");
            let e = pg.project_ids_named_with_prefix(p).await.unwrap_err();
            assert!(e.to_string().contains("empty prefix"), "{e}");
        }
    }

    #[test]
    fn super_admin_is_recognised_only_by_the_builtin_key() {
        let g = |key: Option<&str>, name: &str| SystemGrantRow {
            role_key: key.map(str::to_owned),
            role_name: name.into(),
            role_id: None,
            permissions: vec![],
        };
        assert!(g(Some("super_admin"), "Super Admin").is_super_admin());
        assert!(!g(Some("admin"), "Super Admin").is_super_admin(), "a custom name never makes a Super Admin");
        assert!(!g(None, "super_admin").is_super_admin());
    }

    #[test]
    fn the_migration_seeds_the_builtin_system_roles_and_locks_down_project_deletion() {
        let sql = include_str!("../../../infra/migrations/postgres/0006_system_roles.sql");
        for key in ["'super_admin'", "'admin'", "'auditor'"] {
            assert!(sql.contains(key), "{key}");
        }
        assert!(sql.contains("SECURITY DEFINER SET search_path"), "definer function pins its search_path");
        assert!(sql.contains("REVOKE ALL ON FUNCTION karbird_delete_project(TEXT) FROM PUBLIC"));
        assert!(sql.contains("ON DELETE CASCADE"), "deleting a user removes their admin grant");
    }
}
