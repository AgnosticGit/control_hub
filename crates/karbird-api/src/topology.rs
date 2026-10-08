//! Server-owned graph snapshots. Reads are fast; stale saved maps refresh behind a fenced lease.
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use karbird_domain::Scope;
use karbird_engine_core::{SavedTopologyGeometry, TopologySnapshot, build_server_geometry};
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use crate::error::{ApiError, ApiResult};
use crate::routes::infrastructure::load_snapshot;
use crate::routes::telemetry::{Range, TopologyResponse, aggregate_topology, now_ns};
use crate::state::AppState;

#[derive(Clone, Serialize, Deserialize)]
struct Document {
    fingerprint: String,
    response: TopologyResponse,
    /// First backend discovery of an identity, distinct from telemetry's last_seen_ns.
    #[serde(default)]
    first_observed_ns: BTreeMap<String, String>,
}

fn decode_document(value: serde_json::Value) -> Option<Document> {
    let document = serde_json::from_value::<Document>(value).ok()?;
    let graph: TopologySnapshot =
        serde_json::from_value(serde_json::json!({"nodes":document.response.nodes,"edges":document.response.edges}))
            .ok()?;
    document.response.geometry.validate(&graph).ok()?;
    Some(document)
}

async fn baseline(state: &AppState, scope: &Scope, key: &str) -> ApiResult<Option<Document>> {
    Ok(state.pg.peek_topology(scope, key).await?.and_then(|r| r.document).and_then(decode_document))
}

/// Placement variants are keyed by membership, not by time/CPU. Replaying the same placement
/// reuses its saved geometry; current point metrics are carried separately and never cached here.
pub(crate) async fn with_placement(
    state: &AppState,
    scope: &Scope,
    mut response: TopologyResponse,
    snapshot: karbird_engine_core::InfrastructureSnapshot,
) -> ApiResult<TopologyResponse> {
    let graph: TopologySnapshot =
        serde_json::from_value(serde_json::json!({"nodes":response.nodes,"edges":response.edges}))
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    let fingerprint = structural_fingerprint(&graph, &snapshot)?;
    let key = placement_key(&fingerprint, &response.geometry.plain)?;
    if let Some(saved) = state.pg.saved_topology(scope, &key).await?.and_then(|r| r.document).and_then(decode_document)
    {
        response.geometry = saved.response.geometry;
        response.infrastructure_snapshot = Some(snapshot);
        return Ok(response);
    }
    let _permit = tokio::time::timeout(Duration::from_secs(10), slots().acquire_owned())
        .await
        .map_err(|_| ApiError::Unavailable("placement builder busy"))?
        .map_err(|_| ApiError::Unavailable("placement builder stopped"))?;
    if let Some(token) = state.pg.lease_topology(scope, &key).await? {
        let result: ApiResult<SavedTopologyGeometry> = async {
            if let Some(saved) = baseline(state, scope, &key).await? {
                return Ok(saved.response.geometry);
            }
            let previous = response.geometry.clone();
            let input = snapshot.clone();
            let geometry =
                tokio::time::timeout(Duration::from_secs(20), compute_geometry(graph, input, Some(previous)))
                    .await
                    .map_err(|_| ApiError::Unavailable("placement builder timed out"))??;
            let mut stored = response.clone();
            stored.geometry = geometry.clone();
            stored.infrastructure_snapshot = None;
            let document = Document { fingerprint, response: stored, first_observed_ns: BTreeMap::new() };
            let value = serde_json::to_value(document).map_err(|e| ApiError::Internal(e.to_string()))?;
            if !state.pg.publish_topology(scope, &key, token, &value).await? {
                return Err(ApiError::Unavailable("placement lease expired"));
            }
            Ok(geometry)
        }
        .await;
        state.pg.release_topology(scope, &key, token).await?;
        response.geometry = result?;
    } else {
        let mut geometry = None;
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if let Some(saved) = baseline(state, scope, &key).await? {
                geometry = Some(saved.response.geometry);
                break;
            }
        }
        response.geometry = geometry.ok_or(ApiError::Unavailable("placement is being prepared"))?;
    }
    response.infrastructure_snapshot = Some(snapshot);
    Ok(response)
}

fn placement_key(fingerprint: &str, plain: &karbird_engine_core::TopologyView) -> ApiResult<String> {
    use sha2::{Digest, Sha256};
    // A live infrastructure revision can change while these coordinates and the
    // selected point's membership stay identical. It must not invalidate placement.
    let mut coordinates = plain.clone();
    coordinates.revision = 0;
    let identity = serde_json::to_vec(&(fingerprint, coordinates)).map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(format!("placement:{:x}", Sha256::digest(identity)))
}

