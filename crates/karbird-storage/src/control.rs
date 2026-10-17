//! Persistence for one-container Karbird Control installations and one-shot commands.

use karbird_domain::Scope;
use serde_json::Value;
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::Result;
use crate::pg::PgStore;

#[derive(Debug, Clone, FromRow)]
pub struct ControlInstallation {
    pub id: Uuid,
    pub tenant_id: String,
    pub project_id: String,
    pub environment_id: String,
    pub name: String,
    pub container_id: Option<String>,
    pub container_name: Option<String>,
    pub reported_container_id: Option<String>,
    pub reported_container_name: Option<String>,
    pub reported_state: Option<String>,
    pub confirmed_at: Option<OffsetDateTime>,
    pub last_seen_at: Option<OffsetDateTime>,
    pub revoked_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Clone, FromRow)]
pub struct ControlCommand {
    pub id: Uuid,
    pub plan_id: Uuid,
    pub installation_id: Uuid,
    pub container_id: String,
    pub verb: String,
    pub state: String,
    pub expires_at: OffsetDateTime,
    pub result: Option<Value>,
}

const INSTALLATION_COLUMNS: &str = "id, tenant_id, project_id, environment_id, name, container_id, container_name, reported_container_id, reported_container_name, reported_state, confirmed_at, last_seen_at, revoked_at, created_at";
const COMMAND_COLUMNS: &str = "id, plan_id, installation_id, container_id, verb, state, expires_at, result";

impl PgStore {
    pub async fn create_control_installation(
        &self,
        scope: &Scope,
        name: &str,
        enrollment_hash: &[u8],
    ) -> Result<ControlInstallation> {
        let id = Uuid::now_v7();
        let query = format!(
            "INSERT INTO control_installations (id, tenant_id, project_id, environment_id, name, enrollment_hash, enrollment_expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, now() + interval '15 minutes') RETURNING {INSTALLATION_COLUMNS}"
        );
        Ok(sqlx::query_as(&query)
            .bind(id)
            .bind(scope.tenant_id.as_str())
            .bind(scope.project_id.as_str())
            .bind(scope.environment_id.as_str())
            .bind(name)
            .bind(enrollment_hash)
            .fetch_one(self.pool())
            .await?)
    }

    pub async fn list_control_installations(&self, scope: &Scope) -> Result<Vec<ControlInstallation>> {
        let query = format!(
            "SELECT {INSTALLATION_COLUMNS} FROM control_installations
             WHERE tenant_id = $1 AND project_id = $2 AND environment_id = $3 ORDER BY created_at DESC"
        );
        Ok(sqlx::query_as(&query)
            .bind(scope.tenant_id.as_str())
            .bind(scope.project_id.as_str())
            .bind(scope.environment_id.as_str())
            .fetch_all(self.pool())
            .await?)
    }

    pub async fn control_installation(&self, scope: &Scope, id: Uuid) -> Result<Option<ControlInstallation>> {
        let query = format!(
            "SELECT {INSTALLATION_COLUMNS} FROM control_installations
             WHERE id = $1 AND tenant_id = $2 AND project_id = $3 AND environment_id = $4"
        );
        Ok(sqlx::query_as(&query)
            .bind(id)
            .bind(scope.tenant_id.as_str())
            .bind(scope.project_id.as_str())
            .bind(scope.environment_id.as_str())
            .fetch_optional(self.pool())
            .await?)
    }

    /// The code can be consumed exactly once, even with concurrent enrollment requests.
    pub async fn enroll_control(
        &self,
        code_hash: &[u8],
        credential_hash: &[u8],
    ) -> Result<Option<ControlInstallation>> {
        let query = format!(
            "UPDATE control_installations SET enrollment_hash = NULL, enrollment_expires_at = NULL,
             credential_hash = $2 WHERE enrollment_hash = $1 AND enrollment_expires_at > now()
             AND credential_hash IS NULL AND revoked_at IS NULL RETURNING {INSTALLATION_COLUMNS}"
        );
        Ok(sqlx::query_as(&query).bind(code_hash).bind(credential_hash).fetch_optional(self.pool()).await?)
    }

    pub async fn authenticate_control(&self, credential_hash: &[u8]) -> Result<Option<ControlInstallation>> {
        let query = format!(
            "SELECT {INSTALLATION_COLUMNS} FROM control_installations
             WHERE credential_hash = $1 AND revoked_at IS NULL"
        );
        Ok(sqlx::query_as(&query).bind(credential_hash).fetch_optional(self.pool()).await?)
    }

