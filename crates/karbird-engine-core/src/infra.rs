//! Infrastructure layer of the map (ADR-0018): service cards grouped into the machines they run
//! on. Pure and deterministic: the same topology and snapshot always give the same picture.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::types::{InfraBadge, InfraChip, InfraFrame, InfraLink, InfraView, InfrastructureSnapshot};

pub const CELL_W: f32 = 220.0;
pub const CELL_H: f32 = 96.0;
pub const CHIP_W: f32 = 150.0;
pub const CHIP_H: f32 = 28.0;
const CHIP_PITCH_X: f32 = 160.0;
const CHIP_PITCH_Y: f32 = 36.0;
const MAX_CARD_COLS: usize = 3;
const PAD: f32 = 28.0;
const HEADER: f32 = 44.0;
const MIN_INNER_W: f32 = 260.0;
const FRAME_GAP: f32 = 90.0;
/// Gap between the frames and the nodes that run on no known machine (databases, externals…).
const UNPLACED_GAP: f32 = 160.0;
pub const HOT_CPU: f64 = 0.9;

/// A node the layout may place: index, ID and whether it is a service.
pub struct LayoutNode<'a> {
    pub index: u32,
    pub id: &'a str,
    pub name: &'a str,
    pub service: bool,
    pub base: (f32, f32),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct InfraLayout {
    pub positions: HashMap<u32, (f32, f32)>,
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
        return format!("{name} · no agent metrics");
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
    chips: Vec<(u32, &'a str, &'a str, usize)>,
}

pub fn compute(nodes: &[LayoutNode<'_>], snap: &InfrastructureSnapshot) -> InfraLayout {
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
        let mut per_host: BTreeMap<&str, usize> = BTreeMap::new();
        for (_, h) in instances {
            if let Some(h) = h.filter(|h| known.contains(h)) {
                *per_host.entry(h).or_default() += 1;
            }
        }
        // Most instances wins; ties go to the first host by ID.
        let Some((&primary, _)) = per_host.iter().max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0))) else { continue };
        primary_of.insert(node, primary);
        if let Some(&s) = slot.get(primary) {
            placements[s].cards.push(node);
        }
        if instances.len() > 1 {
            badges.push(InfraBadge { node, count: instances.len() as u32, hosts: per_host.len() as u32 });
            for (n, (instance, host)) in instances.iter().enumerate() {
                if let Some(&s) = host.and_then(|h| slot.get(h)) {
                    placements[s].chips.push((node, service_id, instance, n + 1));
                }
            }
        }
    }

    let name_of: HashMap<u32, &str> = nodes.iter().map(|n| (n.index, n.name)).collect();
    let mut positions = HashMap::new();
    let mut frames = Vec::new();
    let mut chips = Vec::new();
    let cols = (placements.len() as f32).sqrt().ceil().max(1.0) as usize;
    let (mut x, mut y, mut row_h, mut max_x) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for (i, (p, h)) in placements.iter().zip(hosts.iter()).enumerate() {
        if i > 0 && i % cols == 0 {
            x = 0.0;
            y += row_h + FRAME_GAP;
            row_h = 0.0;
        }
        let card_cols = p.cards.len().clamp(1, MAX_CARD_COLS);
        let card_rows = p.cards.len().div_ceil(card_cols);
        let inner_w = (card_cols as f32 * CELL_W).max(MIN_INNER_W);
        let chip_cols = ((inner_w / CHIP_PITCH_X).floor() as usize).max(1);
        let chip_rows = p.chips.len().div_ceil(chip_cols);
        let width = inner_w + 2.0 * PAD;
        let height = HEADER + card_rows as f32 * CELL_H + chip_rows as f32 * CHIP_PITCH_Y + PAD;
        for (k, &node) in p.cards.iter().enumerate() {
            let (c, r) = (k % card_cols, k / card_cols);
            positions.insert(
                node,
                (x + PAD + c as f32 * CELL_W + CELL_W / 2.0, y + HEADER + r as f32 * CELL_H + CELL_H / 2.0),
            );
        }
        let chips_top = y + HEADER + card_rows as f32 * CELL_H;
        for (k, (node, service_id, instance, n)) in p.chips.iter().enumerate() {
            let (c, r) = (k % chip_cols, k / chip_cols);
            let service = name_of.get(node).copied().unwrap_or(service_id);
            chips.push(InfraChip {
                node: *node,
                service_id: (*service_id).to_owned(),
                instance_id: (*instance).to_owned(),
                label: format!(
                    "{} #{n}",
                    if service.is_empty() { display_name(snap, service_id).unwrap_or(service_id) } else { service }
                ),
                host_id: p.host.to_owned(),
                x: x + PAD + c as f32 * CHIP_PITCH_X + CHIP_W / 2.0,
                y: chips_top + r as f32 * CHIP_PITCH_Y + CHIP_H / 2.0,
            });
        }
        let hot = h.cpu_utilization.is_some_and(|c| c >= HOT_CPU) || snap.hints.iter().any(|x| x.host_id == h.id);
        frames.push(InfraFrame {
            host_id: h.id.clone(),
            label: frame_label(h),
            x,
            y,
            width,
            height,
            hot,
            has_metrics: h.has_metrics,
        });
        x += width + FRAME_GAP;
        max_x = max_x.max(x - FRAME_GAP);
        row_h = row_h.max(height);
    }

    // Everything not on a machine keeps its layered position, moved to the right of the frames.
    let unplaced: Vec<&LayoutNode<'_>> = nodes.iter().filter(|n| !positions.contains_key(&n.index)).collect();
    if let (Some(min_x), Some(min_y)) =
        (unplaced.iter().map(|n| n.base.0).reduce(f32::min), unplaced.iter().map(|n| n.base.1).reduce(f32::min))
    {
        let dx = if frames.is_empty() { 0.0 } else { max_x + UNPLACED_GAP - min_x };
        let dy = if frames.is_empty() { 0.0 } else { -min_y + HEADER };
        for n in unplaced {
            positions.insert(n.index, (n.base.0 + dx, n.base.1 + dy));
        }
    }

    let links = noisy_links(snap, &node_of, &positions, &chips, &frames);
    InfraLayout { positions, view: InfraView { frames, chips, badges, links } }
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
        .find(|c| instance.is_none_or(|i| c.instance_id == i))
        .or_else(|| chips.iter().find(on_host))
        .map(|c| [c.x, c.y])
        .or_else(|| positions.get(&node).map(|&(x, y)| [x, y]))
}

