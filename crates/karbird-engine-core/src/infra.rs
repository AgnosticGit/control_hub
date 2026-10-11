//! Infrastructure layer of the map (ADR-0018): service cards grouped into the machines they run
//! on. Pure and deterministic: the same topology and snapshot always give the same picture.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::layout::{BandSpec, COLUMN_SPACING, Layout};
use crate::types::{InfraBadge, InfraChip, InfraFrame, InfraLink, InfraView, InfrastructureSnapshot, ResourceLocation};

pub const CHIP_W: f32 = 220.0;
pub const CHIP_H: f32 = 104.0;
const CHIP_PITCH_Y: f32 = 116.0;
/// Space between the lowest card or corridor of a machine and its first replica card.
const CHIP_GAP: f32 = 18.0;
/// Half the width of the maximum service card in the renderer (220 px).
const CARD_HALF_W: f32 = 110.0;
const PAD: f32 = 28.0;
const HEADER: f32 = 44.0;
const MIN_INNER_W: f32 = 260.0;
pub const HOT_CPU: f64 = 0.9;

/// A node the layout may place: index, ID and whether it is a service.
pub struct LayoutNode<'a> {
    pub index: u32,
    pub id: &'a str,
    pub name: &'a str,
    pub service: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct InfraLayout {
    pub positions: HashMap<u32, (f32, f32)>,
    /// Corridor points of each edge, like [`Layout::waypoints`].
    pub waypoints: Vec<Vec<(f32, f32)>>,
    /// Database and queue nodes that run outside every machine with the agent.
    pub external: HashSet<u32>,
    pub view: InfraView,
}

impl InfraLayout {
    /// Service node under a chip at world point `(x, y)`.
    pub fn chip_at(&self, x: f32, y: f32) -> Option<u32> {
        self.view
            .chips
            .iter()
            .find(|c| (c.x - x).abs() <= CHIP_W / 2.0 && (c.y - y).abs() <= CHIP_H / 2.0)
            .map(|c| c.node)
    }
}

fn mb(bytes: f64) -> i64 {
    (bytes / 1_048_576.0).round() as i64
}

pub fn frame_label(h: &crate::types::HostDto) -> String {
    let name = if h.name.is_empty() { h.id.as_str() } else { h.name.as_str() };
    if !h.has_metrics {
        return format!("{name} · no collector metrics");
    }
    let mut label = name.to_owned();
    if let Some(cpu) = h.cpu_utilization {
        label.push_str(&format!(" · CPU {}%", (cpu * 100.0).round() as i64));
    }
    if let (Some(used), Some(total)) = (h.memory_used_bytes, h.memory_total_bytes) {
        label.push_str(&format!(" · RAM {}/{} MB", mb(used), mb(total)));
    }
    label
}

struct Placement<'a> {
    host: &'a str,
    cards: Vec<u32>,
    chips: Vec<(u32, &'a str, Vec<&'a str>)>,
}

