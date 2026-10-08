//! Durable map snapshots and fenced, scope-bound refresh leases.
use crate::{Result, pg::PgStore};
use karbird_domain::Scope;
use serde_json::Value;
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, FromRow)]
pub struct SavedTopologyRow {
    pub document: Option<Value>,
    pub refreshed_at: Option<OffsetDateTime>,
    pub accessed_at: OffsetDateTime,
}

impl PgStore {
    pub async fn saved_topology(&self, scope: &Scope, key: &str) -> Result<Option<SavedTopologyRow>> {
        Ok(sqlx::query_as("UPDATE topology_snapshots SET accessed_at=now() WHERE tenant_id=$1 AND project_id=$2 AND environment_id=$3 AND cache_key=$4 RETURNING document, refreshed_at, accessed_at")
            .bind(scope.tenant_id.as_str()).bind(scope.project_id.as_str()).bind(scope.environment_id.as_str()).bind(key)
            .fetch_optional(self.pool()).await?)
    }

    pub async fn peek_topology(&self, scope: &Scope, key: &str) -> Result<Option<SavedTopologyRow>> {
        Ok(sqlx::query_as("SELECT document,refreshed_at,accessed_at FROM topology_snapshots WHERE tenant_id=$1 AND project_id=$2 AND environment_id=$3 AND cache_key=$4")
            .bind(scope.tenant_id.as_str()).bind(scope.project_id.as_str()).bind(scope.environment_id.as_str()).bind(key).fetch_optional(self.pool()).await?)
    }

    pub async fn watch_topology(&self, scope: &Scope, token: Uuid, renew: bool) -> Result<bool> {
        let result = sqlx::query("UPDATE topology_snapshots SET watch_token=$4,watch_until=now()+interval '60 seconds' WHERE tenant_id=$1 AND project_id=$2 AND environment_id=$3 AND cache_key='live' AND (($5 AND watch_token=$4) OR (NOT $5 AND (watch_until IS NULL OR watch_until < now())))")
            .bind(scope.tenant_id.as_str()).bind(scope.project_id.as_str()).bind(scope.environment_id.as_str()).bind(token).bind(renew).execute(self.pool()).await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn lease_topology(&self, scope: &Scope, key: &str) -> Result<Option<Uuid>> {
        let token = Uuid::new_v4();
        let row: Option<(Uuid,)> = sqlx::query_as("INSERT INTO topology_snapshots (tenant_id,project_id,environment_id,cache_key,lease_token,lease_until) VALUES ($1,$2,$3,$4,$5,now()+interval '60 seconds') ON CONFLICT (tenant_id,project_id,environment_id,cache_key) DO UPDATE SET lease_token=$5, lease_until=now()+interval '60 seconds' WHERE topology_snapshots.lease_until IS NULL OR topology_snapshots.lease_until < now() RETURNING lease_token")
            .bind(scope.tenant_id.as_str()).bind(scope.project_id.as_str()).bind(scope.environment_id.as_str()).bind(key).bind(token)
            .fetch_optional(self.pool()).await?;
        Ok(row.map(|r| r.0))
    }

    pub async fn publish_topology(&self, scope: &Scope, key: &str, token: Uuid, document: &Value) -> Result<bool> {
        let result = sqlx::query("UPDATE topology_snapshots SET document=$6, refreshed_at=now(),lease_token=NULL,lease_until=NULL WHERE tenant_id=$1 AND project_id=$2 AND environment_id=$3 AND cache_key=$4 AND lease_token=$5 AND lease_until > now()")
            .bind(scope.tenant_id.as_str()).bind(scope.project_id.as_str()).bind(scope.environment_id.as_str()).bind(key).bind(token).bind(document)
            .execute(self.pool()).await?;
        // At most 64 history/placement entries per environment, plus its live inventory.
        sqlx::query("DELETE FROM topology_snapshots WHERE tenant_id=$1 AND project_id=$2 AND environment_id=$3 AND cache_key IN (SELECT cache_key FROM topology_snapshots WHERE tenant_id=$1 AND project_id=$2 AND environment_id=$3 AND cache_key <> 'live' AND (lease_until IS NULL OR lease_until < now()) ORDER BY accessed_at DESC OFFSET 64)")
            .bind(scope.tenant_id.as_str()).bind(scope.project_id.as_str()).bind(scope.environment_id.as_str()).execute(self.pool()).await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn release_topology(&self, scope: &Scope, key: &str, token: Uuid) -> Result<()> {
        sqlx::query("UPDATE topology_snapshots SET lease_token=NULL,lease_until=NULL WHERE tenant_id=$1 AND project_id=$2 AND environment_id=$3 AND cache_key=$4 AND lease_token=$5")
            .bind(scope.tenant_id.as_str()).bind(scope.project_id.as_str()).bind(scope.environment_id.as_str()).bind(key).bind(token).execute(self.pool()).await?;
        Ok(())
    }
}