fn slots() -> Arc<Semaphore> {
    static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    SLOTS.get_or_init(|| Arc::new(Semaphore::new(2))).clone()
}

async fn compute_geometry(
    graph: TopologySnapshot,
    infrastructure: karbird_engine_core::InfrastructureSnapshot,
    previous: Option<SavedTopologyGeometry>,
) -> ApiResult<SavedTopologyGeometry> {
    // Blocking jobs survive cancellation. Keep their shared CPU permit until they finish.
    static GEOMETRY_JOBS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    let permit = GEOMETRY_JOBS
        .get_or_init(|| Arc::new(Semaphore::new(2)))
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| ApiError::Unavailable("geometry builder stopped"))?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        build_server_geometry(&graph, Some(&infrastructure), previous.as_ref())
    })
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    .map_err(|e| ApiError::Internal(e.to_string()))
}

pub(crate) async fn read(state: &AppState, scope: &Scope, range: Range, live: bool) -> ApiResult<TopologyResponse> {
    let key = if live { "live".to_owned() } else { format!("{}:{}", range.from, range.to) };
    let row = state.pg.saved_topology(scope, &key).await?;
    if live && row.is_some() {
        start_watch(state, scope).await?;
    }
    let previous = row.as_ref().and_then(|r| r.document.clone()).and_then(decode_document);
    let fresh = row
        .as_ref()
        .and_then(|r| r.refreshed_at)
        .is_some_and(|at| (time::OffsetDateTime::now_utc() - at).whole_seconds() < 10);
    if let Some(saved) = &previous {
        if !fresh
            && let Ok(permit) = slots().try_acquire_owned()
            && let Some(token) = state.pg.lease_topology(scope, &key).await?
        {
            let state = state.clone();
            let scope = scope.clone();
            let key = key.clone();
            tokio::spawn(async move {
                let _permit = permit;
                let result = tokio::time::timeout(Duration::from_secs(45), async {
                    let previous = baseline(&state, &scope, &key).await?;
                    rebuild(&state, &scope, range, live, previous).await
                })
                .await;
                match result {
                    Ok(Ok(document)) => match serde_json::to_value(document) {
                        Ok(value) => {
                            if let Err(error) = state.pg.publish_topology(&scope, &key, token, &value).await {
                                tracing::error!(%error, "topology publish failed");
                            }
                        }
                        Err(error) => tracing::error!(%error, "topology serialization failed"),
                    },
                    Ok(Err(error)) => tracing::error!(%error, "topology refresh failed; retained saved map"),
                    Err(_) => tracing::error!("topology refresh timed out; retained saved map"),
                }
                if let Err(error) = state.pg.release_topology(&scope, &key, token).await {
                    tracing::error!(%error, "topology lease release failed");
                }
            });
        }
        return Ok(saved.response.clone());
    }
    let _permit = tokio::time::timeout(Duration::from_secs(10), slots().acquire_owned())
        .await
        .map_err(|_| ApiError::Unavailable("map builder busy"))?
        .map_err(|_| ApiError::Unavailable("map builder stopped"))?;
    if let Some(token) = state.pg.lease_topology(scope, &key).await? {
        // A queued cold reader may acquire the lease after a different instance published.
        // Check under the fenced lease so it never replaces that snapshot with revision 1.
        if let Some(saved) = baseline(state, scope, &key).await? {
            state.pg.release_topology(scope, &key, token).await?;
            return Ok(saved.response);
        }
        let result = tokio::time::timeout(Duration::from_secs(45), rebuild(state, scope, range, live, None)).await;
        let document = match result {
            Ok(Ok(document)) => document,
            other => {
                state.pg.release_topology(scope, &key, token).await?;
                return match other {
                    Ok(Err(error)) => Err(error),
                    _ => Err(ApiError::Unavailable("map builder timed out")),
                };
            }
        };
        let value = serde_json::to_value(&document).map_err(|e| ApiError::Internal(e.to_string()))?;
        if !state.pg.publish_topology(scope, &key, token, &value).await? {
            return Err(ApiError::Unavailable("map refresh lease expired"));
        }
        if live {
            start_watch(state, scope).await?;
        }
        return Ok(document.response);
    }
    // Another API instance owns the first build. Never duplicate its aggregation/layout work.
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if let Some(document) =
            state.pg.saved_topology(scope, &key).await?.and_then(|r| r.document).and_then(decode_document)
        {
            return Ok(document.response);
        }
    }
    Err(ApiError::Unavailable("map is being prepared"))
}