/// `edges` are `(source, target)` node indices, like the engine's edge list.
pub fn compute(nodes: &[LayoutNode<'_>], edges: &[(u32, u32)], snap: &InfrastructureSnapshot) -> InfraLayout {
    let node_of: HashMap<&str, u32> = nodes.iter().filter(|n| n.service).map(|n| (n.id, n.index)).collect();

    // Instances per service (only services on the map), in a stable order.
    let mut by_service: BTreeMap<&str, Vec<(&str, Option<&str>)>> = BTreeMap::new();
    for i in &snap.instances {
        if node_of.contains_key(i.service_id.as_str()) {
            by_service.entry(i.service_id.as_str()).or_default().push((i.instance_id.as_str(), i.host_id.as_deref()));
        }
    }
    for list in by_service.values_mut() {
        list.sort();
        list.dedup();
    }

    let mut hosts: Vec<&crate::types::HostDto> = snap.hosts.iter().collect();
    hosts.sort_by(|a, b| (a.name.as_str(), a.id.as_str()).cmp(&(b.name.as_str(), b.id.as_str())));
    let known: BTreeSet<&str> = hosts.iter().map(|h| h.id.as_str()).collect();
    let mut placements: Vec<Placement<'_>> =
        hosts.iter().map(|h| Placement { host: h.id.as_str(), cards: Vec::new(), chips: Vec::new() }).collect();
    let slot: HashMap<&str, usize> = placements.iter().enumerate().map(|(i, p)| (p.host, i)).collect();

    let mut badges = Vec::new();
    let mut primary_of: HashMap<u32, &str> = HashMap::new();
    for (service_id, instances) in &by_service {
        let Some(&node) = node_of.get(service_id) else { continue };
        let mut per_host: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (instance_id, h) in instances {
            if let Some(h) = h.filter(|h| known.contains(h)) {
                per_host.entry(h).or_default().push(instance_id);
            }
        }
        // Most instances wins; ties go to the first host by ID.
        let Some((&primary, _)) = per_host.iter().max_by(|a, b| a.1.len().cmp(&b.1.len()).then(b.0.cmp(a.0))) else {
            continue;
        };
        primary_of.insert(node, primary);
        if let Some(&s) = slot.get(primary) {
            placements[s].cards.push(node);
        }
        if instances.len() > 1 {
            badges.push(InfraBadge { node, count: instances.len() as u32, hosts: per_host.len() as u32 });
            for (host, instance_ids) in per_host {
                // The logical card already represents the primary host. Replica placements
                // on every other host get a full-size card in the reserved lower band.
                if host != primary
                    && let Some(&s) = slot.get(host)
                {
                    placements[s].chips.push((node, service_id, instance_ids));
                }
            }
        }
    }

    // Databases and queues the API placed on a machine join its frame; those it found at an
    // address no agent answers to are external while this layer is on.
    let node_by_id: HashMap<&str, u32> = nodes.iter().filter(|n| !n.service).map(|n| (n.id, n.index)).collect();
    let mut external = HashSet::new();
    for r in &snap.resources {
        let Some(&node) = node_by_id.get(r.node_id.as_str()) else { continue };
        match r.location {
            ResourceLocation::Internal => {
                if let Some(&s) = r.host_id.as_deref().and_then(|h| slot.get(h))
                    && !placements.iter().any(|p| p.cards.contains(&node))
                {
                    placements[s].cards.push(node);
                }
            }
            ResourceLocation::External => {
                external.insert(node);
            }
            ResourceLocation::Unknown => {}
        }
    }

    // One band per machine (cards of the services whose most instances run there); everything
    // on no known machine is frameless and sits level with its neighbours. Columns stay those
    // of the layered layout, so calls still flow left to right and long edges keep corridors
    // free of cards.
    let unplaced = placements.len();
    let slot_of_node: HashMap<u32, usize> =
        placements.iter().enumerate().flat_map(|(s, p)| p.cards.iter().map(move |&n| (n, s))).collect();
    let at: HashMap<u32, usize> = nodes.iter().enumerate().map(|(i, n)| (n.index, i)).collect();
    let ids: Vec<String> = nodes.iter().map(|n| n.id.to_owned()).collect();
    let local: Vec<(u32, u32)> = edges
        .iter()
        .map(|&(s, t)| {
            let (s, t) = (at.get(&s).copied().unwrap_or(usize::MAX), at.get(&t).copied().unwrap_or(usize::MAX));
            (u32::try_from(s).unwrap_or(u32::MAX), u32::try_from(t).unwrap_or(u32::MAX))
        })
        .collect();
    let group: Vec<usize> = nodes.iter().map(|n| slot_of_node.get(&n.index).copied().unwrap_or(unplaced)).collect();
    let depth = crate::layout::layers(nodes.len(), &local);
    let depth_of = |node: u32| at.get(&node).and_then(|&i| depth.get(i)).copied().unwrap_or(0);

    // Replica cards sit under the machine's cards, in the service column, stacked per column.
    let chip_row: Vec<Vec<usize>> = placements
        .iter()
        .map(|p| {
            let mut per_column: HashMap<usize, usize> = HashMap::new();
            p.chips
                .iter()
                .map(|(node, ..)| {
                    let r = per_column.entry(depth_of(*node)).or_default();
                    *r += 1;
                    *r - 1
                })
                .collect()
        })
        .collect();
    let mut specs: Vec<BandSpec> = placements
        .iter()
        .zip(&chip_row)
        .map(|(p, rows)| {
            let chips_h = rows.iter().max().map_or(0.0, |&r| CHIP_GAP + r as f32 * CHIP_PITCH_Y + CHIP_H);
            let columns = p.chips.iter().map(|(node, ..)| depth_of(*node)).collect();
            BandSpec { top: HEADER, bottom: chips_h + PAD, columns, free: false }
        })
        .collect();
    specs.push(BandSpec { free: true, ..BandSpec::default() });
    let mut layout = Layout::default();
    layout.compute_banded(&ids, &local, &group, &specs);

    let name_of: HashMap<u32, &str> = nodes.iter().map(|n| (n.index, n.name)).collect();
    let positions: HashMap<u32, (f32, f32)> =
        nodes.iter().map(|n| (n.index, layout.position(n.id).unwrap_or((0.0, 0.0)))).collect();
    let waypoints = (0..edges.len()).map(|e| layout.waypoints(e).to_vec()).collect();
    let mut frames = Vec::new();
    let mut chips = Vec::new();
    for (s, (p, h)) in placements.iter().zip(hosts.iter()).enumerate() {
        let band = layout.band(s).unwrap_or_default();
        let mut xs: Vec<(f32, f32)> =
            p.cards.iter().filter_map(|n| positions.get(n)).map(|&(x, _)| (x - CARD_HALF_W, x + CARD_HALF_W)).collect();
        for ((node, service_id, instance_ids), &r) in p.chips.iter().zip(&chip_row[s]) {
            let service = name_of.get(node).copied().unwrap_or(service_id);
            let x = depth_of(*node) as f32 * COLUMN_SPACING;
            xs.push((x - CHIP_W / 2.0, x + CHIP_W / 2.0));
            chips.push(InfraChip {
                node: *node,
                service_id: (*service_id).to_owned(),
                instance_id: instance_ids.first().copied().unwrap_or_default().to_owned(),
                instance_ids: instance_ids.iter().map(|id| (*id).to_owned()).collect(),
                label: if service.is_empty() { display_name(snap, service_id).unwrap_or(service_id) } else { service }
                    .to_owned(),
                host_id: p.host.to_owned(),
                x,
                y: band.content_bottom + CHIP_GAP + r as f32 * CHIP_PITCH_Y + CHIP_H / 2.0,
            });
        }
        let left = xs.iter().map(|x| x.0).reduce(f32::min).unwrap_or(0.0) - PAD;
        let right = xs.iter().map(|x| x.1).reduce(f32::max).unwrap_or(0.0) + PAD;
        let hot = h.cpu_utilization.is_some_and(|c| c >= HOT_CPU) || snap.hints.iter().any(|x| x.host_id == h.id);
        frames.push(InfraFrame {
            host_id: h.id.clone(),
            label: frame_label(h),
            x: left,
            y: band.top,
            width: (right - left).max(MIN_INNER_W + 2.0 * PAD),
            height: band.bottom - band.top,
            hot,
            has_metrics: h.has_metrics,
        });
    }

    let links = noisy_links(snap, &node_of, &positions, &chips, &frames);
    InfraLayout { positions, waypoints, external, view: InfraView { frames, chips, badges, links } }
}

fn display_name<'a>(snap: &'a InfrastructureSnapshot, service_id: &str) -> Option<&'a str> {
    snap.instances
        .iter()
        .find(|i| i.service_id == service_id)
        .map(|i| i.service_name.as_str())
        .filter(|n| !n.is_empty())
}