    pub async fn report_control_container(
        &self,
        installation_id: Uuid,
        container_id: &str,
        container_name: &str,
        state: &str,
    ) -> Result<bool> {
        let changed = sqlx::query(
            "UPDATE control_installations SET reported_container_id = $2, reported_container_name = $3,
             reported_state = $4, last_seen_at = now() WHERE id = $1 AND revoked_at IS NULL
             AND (container_id IS NULL OR container_id = $2)",
        )
        .bind(installation_id)
        .bind(container_id)
        .bind(container_name)
        .bind(state)
        .execute(self.pool())
        .await?;
        Ok(changed.rows_affected() == 1)
    }

    pub async fn confirm_control_container(&self, scope: &Scope, id: Uuid, container_id: &str) -> Result<bool> {
        let changed = sqlx::query(
            "UPDATE control_installations SET container_id = reported_container_id,
             container_name = reported_container_name, confirmed_at = now()
             WHERE id = $1 AND tenant_id = $2 AND project_id = $3 AND environment_id = $4
             AND revoked_at IS NULL AND credential_hash IS NOT NULL
             AND reported_container_id = $5 AND reported_container_id IS NOT NULL",
        )
        .bind(id)
        .bind(scope.tenant_id.as_str())
        .bind(scope.project_id.as_str())
        .bind(scope.environment_id.as_str())
        .bind(container_id)
        .execute(self.pool())
        .await?;
        Ok(changed.rows_affected() == 1)
    }

    pub async fn revoke_control_installation(&self, scope: &Scope, id: Uuid) -> Result<bool> {
        let changed = sqlx::query(
            "UPDATE control_installations SET revoked_at = now(), credential_hash = NULL,
             enrollment_hash = NULL, enrollment_expires_at = NULL
             WHERE id = $1 AND tenant_id = $2 AND project_id = $3 AND environment_id = $4 AND revoked_at IS NULL",
        )
        .bind(id)
        .bind(scope.tenant_id.as_str())
        .bind(scope.project_id.as_str())
        .bind(scope.environment_id.as_str())
        .execute(self.pool())
        .await?;
        Ok(changed.rows_affected() == 1)
    }

    /// Unique plan_id prevents the executor from creating a second operation after a restart.
    pub async fn create_control_command(
        &self,
        plan_id: Uuid,
        installation_id: Uuid,
        container_id: &str,
        verb: &str,
    ) -> Result<Option<ControlCommand>> {
        let id = Uuid::now_v7();
        let query = format!(
            "INSERT INTO control_commands (id, plan_id, installation_id, container_id, verb, expires_at)
             SELECT $1, $2, i.id, i.container_id, $5, now() + interval '2 minutes'
             FROM control_installations i WHERE i.id = $3 AND i.container_id = $4
             AND i.confirmed_at IS NOT NULL AND i.revoked_at IS NULL
             ON CONFLICT (plan_id) DO NOTHING RETURNING {COMMAND_COLUMNS}"
        );
        Ok(sqlx::query_as(&query)
            .bind(id)
            .bind(plan_id)
            .bind(installation_id)
            .bind(container_id)
            .bind(verb)
            .fetch_optional(self.pool())
            .await?)
    }

    /// Delivered commands can be re-sent; the local journal makes execution idempotent.
    pub async fn poll_control_command(&self, installation_id: Uuid) -> Result<Option<ControlCommand>> {
        let query = format!(
            "UPDATE control_commands SET state = 'delivered', delivered_at = now()
             WHERE id = (SELECT c.id FROM control_commands c JOIN control_installations i ON i.id = c.installation_id
                         WHERE c.installation_id = $1 AND c.state IN ('pending', 'delivered')
                         AND c.expires_at > now() AND i.revoked_at IS NULL
                         AND i.confirmed_at IS NOT NULL AND i.container_id = c.container_id
                         ORDER BY c.created_at FOR UPDATE OF c SKIP LOCKED LIMIT 1)
             RETURNING {COMMAND_COLUMNS}"
        );
        Ok(sqlx::query_as(&query).bind(installation_id).fetch_optional(self.pool()).await?)
    }

    pub async fn complete_control_command(
        &self,
        installation_id: Uuid,
        command_id: Uuid,
        result: &Value,
    ) -> Result<bool> {
        let changed = sqlx::query(
            "UPDATE control_commands SET state = 'completed', completed_at = now(), result = $3
             WHERE id = $1 AND installation_id = $2 AND state = 'delivered' AND expires_at > now()",
        )
        .bind(command_id)
        .bind(installation_id)
        .bind(result)
        .execute(self.pool())
        .await?;
        Ok(changed.rows_affected() == 1)
    }

    pub async fn control_command_for_plan(&self, plan_id: Uuid) -> Result<Option<ControlCommand>> {
        let query = format!("SELECT {COMMAND_COLUMNS} FROM control_commands WHERE plan_id = $1");
        Ok(sqlx::query_as(&query).bind(plan_id).fetch_optional(self.pool()).await?)
    }
}