fn noisy_links(
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
    use crate::types::{HostDto, InstanceDto, InstanceMatch, NoisyNeighborHint};
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

    fn nodes() -> Vec<LayoutNode<'static>> {
        IDS.iter()
            .enumerate()
            .map(|(i, id)| LayoutNode {
                index: i as u32,
                id,
                name: id.trim_start_matches("svc_"),
                service: id.starts_with("svc_"),
                base: (i as f32 * 100.0, 500.0 + i as f32),
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
        assert_eq!(frame_label(&host("edge", None)), "edge · no agent metrics");
        let cpu_only = HostDto { memory_total_bytes: None, ..host("h", Some(0.054)) };
        assert_eq!(frame_label(&cpu_only), "h · CPU 5%");
        let unnamed = HostDto { name: String::new(), ..host("i-0abc", Some(1.0)) };
        assert!(frame_label(&unnamed).starts_with("i-0abc · CPU 100%"));
    }

    #[test]
    fn services_sit_inside_their_machine_frame() {
        let l = compute(&nodes(), &demo());
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
    fn replicas_get_a_badge_and_a_chip_per_instance_on_their_machine() {
        let l = compute(&nodes(), &demo());
        assert_eq!(l.view.badges, vec![InfraBadge { node: 3, count: 3, hosts: 2 }]);
        let chips: Vec<_> = l.view.chips.iter().map(|c| (c.label.as_str(), c.host_id.as_str())).collect();
        assert_eq!(chips.len(), 3);
        assert!(chips.contains(&("catalog #1", "vps-app")));
        assert!(chips.contains(&("catalog #2", "vps-worker")));
        assert!(chips.contains(&("catalog #3", "vps-worker")));
        for c in &l.view.chips {
            let f = l.view.frames.iter().find(|f| f.host_id == c.host_id).unwrap();
            assert!(inside(f, (c.x, c.y)), "{} inside {}", c.label, f.host_id);
            assert_eq!(l.chip_at(c.x + CHIP_W / 2.0 - 1.0, c.y), Some(3));
        }
        assert_eq!(l.chip_at(-1000.0, -1000.0), None);
    }

    #[test]
    fn unplaced_nodes_move_right_of_the_frames() {
        let l = compute(&nodes(), &demo());
        let right = l.view.frames.iter().map(|f| f.x + f.width).fold(0.0f32, f32::max);
        assert!(l.positions[&5].0 > right, "database is not on a machine");
    }

    #[test]
    fn noisy_neighbour_link_joins_victim_and_neighbour_on_the_host() {
        let l = compute(&nodes(), &demo());
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
        let l = compute(&nodes(), &s);
        let f = l.view.frames.iter().find(|f| f.host_id == "vps-app").unwrap();
        assert_eq!(l.view.links[0].to, [f.x + PAD, f.y + HEADER / 2.0]);
        assert_eq!(l.view.links[0].neighbor_node, None);
    }

    #[test]
    fn link_from_a_replica_starts_at_its_chip_on_the_hot_host() {
        let mut s = demo();
        s.hints[0].service_id = "svc_catalog".into();
        s.hints[0].instance_id = "catalog-1".into();
        let l = compute(&nodes(), &s);
        let chip = l.view.chips.iter().find(|c| c.instance_id == "catalog-1").unwrap();
        assert_eq!(l.view.links[0].from, [chip.x, chip.y]);
    }

    #[test]
    fn hosts_known_only_from_spans_are_framed_but_not_hot() {
        let mut s = demo();
        s.hosts.push(host("edge", None));
        s.instances.push(inst("gateway", 2, Some("edge")));
        let l = compute(&nodes(), &s);
        let edge = l.view.frames.iter().find(|f| f.host_id == "edge").unwrap();
        assert!(!edge.has_metrics && !edge.hot);
        assert_eq!(edge.label, "edge · no agent metrics");
    }

    #[test]
    fn empty_snapshot_keeps_the_layered_positions() {
        let l = compute(&nodes(), &InfrastructureSnapshot::default());
        assert!(l.view.frames.is_empty() && l.view.chips.is_empty());
        for n in nodes() {
            assert_eq!(l.positions[&n.index], n.base);
        }
    }

    #[test]
    fn instances_of_unknown_services_or_hosts_are_ignored() {
        let mut s = demo();
        s.instances.push(inst("ghost", 1, Some("vps-app")));
        s.instances.push(inst("payments", 2, Some("nowhere")));
        let l = compute(&nodes(), &s);
        assert!(l.view.chips.iter().all(|c| c.service_id != "svc_ghost"));
        let payments = l.view.badges.iter().find(|b| b.node == 4).unwrap();
        assert_eq!((payments.count, payments.hosts), (2, 1));
    }

    proptest! {
        #[test]
        fn layout_is_deterministic_and_every_node_is_placed(
            placements in prop::collection::vec((0usize..5, 0usize..4), 0..30),
            cpus in prop::collection::vec(prop::option::of(0.0f64..1.0), 4),
        ) {
            let hosts = ["h0", "h1", "h2", "h3"];
            let mut snap = InfrastructureSnapshot {
                hosts: hosts.iter().zip(&cpus).map(|(h, c)| host(h, *c)).collect(),
                ..InfrastructureSnapshot::default()
            };
            let services = ["gateway", "orders", "inventory", "catalog", "payments"];
            for (n, (s, h)) in placements.iter().enumerate() {
                snap.instances.push(inst(services[*s], n as u32, Some(hosts[*h])));
            }
            let a = compute(&nodes(), &snap);
            let mut reversed = snap.clone();
            reversed.hosts.reverse();
            reversed.instances.reverse();
            let b = compute(&nodes(), &reversed);
            prop_assert_eq!(&a, &b);
            prop_assert_eq!(a.positions.len(), IDS.len());
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