async fn start_watch(state: &AppState, scope: &Scope) -> ApiResult<()> {
    static WATCHES: OnceLock<Arc<Semaphore>> = OnceLock::new();
    let Ok(permit) = WATCHES.get_or_init(|| Arc::new(Semaphore::new(64))).clone().try_acquire_owned() else {
        return Ok(());
    };
    let owner = uuid::Uuid::new_v4();
    if !state.pg.watch_topology(scope, owner, false).await? {
        return Ok(());
    }
    let state = state.clone();
    let scope = scope.clone();
    tokio::spawn(async move {
        let _watch_permit = permit;
        loop {
            tokio::time::sleep(Duration::from_secs(10)).await;
            let result: ApiResult<bool> = async {
                let Some(row) = state.pg.peek_topology(&scope, "live").await? else {
                    return Ok(false);
                };
                if (time::OffsetDateTime::now_utc() - row.accessed_at).whole_seconds() > 120 {
                    return Ok(false);
                }
                if !state.pg.watch_topology(&scope, owner, true).await? {
                    return Ok(false);
                }
                let Ok(_permit) = slots().try_acquire_owned() else {
                    return Ok(true);
                };
                let Some(token) = state.pg.lease_topology(&scope, "live").await? else {
                    return Ok(true);
                };
                let previous = baseline(&state, &scope, "live").await?;
                let to = now_ns();
                let result = tokio::time::timeout(
                    Duration::from_secs(20),
                    rebuild(&state, &scope, Range { from: to - 900_000_000_000, to }, true, previous),
                )
                .await;
                match result {
                    Ok(Ok(document)) => {
                        let value = serde_json::to_value(document).map_err(|e| ApiError::Internal(e.to_string()))?;
                        state.pg.publish_topology(&scope, "live", token, &value).await?;
                    }
                    Ok(Err(error)) => tracing::error!(%error, "active map refresh failed"),
                    Err(_) => tracing::error!("active map refresh timed out"),
                }
                state.pg.release_topology(&scope, "live", token).await?;
                Ok(true)
            }
            .await;
            match result {
                Ok(true) => {}
                Ok(false) => break,
                Err(error) => {
                    tracing::error!(%error,"map watch stopped");
                    break;
                }
            }
        }
    });
    Ok(())
}

async fn rebuild(
    state: &AppState,
    scope: &Scope,
    range: Range,
    live: bool,
    previous: Option<Document>,
) -> ApiResult<Document> {
    let range = if live {
        let to = now_ns();
        Range { from: to - 900_000_000_000, to }
    } else {
        range
    };
    let (mut nodes, mut edges, mut completeness) = aggregate_topology(state, scope, range).await?;
    if live && let Some(old) = &previous {
        for node in &old.response.nodes {
            if !nodes.iter().any(|n| n.id == node.id) {
                nodes.push(node.clone());
            }
        }
        for edge in &old.response.edges {
            if !edges.iter().any(|e| e.source == edge.source && e.target == edge.target && e.relation == edge.relation)
            {
                let mut inactive = edge.clone();
                inactive.calls = 0;
                inactive.errors = 0;
                edges.push(inactive);
            }
        }
    }
    // Stable index order and explicit bounds prevent unbounded inventory growth from telemetry.
    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    if nodes.len() > 2000 || edges.len() > 10000 {
        completeness.truncated = true;
        completeness.notes.push("Saved topology reached its 2000-node / 10000-edge safety limit".into());
        tracing::warn!(nodes = nodes.len(), edges = edges.len(), "topology safety limit reached");
    }
    nodes.truncate(2000);
    edges.retain(|e| nodes.iter().any(|n| n.id == e.source) && nodes.iter().any(|n| n.id == e.target));
    edges.sort_by(|a, b| {
        (&a.source, &a.target, format!("{:?}", a.relation)).cmp(&(&b.source, &b.target, format!("{:?}", b.relation)))
    });
    edges.truncate(10000);
    let snapshot: TopologySnapshot = serde_json::from_value(serde_json::json!({"nodes":nodes,"edges":edges}))
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let infrastructure = load_snapshot(state, scope, range.to).await?;
    let fingerprint = structural_fingerprint(&snapshot, &infrastructure)?;
    let geometry = if let Some(old) = &previous
        && old.fingerprint == fingerprint
    {
        old.response.geometry.clone()
    } else {
        let previous: Option<SavedTopologyGeometry> = previous.as_ref().map(|d| d.response.geometry.clone());
        compute_geometry(snapshot, infrastructure, previous).await?
    };
    let discovered_at = now_ns().to_string();
    let identities = nodes
        .iter()
        .map(|n| format!("node:{}", n.id))
        .chain(edges.iter().map(|e| format!("edge:{}:{}:{:?}", e.source, e.target, e.relation)));
    let first_observed_ns = identities
        .map(|id| {
            let at = previous
                .as_ref()
                .and_then(|p| p.first_observed_ns.get(&id))
                .cloned()
                .unwrap_or_else(|| discovered_at.clone());
            (id, at)
        })
        .collect();
    Ok(Document {
        fingerprint,
        first_observed_ns,
        response: TopologyResponse {
            nodes,
            edges,
            from_ns: range.from.to_string(),
            to_ns: range.to.to_string(),
            completeness,
            geometry,
            updated_at_ns: now_ns().to_string(),
            infrastructure_snapshot: None,
        },
    })
}

