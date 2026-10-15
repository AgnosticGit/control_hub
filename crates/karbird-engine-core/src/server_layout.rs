//! Persistable geometry computed natively and consumed unchanged by WASM.
use crate::*;

pub const SERVER_LAYOUT_VERSION: u32 = 3;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SavedTopologyGeometry {
    pub version: u32,
    pub revision: u64,
    pub plain: TopologyView,
    pub infrastructure: Option<TopologyView>,
    #[serde(default)]
    pub external: Vec<u32>,
}

impl SavedTopologyGeometry {
    pub fn validate(&self, snapshot: &TopologySnapshot) -> Result<(), EngineError> {
        if self.version != SERVER_LAYOUT_VERSION || self.revision == 0 {
            return Err(EngineError::InvalidInput("unsupported server geometry version/revision".into()));
        }
        for view in std::iter::once(&self.plain).chain(self.infrastructure.iter()) {
            if view.nodes.len() != snapshot.nodes.len() || view.edges.len() != snapshot.edges.len() {
                return Err(EngineError::InvalidInput("server geometry does not cover the graph".into()));
            }
            for (i, (node, source)) in view.nodes.iter().zip(&snapshot.nodes).enumerate() {
                if node.index as usize != i || node.id != source.id || !node.x.is_finite() || !node.y.is_finite() {
                    return Err(EngineError::InvalidInput("invalid server node geometry".into()));
                }
            }
            for (i, (edge, source)) in view.edges.iter().zip(&snapshot.edges).enumerate() {
                if edge.index as usize != i
                    || view.nodes.get(edge.source as usize).map(|n| &n.id) != Some(&source.source)
                    || view.nodes.get(edge.target as usize).map(|n| &n.id) != Some(&source.target)
                    || edge.relation != source.relation
                    || edge.points.iter().flatten().any(|v| !v.is_finite())
                {
                    return Err(EngineError::InvalidInput("invalid server edge geometry".into()));
                }
            }
        }
        Ok(())
    }
}

pub fn build_server_geometry(
    snapshot: &TopologySnapshot,
    infrastructure: Option<&InfrastructureSnapshot>,
    previous: Option<&SavedTopologyGeometry>,
) -> Result<SavedTopologyGeometry, EngineError> {
    let mut engine = Engine::new(EngineConfig::default());
    engine.load_topology(snapshot.clone());
    let mut plain = engine.topology();
    // Keep good layouts anchored, but recover from a saved row that formed while telemetry
    // nodes arrived before their edges. Preserving that row would make every later fan-out
    // route collapse onto the same line.
    let preserve_plain = previous.is_some_and(|old| !has_collapsed_branch(&old.plain, snapshot));
    if preserve_plain && let Some(old) = previous {
        let mut positions: std::collections::HashMap<String, (f32, f32)> =
            old.plain.nodes.iter().map(|n| (n.id.clone(), (n.x, n.y))).collect();
        let corridors: Vec<[f32; 2]> = old.plain.edges.iter().flat_map(|e| e.points.iter().copied()).collect();
        for node in &mut plain.nodes {
            if let Some(&(x, y)) = positions.get(&node.id) {
                node.x = x;
                node.y = y;
            } else {
                if let Some((x, y)) = snapshot
                    .edges
                    .iter()
                    .filter(|e| e.target == node.id)
                    .find_map(|e| positions.get(&e.source).copied())
                {
                    node.x = x + 300.0;
                    node.y = y;
                }
                while positions.values().any(|&(x, y)| (x - node.x).abs() < 172.0 && (y - node.y).abs() < 120.0)
                    || corridors.iter().any(|p| (p[0] - node.x).abs() < 172.0 && (p[1] - node.y).abs() < 64.0)
                {
                    node.y += 120.0;
                }
                positions.insert(node.id.clone(), (node.x, node.y));
            }
        }
    }
    route_saved_edges(&mut plain, if preserve_plain { previous.map(|p| &p.plain) } else { None });
    let revision = previous.map_or(1, |old| old.revision.saturating_add(1));
    plain.revision = revision;
    let (infrastructure, external) = if let Some(infrastructure_snapshot) = infrastructure {
        engine.set_infrastructure(infrastructure_snapshot.clone());
        engine.set_filters(Filters { infrastructure: true, ..Filters::default() });
        let mut external: Vec<u32> =
            engine.infra_layout.as_ref().map(|layout| layout.external.iter().copied().collect()).unwrap_or_default();
        external.sort_unstable();
        let mut view = engine.topology();
        let old_infrastructure = previous.and_then(|p| p.infrastructure.as_ref());
        let reusable_infrastructure = old_infrastructure.filter(|old| {
            !has_collapsed_branch(old, snapshot) && infrastructure_membership(&view) == infrastructure_membership(old)
        });
        if let Some(old) = reusable_infrastructure {
            for node in &mut view.nodes {
                if let Some(saved) = old.nodes.iter().find(|n| n.id == node.id) {
                    node.x = saved.x;
                    node.y = saved.y;
                }
            }
            if let (Some(current), Some(prior)) = (&mut view.infra, &old.infra) {
                for frame in &mut current.frames {
                    if let Some(saved) = prior.frames.iter().find(|f| f.host_id == frame.host_id) {
                        frame.x = saved.x;
                        frame.y = saved.y;
                        frame.width = saved.width;
                        frame.height = saved.height;
                    }
                }
                for chip in &mut current.chips {
                    if let Some(saved) =
                        prior.chips.iter().find(|c| c.service_id == chip.service_id && c.host_id == chip.host_id)
                    {
                        chip.x = saved.x;
                        chip.y = saved.y;
                    }
                }
                let node_of = view.nodes.iter().map(|n| (n.id.as_str(), n.index)).collect();
                let positions = view.nodes.iter().map(|n| (n.index, (n.x, n.y))).collect();
                current.links =
                    infra::noisy_links(infrastructure_snapshot, &node_of, &positions, &current.chips, &current.frames);
            }
            route_saved_edges(&mut view, Some(old));
        } else {
            route_saved_edges(&mut view, None);
        }
        view.revision = revision;
        (Some(view), external)
    } else {
        (None, Vec::new())
    };
    let result = SavedTopologyGeometry { version: SERVER_LAYOUT_VERSION, revision, plain, infrastructure, external };
    result.validate(snapshot)?;
    Ok(result)
}