/// Where a service appears on a host: its chip for that instance/host, else its card.
fn point_on_host(
    node: u32,
    host: &str,
    instance: Option<&str>,
    positions: &HashMap<u32, (f32, f32)>,
    chips: &[InfraChip],
) -> Option<[f32; 2]> {
    let on_host = |c: &&InfraChip| c.node == node && c.host_id == host;
    chips
        .iter()
        .filter(on_host)
        .find(|c| instance.is_none_or(|i| c.instance_ids.iter().any(|id| id == i)))
        .or_else(|| chips.iter().find(on_host))
        .map(|c| [c.x, c.y])
        .or_else(|| positions.get(&node).map(|&(x, y)| [x, y]))
}

pub(crate) fn noisy_links(
    snap: &InfrastructureSnapshot,
    node_of: &HashMap<&str, u32>,
    positions: &HashMap<u32, (f32, f32)>,
    chips: &[InfraChip],
    frames: &[InfraFrame],
) -> Vec<InfraLink> {
    let mut links = Vec::new();
    for h in &snap.hints {
        let Some(&victim) = node_of.get(h.service_id.as_str()) else { continue };
        let Some(from) = point_on_host(victim, &h.host_id, Some(&h.instance_id), positions, chips) else { continue };
        let neighbor = h.neighbor_service_id.as_deref().and_then(|id| node_of.get(id)).copied();
        let to = neighbor
            .and_then(|n| point_on_host(n, &h.host_id, None, positions, chips))
            .or_else(|| frames.iter().find(|f| f.host_id == h.host_id).map(|f| [f.x + PAD, f.y + HEADER / 2.0]));
        let Some(to) = to else { continue };
        links.push(InfraLink {
            from,
            to,
            victim_node: victim,
            neighbor_node: neighbor,
            host_id: h.host_id.clone(),
            label: format!(
                "noisy neighbour: {} at {}% CPU",
                h.neighbor_name,
                (h.neighbor_cpu_share * 100.0).round() as i64
            ),
        });
    }
    links
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{HostDto, InstanceDto, InstanceMatch, NoisyNeighborHint, ResourcePlacementDto};
    use proptest::prelude::*;

    fn host(id: &str, cpu: Option<f64>) -> HostDto {
        HostDto {
            id: id.into(),
            name: id.into(),
            has_metrics: cpu.is_some(),
            cpu_utilization: cpu,
            memory_used_bytes: cpu.map(|_| 870.0 * 1_048_576.0),
            memory_total_bytes: cpu.map(|_| 1024.0 * 1_048_576.0),
            ..HostDto::default()
        }
    }

    fn inst(service: &str, n: u32, host: Option<&str>) -> InstanceDto {
        InstanceDto {
            service_id: format!("svc_{service}"),
            service_name: service.into(),
            instance_id: format!("{service}-{n}"),
            host_id: host.map(str::to_owned),
            match_reason: if host.is_some() { InstanceMatch::ContainerId } else { InstanceMatch::Unknown },
            ..InstanceDto::default()
        }
    }

    const IDS: [&str; 6] = ["svc_gateway", "svc_orders", "svc_inventory", "svc_catalog", "svc_payments", "res_db"];
    /// gateway → orders, catalog; orders → inventory, payments; catalog → inventory; inventory → db.
    const EDGES: [(u32, u32); 6] = [(0, 1), (0, 3), (1, 2), (1, 4), (3, 2), (2, 5)];
    const CARD_HALF_H: f32 = crate::layout::CARD_HALF_H;

    fn nodes() -> Vec<LayoutNode<'static>> {
        IDS.iter()
            .enumerate()
            .map(|(i, id)| LayoutNode {
                index: i as u32,
                id,
                name: id.trim_start_matches("svc_"),
                service: id.starts_with("svc_"),
            })
            .collect()
    }

    fn demo() -> InfrastructureSnapshot {
        InfrastructureSnapshot {
            hosts: vec![host("vps-worker", Some(0.3)), host("vps-app", Some(0.92))],
            instances: vec![
                inst("gateway", 1, Some("vps-app")),
                inst("orders", 1, Some("vps-app")),
                inst("inventory", 1, Some("vps-app")),
                inst("catalog", 1, Some("vps-app")),
                inst("catalog", 2, Some("vps-worker")),
                inst("catalog", 3, Some("vps-worker")),
                inst("payments", 1, Some("vps-worker")),
            ],
            hints: vec![NoisyNeighborHint {
                service_id: "svc_orders".into(),
                service_name: "orders".into(),
                instance_id: "orders-1".into(),
                neighbor_service_id: Some("svc_inventory".into()),
                neighbor_name: "inventory".into(),
                host_id: "vps-app".into(),
                host_name: "vps-app".into(),
                neighbor_cpu_share: 0.95,
                ..NoisyNeighborHint::default()
            }],
            ..InfrastructureSnapshot::default()
        }
    }

    fn inside(f: &InfraFrame, (x, y): (f32, f32)) -> bool {
        x > f.x && x < f.x + f.width && y > f.y && y < f.y + f.height
    }

    #[test]
    fn frame_labels_match_the_spec() {
        assert_eq!(frame_label(&host("vps-app", Some(0.92))), "vps-app · CPU 92% · RAM 870/1024 MB");
        assert_eq!(frame_label(&host("edge", None)), "edge · no collector metrics");
        let cpu_only = HostDto { memory_total_bytes: None, ..host("h", Some(0.054)) };
        assert_eq!(frame_label(&cpu_only), "h · CPU 5%");
        let unnamed = HostDto { name: String::new(), ..host("i-0abc", Some(1.0)) };
        assert!(frame_label(&unnamed).starts_with("i-0abc · CPU 100%"));
    }

    #[test]
    fn services_sit_inside_their_machine_frame() {
        let l = compute(&nodes(), &EDGES, &demo());
        let frame = |id: &str| l.view.frames.iter().find(|f| f.host_id == id).unwrap();
        let app = frame("vps-app");
        let worker = frame("vps-worker");
        for node in [0u32, 1, 2] {
            assert!(inside(app, l.positions[&node]), "node {node} on vps-app");
        }
        // catalog has 1 instance on vps-app and 2 on vps-worker: the card goes where most run.
        assert!(inside(worker, l.positions[&3]));
        assert!(inside(worker, l.positions[&4]));
        assert!(app.hot && !worker.hot);
        assert_eq!(app.label, "vps-app · CPU 92% · RAM 870/1024 MB");
        // Frames do not overlap.
        assert!(app.x + app.width <= worker.x || worker.x + worker.width <= app.x || app.y + app.height <= worker.y);
    }

    #[test]
    fn replicas_are_grouped_per_machine_with_a_local_replica_count() {
        let l = compute(&nodes(), &EDGES, &demo());
        assert_eq!(l.view.badges, vec![InfraBadge { node: 3, count: 3, hosts: 2 }]);
        let chips: Vec<_> = l.view.chips.iter().map(|c| (c.label.as_str(), c.host_id.as_str())).collect();
        assert_eq!(chips.len(), 1, "the primary host already has the regular service card");
        assert!(chips.contains(&("catalog", "vps-app")));
        assert!(!chips.iter().any(|(_, host)| *host == "vps-worker"));
        assert_eq!(l.view.chips.iter().find(|c| c.host_id == "vps-app").unwrap().instance_ids, ["catalog-1"]);
        for c in &l.view.chips {
            let f = l.view.frames.iter().find(|f| f.host_id == c.host_id).unwrap();
            assert!(inside(f, (c.x, c.y)), "{} inside {}", c.label, f.host_id);
            assert_eq!(l.chip_at(c.x + CHIP_W / 2.0 - 1.0, c.y), Some(3));
            assert!(c.x - CHIP_W / 2.0 >= f.x, "replica card {} left inside {}", c.label, f.host_id);
            assert!(c.x + CHIP_W / 2.0 <= f.x + f.width, "replica card {} right inside {}", c.label, f.host_id);
            assert!(c.y - CHIP_H / 2.0 >= f.y, "replica card {} top inside {}", c.label, f.host_id);
            assert!(c.y + CHIP_H / 2.0 <= f.y + f.height, "replica card {} bottom inside {}", c.label, f.host_id);
        }
        assert_eq!(CHIP_W, CARD_HALF_W * 2.0, "a replica uses the service card width");
        assert_eq!(CHIP_H, CARD_HALF_H * 2.0, "a replica uses the service card height");
        assert_eq!(CHIP_W, 220.0, "expanded service metric cards reserve 220 px width");
        assert_eq!(CHIP_H, 104.0, "expanded service metric cards reserve 104 px height");
        assert_eq!(l.chip_at(-1000.0, -1000.0), None);
    }

    #[test]
    fn unplaced_nodes_stay_in_the_flow_outside_every_frame() {
        let l = compute(&nodes(), &EDGES, &demo());
        for f in &l.view.frames {
            assert!(!inside(f, l.positions[&5]), "database is not on {}", f.host_id);
        }
        // Calls still go left to right: the database is right of inventory, which calls it,
        // and with no frame in its column it sits level with it instead of in a band of its own.
        assert!(l.positions[&5].0 > l.positions[&2].0);
        assert_eq!(l.positions[&5].1, l.positions[&2].1);
    }

    /// The card box (172 × 56) around a node position.
    fn card(p: (f32, f32)) -> (f32, f32, f32, f32) {
        (p.0 - CARD_HALF_W, p.1 - CARD_HALF_H, p.0 + CARD_HALF_W, p.1 + CARD_HALF_H)
    }

    fn overlaps(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> bool {
        a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3
    }

    /// Cards never overlap each other or a chip, a frame holds only its own machine's cards and
    /// corridor points never touch a card.
    fn assert_clean(l: &InfraLayout, snap: &InfrastructureSnapshot) {
        let cards: Vec<(u32, (f32, f32, f32, f32))> = l.positions.iter().map(|(&n, &p)| (n, card(p))).collect();
        for (i, (a, ra)) in cards.iter().enumerate() {
            for (b, rb) in &cards[i + 1..] {
                assert!(!overlaps(*ra, *rb), "cards {a} and {b} overlap");
            }
            for c in &l.view.chips {
                let rc = (c.x - CHIP_W / 2.0, c.y - CHIP_H / 2.0, c.x + CHIP_W / 2.0, c.y + CHIP_H / 2.0);
                assert!(!overlaps(*ra, rc), "card {a} overlaps chip {}", c.label);
            }
        }
        for f in &l.view.frames {
            for (&n, &p) in &l.positions {
                let primary = snap
                    .instances
                    .iter()
                    .filter(|i| i.service_id == IDS[n as usize])
                    .filter_map(|i| i.host_id.as_deref())
                    .chain(
                        snap.resources
                            .iter()
                            .filter(|r| r.node_id == IDS[n as usize] && r.location == ResourceLocation::Internal)
                            .filter_map(|r| r.host_id.as_deref()),
                    )
                    .any(|h| h == f.host_id);
                if inside(f, p) {
                    assert!(primary, "node {n} drawn inside {} where it does not run", f.host_id);
                }
            }
        }
        for w in &l.waypoints {
            for &(x, y) in w {
                for (n, r) in &cards {
                    assert!(
                        !(x > r.0 && x < r.2 && y > r.1 - 8.0 && y < r.3 + 8.0),
                        "corridor ({x}, {y}) touches card {n}"
                    );
                }
            }
        }
    }

    #[test]
    fn edges_cards_chips_and_frames_never_lie_on_each_other() {
        let s = demo();
        assert_clean(&compute(&nodes(), &EDGES, &s), &s);
    }

    #[test]
    fn long_edges_across_machines_get_corridors_clear_of_every_card() {
        // gateway (vps-app) → db (no machine) skips two columns; catalog (vps-worker) → db one.
        let edges = [EDGES.as_slice(), &[(0, 5), (3, 5)]].concat();
        let s = demo();
        let l = compute(&nodes(), &edges, &s);
        assert_eq!(l.waypoints[6].len(), 2, "one corridor point per skipped column");
        assert_eq!(l.waypoints[7].len(), 1);
        assert_clean(&l, &s);
    }

    #[test]
    fn frames_never_overlap_and_keep_the_columns_of_the_plain_layout() {
        let l = compute(&nodes(), &EDGES, &demo());
        let ids: Vec<String> = IDS.iter().map(|s| (*s).to_owned()).collect();
        let mut plain = crate::layout::Layout::default();
        plain.compute(&ids, &EDGES);
        for (i, id) in IDS.iter().enumerate() {
            assert_eq!(l.positions[&(i as u32)].0, plain.position(id).unwrap().0, "{id} keeps its column");
        }
        assert_frames_apart(&l);
    }

    fn assert_frames_apart(l: &InfraLayout) {
        let rect = |f: &InfraFrame| (f.x, f.y, f.x + f.width, f.y + f.height);
        for (i, a) in l.view.frames.iter().enumerate() {
            for b in &l.view.frames[i + 1..] {
                assert!(!overlaps(rect(a), rect(b)), "frames {} and {} overlap", a.host_id, b.host_id);
            }
        }
    }

    #[test]
    fn machines_on_separate_columns_sit_side_by_side() {
        // gateway, orders on vps-app (columns 0-1); payments alone on vps-worker (column 2),
        // next door: those stack. Inventory and catalog on no machine.
        let mut s = demo();
        s.instances = vec![
            inst("gateway", 1, Some("vps-app")),
            inst("orders", 1, Some("vps-app")),
            inst("payments", 1, Some("vps-worker")),
        ];
        let l = compute(&nodes(), &EDGES, &s);
        assert_clean(&l, &s);
        assert_frames_apart(&l);
        // db (column 3) is level with inventory, its only neighbour, beside every frame.
        assert_eq!(l.positions[&5].1, l.positions[&2].1);
    }

    #[test]
    fn noisy_neighbour_link_joins_victim_and_neighbour_on_the_host() {
        let l = compute(&nodes(), &EDGES, &demo());
        assert_eq!(l.view.links.len(), 1);
        let link = &l.view.links[0];
        assert_eq!((link.victim_node, link.neighbor_node), (1, Some(2)));
        assert_eq!(link.from, [l.positions[&1].0, l.positions[&1].1]);
        assert_eq!(link.to, [l.positions[&2].0, l.positions[&2].1]);
        assert_eq!(link.label, "noisy neighbour: inventory at 95% CPU");
    }

    #[test]
    fn link_to_a_neighbour_without_a_node_points_at_the_frame_label() {
        let mut s = demo();
        s.hints[0].neighbor_service_id = None;
        s.hints[0].neighbor_name = "backup-job".into();
        let l = compute(&nodes(), &EDGES, &s);
        let f = l.view.frames.iter().find(|f| f.host_id == "vps-app").unwrap();
        assert_eq!(l.view.links[0].to, [f.x + PAD, f.y + HEADER / 2.0]);
        assert_eq!(l.view.links[0].neighbor_node, None);
    }

    #[test]
    fn link_from_a_replica_starts_at_its_chip_on_the_hot_host() {
        let mut s = demo();
        s.hints[0].service_id = "svc_catalog".into();
        s.hints[0].instance_id = "catalog-1".into();
        let l = compute(&nodes(), &EDGES, &s);
        let chip = l.view.chips.iter().find(|c| c.instance_ids.iter().any(|id| id == "catalog-1")).unwrap();
        assert_eq!(l.view.links[0].from, [chip.x, chip.y]);
    }

    #[test]
    fn hosts_known_only_from_spans_are_framed_but_not_hot() {
        let mut s = demo();
        s.hosts.push(host("edge", None));
        s.instances.push(inst("gateway", 2, Some("edge")));
        let l = compute(&nodes(), &EDGES, &s);
        let edge = l.view.frames.iter().find(|f| f.host_id == "edge").unwrap();
        assert!(!edge.has_metrics && !edge.hot);
        assert_eq!(edge.label, "edge · no collector metrics");
    }

    #[test]
    fn empty_snapshot_keeps_the_layered_positions() {
        let l = compute(&nodes(), &EDGES, &InfrastructureSnapshot::default());
        assert!(l.view.frames.is_empty() && l.view.chips.is_empty());
        let ids: Vec<String> = IDS.iter().map(|s| (*s).to_owned()).collect();
        let mut plain = crate::layout::Layout::default();
        plain.compute(&ids, &EDGES);
        for n in nodes() {
            assert_eq!(Some(l.positions[&n.index]), plain.position(n.id));
        }
        for e in 0..EDGES.len() {
            assert_eq!(l.waypoints[e], plain.waypoints(e));
        }
    }

    fn db_on(location: ResourceLocation, host: Option<&str>) -> ResourcePlacementDto {
        ResourcePlacementDto {
            node_id: "res_db".into(),
            name: "postgresql: orders".into(),
            kind: "database".into(),
            location,
            host_id: host.map(str::to_owned),
            ..ResourcePlacementDto::default()
        }
    }

    #[test]
    fn an_internal_database_sits_inside_its_machine_frame() {
        let mut s = demo();
        s.resources = vec![db_on(ResourceLocation::Internal, Some("vps-app"))];
        let l = compute(&nodes(), &EDGES, &s);
        let app = l.view.frames.iter().find(|f| f.host_id == "vps-app").unwrap();
        assert!(inside(app, l.positions[&5]), "database inside vps-app");
        assert!(l.external.is_empty());
        assert_clean(&l, &s);
        assert_frames_apart(&l);
    }

    #[test]
    fn an_external_database_stays_outside_and_is_marked() {
        let mut s = demo();
        s.resources = vec![db_on(ResourceLocation::External, None)];
        let l = compute(&nodes(), &EDGES, &s);
        for f in &l.view.frames {
            assert!(!inside(f, l.positions[&5]), "external database is not on {}", f.host_id);
        }
        assert_eq!(l.external, HashSet::from([5]));
    }

    #[test]
    fn unknown_or_unmatched_resources_change_nothing() {
        let base = compute(&nodes(), &EDGES, &demo());
        for r in [
            db_on(ResourceLocation::Unknown, None),
            db_on(ResourceLocation::Internal, Some("nowhere")),
            ResourcePlacementDto { node_id: "res_ghost".into(), ..db_on(ResourceLocation::Internal, Some("vps-app")) },
            ResourcePlacementDto { node_id: "svc_payments".into(), ..db_on(ResourceLocation::External, None) },
        ] {
            let mut s = demo();
            s.resources = vec![r.clone()];
            assert_eq!(compute(&nodes(), &EDGES, &s), base, "{r:?}");
        }
    }

    #[test]
    fn instances_of_unknown_services_or_hosts_are_ignored() {
        let mut s = demo();
        s.instances.push(inst("ghost", 1, Some("vps-app")));
        s.instances.push(inst("payments", 2, Some("nowhere")));
        let l = compute(&nodes(), &EDGES, &s);
        assert!(l.view.chips.iter().all(|c| c.service_id != "svc_ghost"));
        let payments = l.view.badges.iter().find(|b| b.node == 4).unwrap();
        assert_eq!((payments.count, payments.hosts), (2, 1));
    }

    proptest! {
        #[test]
        fn layout_is_deterministic_and_every_node_is_placed(
            placements in prop::collection::vec((0usize..5, 0usize..4), 0..30),
            cpus in prop::collection::vec(prop::option::of(0.0f64..1.0), 4),
            db_host in prop::option::of(0usize..4),
        ) {
            let hosts = ["h0", "h1", "h2", "h3"];
            let mut snap = InfrastructureSnapshot {
                hosts: hosts.iter().zip(&cpus).map(|(h, c)| host(h, *c)).collect(),
                resources: db_host.map(|h| db_on(ResourceLocation::Internal, Some(hosts[h]))).into_iter().collect(),
                ..InfrastructureSnapshot::default()
            };
            let services = ["gateway", "orders", "inventory", "catalog", "payments"];
            for (n, (s, h)) in placements.iter().enumerate() {
                snap.instances.push(inst(services[*s], n as u32, Some(hosts[*h])));
            }
            let a = compute(&nodes(), &EDGES, &snap);
            let mut reversed = snap.clone();
            reversed.hosts.reverse();
            reversed.instances.reverse();
            let b = compute(&nodes(), &EDGES, &reversed);
            prop_assert_eq!(&a, &b);
            prop_assert_eq!(a.positions.len(), IDS.len());
            assert_clean(&a, &snap);
            for c in &a.view.chips {
                let f = a.view.frames.iter().find(|f| f.host_id == c.host_id).unwrap();
                prop_assert!(inside(f, (c.x, c.y)));
            }
            for (i, f) in a.view.frames.iter().enumerate() {
                for g in &a.view.frames[i + 1..] {
                    let apart = f.x + f.width <= g.x || g.x + g.width <= f.x || f.y + f.height <= g.y || g.y + g.height <= f.y;
                    prop_assert!(apart, "{} overlaps {}", f.host_id, g.host_id);
                }
            }
        }
    }
}