fn structural_fingerprint(
    graph: &TopologySnapshot,
    infra: &karbird_engine_core::InfrastructureSnapshot,
) -> ApiResult<String> {
    use sha2::{Digest, Sha256};
    let hosts: BTreeMap<_, _> = infra.hosts.iter().map(|h| (&h.id, &h.name)).collect();
    let instances: BTreeMap<_, _> =
        infra.instances.iter().map(|i| (&i.instance_id, (&i.service_id, &i.host_id, &i.container_id))).collect();
    let resources: BTreeMap<_, _> =
        infra.resources.iter().map(|r| (&r.node_id, (&r.location, &r.host_id, &r.container_id))).collect();
    let bytes = serde_json::to_vec(&(graph, hosts, instances, resources, karbird_engine_core::SERVER_LAYOUT_VERSION))
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use karbird_engine_core::{HostDto, InfrastructureSnapshot, InstanceDto};

    #[test]
    fn selected_time_placement_ignores_live_revision_changes_without_new_coordinates() {
        let graph: TopologySnapshot = serde_json::from_str(
            r#"{"nodes":[{"id":"svc","name":"service","kind":"service","provenance":"observed"}],"edges":[]}"#,
        )
        .unwrap();
        let mut plain = build_server_geometry(&graph, None, None).unwrap().plain;
        let key = placement_key("fixed-point-membership", &plain).unwrap();
        plain.revision += 1;
        assert_eq!(placement_key("fixed-point-membership", &plain).unwrap(), key);
        plain.nodes[0].x += 300.0;
        assert_ne!(placement_key("fixed-point-membership", &plain).unwrap(), key);
    }

    #[test]
    fn metrics_do_not_change_structure_but_replica_membership_does() {
        let graph = TopologySnapshot::default();
        let mut infrastructure = InfrastructureSnapshot {
            hosts: vec![HostDto {
                id: "host".into(),
                name: "machine".into(),
                cpu_utilization: Some(0.1),
                ..HostDto::default()
            }],
            ..InfrastructureSnapshot::default()
        };
        let first = structural_fingerprint(&graph, &infrastructure).unwrap();
        infrastructure.at_ns = 100;
        infrastructure.hosts[0].cpu_utilization = Some(0.99);
        infrastructure.hosts[0].memory_used_bytes = Some(999.0);
        assert_eq!(structural_fingerprint(&graph, &infrastructure).unwrap(), first);
        infrastructure.instances.push(InstanceDto {
            instance_id: "replica".into(),
            service_id: "service".into(),
            host_id: Some("host".into()),
            ..InstanceDto::default()
        });
        assert_ne!(structural_fingerprint(&graph, &infrastructure).unwrap(), first);
    }

    #[test]
    fn machine_order_does_not_change_structure() {
        let graph = TopologySnapshot::default();
        let mut infrastructure = InfrastructureSnapshot {
            hosts: vec![
                HostDto { id: "a".into(), ..HostDto::default() },
                HostDto { id: "b".into(), ..HostDto::default() },
            ],
            ..InfrastructureSnapshot::default()
        };
        let first = structural_fingerprint(&graph, &infrastructure).unwrap();
        infrastructure.hosts.reverse();
        assert_eq!(structural_fingerprint(&graph, &infrastructure).unwrap(), first);
    }

    #[test]
    fn saved_geometry_from_an_older_layout_is_discarded_and_rebuilt() {
        let graph = TopologySnapshot::default();
        let mut geometry = build_server_geometry(&graph, None, None).unwrap();
        geometry.version = karbird_engine_core::SERVER_LAYOUT_VERSION - 1;
        let document = serde_json::json!({
            "fingerprint": "same-graph",
            "response": {
                "nodes": [],
                "edges": [],
                "from_ns": "0",
                "to_ns": "1",
                "completeness": {
                    "watermark_ns": null,
                    "processing_caught_up": true,
                    "unresolved_processing_errors": 0,
                    "truncated": false,
                    "sampling": "unknown",
                    "notes": []
                },
                "geometry": geometry,
                "updated_at_ns": "1"
            }
        });

        assert!(decode_document(document).is_none(), "legacy geometry must trigger a fresh layout build");
    }
}