fn infrastructure_membership(view: &TopologyView) -> String {
    let membership: std::collections::BTreeMap<_, _> = view
        .nodes
        .iter()
        .map(|n| {
            let host = view
                .infra
                .as_ref()
                .and_then(|i| {
                    i.frames.iter().find(|f| n.x >= f.x && n.x <= f.x + f.width && n.y >= f.y && n.y <= f.y + f.height)
                })
                .map(|f| f.host_id.as_str());
            (n.id.as_str(), host)
        })
        .collect();
    let chips: std::collections::BTreeMap<_, _> = view
        .infra
        .as_ref()
        .map(|i| i.chips.iter().map(|c| ((c.service_id.as_str(), c.host_id.as_str()), &c.instance_ids)).collect())
        .unwrap_or_default();
    format!("{membership:?}|{chips:?}")
}

/// A layout persisted before its edges arrived may leave every branch on the same horizontal
/// row. Reuse is safe only when each multi-neighbour fan-out/fan-in has visible vertical spread.
fn has_collapsed_branch(view: &TopologyView, graph: &TopologySnapshot) -> bool {
    use std::collections::{BTreeSet, HashMap};

    let positions: HashMap<&str, (f32, f32)> =
        view.nodes.iter().map(|node| (node.id.as_str(), (node.x, node.y))).collect();
    let mut outgoing: HashMap<&str, BTreeSet<&str>> = HashMap::new();
    let mut incoming: HashMap<&str, BTreeSet<&str>> = HashMap::new();
    for edge in &graph.edges {
        outgoing.entry(&edge.source).or_default().insert(&edge.target);
        incoming.entry(&edge.target).or_default().insert(&edge.source);
    }
    let branches: Vec<&BTreeSet<&str>> = outgoing.values().chain(incoming.values()).filter(|n| n.len() > 1).collect();
    if branches.is_empty() {
        return false;
    }
    let min_x = view.nodes.iter().map(|node| node.x).fold(f32::INFINITY, f32::min);
    let max_x = view.nodes.iter().map(|node| node.x).fold(f32::NEG_INFINITY, f32::max);
    if max_x - min_x < crate::layout::COLUMN_SPACING / 2.0 {
        return true;
    }
    branches.into_iter().any(|neighbours| {
        let mut ys = neighbours.iter().filter_map(|id| positions.get(id).map(|point| point.1));
        let Some(first) = ys.next() else { return false };
        let Some(second) = ys.next() else { return false };
        let (min, max) = ys.fold((first.min(second), first.max(second)), |(min, max), y| (min.min(y), max.max(y)));
        max - min < crate::layout::ROW_SPACING / 2.0
    })
}

fn route_saved_edges(view: &mut TopologyView, previous: Option<&TopologyView>) {
    let bottom = view
        .nodes
        .iter()
        .map(|n| n.y)
        .chain(view.infra.iter().flat_map(|i| i.frames.iter().map(|f| f.y + f.height)))
        .fold(0.0_f32, f32::max)
        + 120.0;
    for edge in &mut view.edges {
        let source = &view.nodes[edge.source as usize];
        let target = &view.nodes[edge.target as usize];
        if let Some(old) = previous
            && let Some(saved) = old.edges.iter().find(|e| {
                old.nodes.get(e.source as usize).is_some_and(|n| n.id == source.id)
                    && old.nodes.get(e.target as usize).is_some_and(|n| n.id == target.id)
                    && e.relation == edge.relation
            })
            && (target.x > source.x || !saved.points.is_empty())
        {
            edge.points = saved.points.clone();
            continue;
        }
        if target.x <= source.x {
            let lane = 150.0 + (edge.index % 5) as f32 * 10.0;
            let y = bottom + edge.index as f32 * 22.0;
            edge.points = vec![
                [source.x + lane, source.y],
                [source.x + lane, y],
                [target.x - lane, y],
                [target.x - lane, target.y],
            ];
        } else if previous.is_some() {
            edge.points.clear();
            let mut x = source.x + 300.0;
            while x < target.x {
                edge.points.push([x, bottom + edge.index as f32 * 22.0]);
                x += 300.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use proptest::prelude::*;

    fn graph(ids: &[&str]) -> TopologySnapshot {
        TopologySnapshot {
            nodes: ids
                .iter()
                .map(|id| TopologyNode {
                    id: (*id).into(),
                    name: (*id).into(),
                    kind: EntityKindDto::Service,
                    provenance: ProvenanceDto::Observed,
                })
                .collect(),
            edges: vec![],
        }
    }

    #[test]
    fn additions_preserve_existing_coordinates_and_have_unique_positions() {
        let old = build_server_geometry(&graph(&["a", "b"]), None, None).unwrap();
        let next = build_server_geometry(&graph(&["a", "b", "c"]), None, Some(&old)).unwrap();
        for node in &old.plain.nodes {
            let actual = next.plain.nodes.iter().find(|n| n.id == node.id).unwrap();
            assert_eq!((actual.x, actual.y), (node.x, node.y));
        }
        assert_eq!(next.revision, old.revision + 1);
        assert!(
            next.plain
                .nodes
                .iter()
                .all(|n| n.id == "c" || (n.x, n.y) != (next.plain.nodes[2].x, next.plain.nodes[2].y))
        );
    }

    #[test]
    fn adding_a_second_external_call_preserves_the_first_service_position() {
        let mut initial = graph(&["external", "first"]);
        initial.edges.push(TopologyEdge {
            source: "external".into(),
            target: "first".into(),
            relation: RelationKindDto::Calls,
            provenance: ProvenanceDto::Observed,
            reasons: vec![],
        });
        let old = build_server_geometry(&initial, None, None).unwrap();
        let mut expanded = graph(&["external", "first", "second"]);
        expanded.edges = initial.edges;
        expanded.edges.push(TopologyEdge {
            source: "external".into(),
            target: "second".into(),
            relation: RelationKindDto::Calls,
            provenance: ProvenanceDto::Observed,
            reasons: vec![],
        });

        let next = build_server_geometry(&expanded, None, Some(&old)).unwrap();
        for saved in &old.plain.nodes {
            let actual = next.plain.nodes.iter().find(|node| node.id == saved.id).unwrap();
            assert_eq!((actual.x, actual.y), (saved.x, saved.y), "{} moved", saved.id);
        }
    }

    #[test]
    fn duplicate_node_identity_is_rejected_before_building() {
        assert!(build_server_geometry(&graph(&["a", "a"]), None, None).is_err());
    }

    #[test]
    fn a_new_dependency_uses_its_callers_column_without_moving_existing_cards() {
        let mut initial = graph(&["a", "b"]);
        initial.edges.push(TopologyEdge {
            source: "a".into(),
            target: "b".into(),
            relation: RelationKindDto::Calls,
            provenance: ProvenanceDto::Observed,
            reasons: vec![],
        });
        let old = build_server_geometry(&initial, None, None).unwrap();
        let mut next = graph(&["a", "b", "c"]);
        next.edges = initial.edges;
        next.edges.push(TopologyEdge {
            source: "a".into(),
            target: "c".into(),
            relation: RelationKindDto::Calls,
            provenance: ProvenanceDto::Observed,
            reasons: vec![],
        });
        let saved = build_server_geometry(&next, None, Some(&old)).unwrap();
        let caller = old.plain.nodes.iter().find(|n| n.id == "a").unwrap();
        let new = saved.plain.nodes.iter().find(|n| n.id == "c").unwrap();
        assert_eq!(new.x, caller.x + 300.0);
    }

    #[test]
    fn later_edges_reflow_a_saved_single_row_instead_of_overlapping() {
        let mut snapshot = graph(&["auth", "catalog", "external", "gateway", "orders"]);
        let mut horizontal = build_server_geometry(&snapshot, None, None).unwrap();
        let mut vertical = horizontal.clone();
        for (index, node) in horizontal.plain.nodes.iter_mut().enumerate() {
            node.x = index as f32 * 300.0;
            node.y = 0.0;
        }
        for (index, node) in vertical.plain.nodes.iter_mut().enumerate() {
            node.x = 0.0;
            node.y = index as f32 * 120.0;
        }
        snapshot.edges = [
            ("external", "auth"),
            ("external", "catalog"),
            ("external", "orders"),
            ("gateway", "auth"),
            ("gateway", "catalog"),
            ("gateway", "orders"),
        ]
        .into_iter()
        .map(|(source, target)| TopologyEdge {
            source: source.into(),
            target: target.into(),
            relation: RelationKindDto::Calls,
            provenance: ProvenanceDto::Observed,
            reasons: vec![],
        })
        .collect();

        for previous in [&horizontal, &vertical] {
            let next = build_server_geometry(&snapshot, None, Some(previous)).unwrap();
            let columns: std::collections::HashSet<_> = next.plain.nodes.iter().map(|node| node.x.to_bits()).collect();
            let rows: std::collections::HashSet<_> = next.plain.nodes.iter().map(|node| node.y.to_bits()).collect();
            assert!(columns.len() > 1, "a branched graph stayed in one column: {:?}", next.plain.nodes);
            assert!(rows.len() > 1, "a branched graph stayed in one row: {:?}", next.plain.nodes);
        }
    }

    #[test]
    fn adding_a_call_keeps_infrastructure_anchors_when_membership_is_unchanged() {
        let mut snapshot = graph(&["a", "b"]);
        let infra = InfrastructureSnapshot {
            hosts: vec![HostDto { id: "machine".into(), name: "machine".into(), ..HostDto::default() }],
            instances: vec![
                InstanceDto {
                    service_id: "a".into(),
                    service_name: "a".into(),
                    instance_id: "a1".into(),
                    host_id: Some("machine".into()),
                    ..InstanceDto::default()
                },
                InstanceDto {
                    service_id: "b".into(),
                    service_name: "b".into(),
                    instance_id: "b1".into(),
                    host_id: Some("machine".into()),
                    ..InstanceDto::default()
                },
            ],
            ..InfrastructureSnapshot::default()
        };
        let old = build_server_geometry(&snapshot, Some(&infra), None).unwrap();
        snapshot.edges.push(TopologyEdge {
            source: "a".into(),
            target: "b".into(),
            relation: RelationKindDto::Calls,
            provenance: ProvenanceDto::Observed,
            reasons: vec![],
        });
        let next = build_server_geometry(&snapshot, Some(&infra), Some(&old)).unwrap();
        for node in &old.infrastructure.as_ref().unwrap().nodes {
            let actual = next.infrastructure.as_ref().unwrap().nodes.iter().find(|n| n.id == node.id).unwrap();
            assert_eq!((actual.x, actual.y), (node.x, node.y));
        }
    }

    #[test]
    fn new_backward_links_get_a_corridor_below_the_cards() {
        let mut snapshot = graph(&["a", "b", "c"]);
        for (source, target) in [("a", "b"), ("b", "c")] {
            snapshot.edges.push(TopologyEdge {
                source: source.into(),
                target: target.into(),
                relation: RelationKindDto::Calls,
                provenance: ProvenanceDto::Observed,
                reasons: vec![],
            });
        }
        let old = build_server_geometry(&snapshot, None, None).unwrap();
        snapshot.edges.push(TopologyEdge {
            source: "c".into(),
            target: "a".into(),
            relation: RelationKindDto::Calls,
            provenance: ProvenanceDto::Observed,
            reasons: vec![],
        });
        let next = build_server_geometry(&snapshot, None, Some(&old)).unwrap();
        assert_eq!(next.plain.edges[2].points.len(), 4);
        assert!(
            next.plain.edges[2].points[1][1] > next.plain.nodes.iter().map(|n| n.y).fold(f32::MIN, f32::max) + 28.0
        );
    }

    #[test]
    fn absent_selected_time_metrics_clear_saved_hot_host_decorations() {
        let snapshot = graph(&["a"]);
        let infra = InfrastructureSnapshot {
            hosts: vec![HostDto {
                id: "machine".into(),
                name: "machine".into(),
                has_metrics: true,
                cpu_utilization: Some(0.99),
                ..HostDto::default()
            }],
            instances: vec![InstanceDto {
                service_id: "a".into(),
                service_name: "a".into(),
                instance_id: "a1".into(),
                host_id: Some("machine".into()),
                ..InstanceDto::default()
            }],
            ..InfrastructureSnapshot::default()
        };
        let geometry = build_server_geometry(&snapshot, Some(&infra), None).unwrap();
        let mut engine = Engine::new(EngineConfig::default());
        engine
            .load_topology_json(
                &serde_json::json!({"nodes":snapshot.nodes,"edges":snapshot.edges,"geometry":geometry}).to_string(),
            )
            .unwrap();
        engine.set_filters(Filters { infrastructure: true, ..Filters::default() });
        engine.set_infrastructure(InfrastructureSnapshot::default());
        let frame = &engine.topology().infra.unwrap().frames[0];
        assert!(!frame.hot);
        assert!(!frame.has_metrics);
        assert!(frame.label.contains("no collector metrics"));
    }

    #[test]
    fn selected_time_server_geometry_moves_a_replica_to_its_machine_atomically() {
        let graph = graph(&["a"]);
        let old = InfrastructureSnapshot {
            hosts: vec![HostDto { id: "old".into(), name: "old".into(), ..HostDto::default() }],
            instances: vec![InstanceDto {
                service_id: "a".into(),
                service_name: "a".into(),
                instance_id: "a1".into(),
                host_id: Some("old".into()),
                ..InstanceDto::default()
            }],
            ..InfrastructureSnapshot::default()
        };
        let geometry = build_server_geometry(&graph, Some(&old), None).unwrap();
        let mut engine = Engine::new(EngineConfig::default());
        engine
            .load_topology_json(
                &serde_json::json!({"nodes":graph.nodes,"edges":graph.edges,"geometry":geometry}).to_string(),
            )
            .unwrap();
        engine.set_filters(Filters { infrastructure: true, ..Filters::default() });
        let mut next = old.clone();
        next.hosts[0].id = "new".into();
        next.hosts[0].name = "new".into();
        next.instances[0].host_id = Some("new".into());
        let geometry = build_server_geometry(&graph, Some(&next), Some(&geometry)).unwrap();
        let mut body = serde_json::to_value(&next).unwrap();
        body["topology"] = serde_json::json!({"nodes":graph.nodes,"edges":graph.edges,"geometry":geometry});
        engine.set_infrastructure_json(&body.to_string()).unwrap();
        assert_eq!(engine.topology().infra.unwrap().frames[0].host_id, "new");
    }

    proptest! {
        #[test]
        fn reversed_input_keeps_coordinates(count in 1usize..40) {
            let ids: Vec<String> = (0..count).map(|i|format!("n{i:03}")).collect();
            let mut snapshot = graph(&ids.iter().map(String::as_str).collect::<Vec<_>>());
            let first = build_server_geometry(&snapshot,None,None).unwrap();
            snapshot.nodes.reverse();
            let second = build_server_geometry(&snapshot,None,None).unwrap();
            for node in &first.plain.nodes {
                let other=second.plain.nodes.iter().find(|n|n.id==node.id).unwrap();
                prop_assert_eq!((node.x,node.y),(other.x,other.y));
            }
        }
    }
}
