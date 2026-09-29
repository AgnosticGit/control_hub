//! Layered (Sugiyama-style) layout.
//!
//! Recomputed from scratch whenever the node or edge set changes, and deterministic for a
//! given graph (independent of insertion order):
//! 1. layers by longest path (cycle-safe, bounded);
//! 2. an edge spanning several layers gets a dummy item in each layer it crosses, so every
//!    link joins adjacent layers and the dummies reserve free corridors between cards;
//! 3. the order inside each layer minimises link crossings (barycenter sweeps, then adjacent
//!    swaps), keeping the best order seen;
//! 4. y coordinates follow the neighbours (iterative averaging) under minimum separations,
//!    solved exactly per layer with isotonic regression (pool adjacent violators).
//!
//! Metrics and new events never move nodes; only a changed graph does.

use std::collections::HashMap;

pub(crate) const COLUMN_SPACING: f32 = 300.0;
/// Centre distance between two cards in one column (cards are 56 px tall).
pub(crate) const ROW_SPACING: f32 = 120.0;
/// Centre distance between a card and a corridor: 28 px half-card plus 36 px clearance.
const CARD_TO_CORRIDOR: f32 = 64.0;
/// Distance between two corridors in one column.
const CORRIDOR_SPACING: f32 = 22.0;
/// Crossing counting is quadratic in the links of a layer pair: above these sizes the
/// barycenter sweeps (and the costlier adjacent swaps) are skipped and the ID order is kept.
const MAX_ORDERED_ITEMS: usize = 400;
const MAX_TRANSPOSED_ITEMS: usize = 150;
const SWEEPS: usize = 24;
const Y_ITERATIONS: usize = 24;

#[derive(Debug, Default, Clone)]
pub(crate) struct Layout {
    positions: HashMap<String, (f32, f32)>,
    /// Corridor points of each edge, indexed like the `edges` passed to [`Layout::compute`].
    waypoints: Vec<Vec<(f32, f32)>>,
}

/// A real node or one corridor slot of a long edge.
#[derive(Debug, Clone)]
struct Item {
    real: bool,
    layer: usize,
    /// Sort key for the initial order: the node ID, or `source\0target\0index` for corridors.
    key: String,
}

impl Layout {
    pub fn position(&self, id: &str) -> Option<(f32, f32)> {
        self.positions.get(id).copied()
    }

    pub fn waypoints(&self, edge: usize) -> &[(f32, f32)] {
        self.waypoints.get(edge).map_or(&[], Vec::as_slice)
    }

    /// Lay out `ids`; `edges` are `(source_index, target_index)` pairs into `ids`.
    pub fn compute(&mut self, ids: &[String], edges: &[(u32, u32)]) {
        let n = ids.len();
        let depth = layers(n, edges);

        let mut items: Vec<Item> =
            ids.iter().enumerate().map(|(i, id)| Item { real: true, layer: depth[i], key: id.clone() }).collect();
        // Links between adjacent layers, and the corridor items of each edge.
        let mut links: Vec<(usize, usize)> = Vec::new();
        let mut corridors: Vec<Vec<usize>> = vec![Vec::new(); edges.len()];
        for (ei, &(s, t)) in edges.iter().enumerate() {
            let (s, t) = (s as usize, t as usize);
            if s >= n || t >= n || depth[t] <= depth[s] {
                continue;
            }
            let mut prev = s;
            for layer in depth[s] + 1..depth[t] {
                let key = format!("{}\0{}\0{ei:08}", ids[s], ids[t]);
                items.push(Item { real: false, layer, key });
                let d = items.len() - 1;
                if let Some(c) = corridors.get_mut(ei) {
                    c.push(d);
                }
                links.push((prev, d));
                prev = d;
            }
            links.push((prev, t));
        }

        let layer_count = items.iter().map(|it| it.layer + 1).max().unwrap_or(0);
        let mut order: Vec<Vec<usize>> = vec![Vec::new(); layer_count];
        for (i, it) in items.iter().enumerate() {
            if let Some(l) = order.get_mut(it.layer) {
                l.push(i);
            }
        }
        for l in &mut order {
            l.sort_by(|&a, &b| items[a].key.cmp(&items[b].key));
        }

        let (preds, succs) = adjacency(items.len(), &links);
        if items.len() <= MAX_ORDERED_ITEMS {
            minimise_crossings(&mut order, &preds, &succs);
        }
        let ys = coordinates(&order, &items, &preds, &succs);

        self.positions.clear();
        for (i, id) in ids.iter().enumerate() {
            let y = ys.get(i).copied().unwrap_or(0.0);
            self.positions.insert(id.clone(), (depth[i] as f32 * COLUMN_SPACING, y));
        }
        self.waypoints = corridors
            .iter()
            .map(|c| {
                c.iter().map(|&d| (items[d].layer as f32 * COLUMN_SPACING, ys.get(d).copied().unwrap_or(0.0))).collect()
            })
            .collect();
    }
}

/// Longest-path layer of each node, with a bounded relaxation (cycle-safe).
fn layers(n: usize, edges: &[(u32, u32)]) -> Vec<usize> {
    let mut depth = vec![0usize; n];
    for _ in 0..n.min(64) {
        let mut changed = false;
        for &(s, t) in edges {
            let (s, t) = (s as usize, t as usize);
            if s < n && t < n && s != t && depth[t] < depth[s] + 1 && depth[s] < 32 {
                depth[t] = depth[s] + 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    depth
}

fn adjacency(count: usize, links: &[(usize, usize)]) -> (Vec<Vec<usize>>, Vec<Vec<usize>>) {
    let mut preds = vec![Vec::new(); count];
    let mut succs = vec![Vec::new(); count];
    for &(a, b) in links {
        if let (Some(s), Some(p)) = (succs.get_mut(a), preds.get_mut(b)) {
            s.push(b);
            p.push(a);
        }
    }
    (preds, succs)
}

fn positions_of(order: &[Vec<usize>], count: usize) -> Vec<usize> {
    let mut pos = vec![0usize; count];
    for layer in order {
        for (k, &i) in layer.iter().enumerate() {
            if let Some(p) = pos.get_mut(i) {
                *p = k;
            }
        }
    }
    pos
}

/// Link crossings between every pair of adjacent layers.
pub(crate) fn crossings(order: &[Vec<usize>], succs: &[Vec<usize>]) -> usize {
    let count = succs.len();
    let pos = positions_of(order, count);
    let mut total = 0;
    for layer in order {
        let mut ends: Vec<(usize, usize)> = Vec::new();
        for &a in layer {
            for &b in succs.get(a).map_or(&[][..], Vec::as_slice) {
                ends.push((pos[a], pos[b]));
            }
        }
        for i in 0..ends.len() {
            for j in i + 1..ends.len() {
                let (a1, b1) = ends[i];
                let (a2, b2) = ends[j];
                if (a1 < a2 && b1 > b2) || (a1 > a2 && b1 < b2) {
                    total += 1;
                }
            }
        }
    }
    total
}

fn minimise_crossings(order: &mut [Vec<usize>], preds: &[Vec<usize>], succs: &[Vec<usize>]) {
    let count = preds.len();
    let mut best = order.to_vec();
    let mut best_cross = crossings(order, succs);
    for sweep in 0..SWEEPS {
        if best_cross == 0 {
            break;
        }
        let down = sweep % 2 == 0;
        let layers: Vec<usize> =
            if down { (1..order.len()).collect() } else { (0..order.len().saturating_sub(1)).rev().collect() };
        for l in layers {
            let pos = positions_of(order, count);
            let neighbours = if down { preds } else { succs };
            let Some(layer) = order.get_mut(l) else { continue };
            let bary: HashMap<usize, f64> = layer
                .iter()
                .map(|&i| {
                    let ns = neighbours.get(i).map_or(&[][..], Vec::as_slice);
                    let b = if ns.is_empty() {
                        pos[i] as f64
                    } else {
                        ns.iter().map(|&j| pos[j] as f64).sum::<f64>() / ns.len() as f64
                    };
                    (i, b)
                })
                .collect();
            layer.sort_by(|a, b| {
                let (ba, bb) = (bary.get(a).copied().unwrap_or(0.0), bary.get(b).copied().unwrap_or(0.0));
                ba.total_cmp(&bb).then(pos[*a].cmp(&pos[*b]))
            });
        }
        if count <= MAX_TRANSPOSED_ITEMS {
            transpose(order, succs);
        }
        let c = crossings(order, succs);
        if c < best_cross {
            best_cross = c;
            best = order.to_vec();
        }
    }
    order.clone_from_slice(&best);
}

/// Swaps neighbours inside a layer while that strictly reduces the total crossings.
fn transpose(order: &mut [Vec<usize>], succs: &[Vec<usize>]) {
    for _ in 0..8 {
        let mut improved = false;
        for l in 0..order.len() {
            let len = order.get(l).map_or(0, Vec::len);
            for k in 0..len.saturating_sub(1) {
                let before = crossings(order, succs);
                if let Some(layer) = order.get_mut(l) {
                    layer.swap(k, k + 1);
                }
                if crossings(order, succs) < before {
                    improved = true;
                } else if let Some(layer) = order.get_mut(l) {
                    layer.swap(k, k + 1);
                }
            }
        }
        if !improved {
            break;
        }
    }
}

fn separation(a: &Item, b: &Item) -> f32 {
    match (a.real, b.real) {
        (true, true) => ROW_SPACING,
        (false, false) => CORRIDOR_SPACING,
        _ => CARD_TO_CORRIDOR,
    }
}

/// y of every item: follow the neighbours, keep the layer order and minimum separations.
fn coordinates(order: &[Vec<usize>], items: &[Item], preds: &[Vec<usize>], succs: &[Vec<usize>]) -> Vec<f32> {
    let mut y = vec![0.0f32; items.len()];
    // Start compact and centred on 0.
    for layer in order {
        let mut acc = 0.0;
        for (k, &i) in layer.iter().enumerate() {
            if k > 0 {
                acc += separation(&items[layer[k - 1]], &items[i]);
            }
            y[i] = acc;
        }
        for &i in layer {
            y[i] -= acc / 2.0;
        }
    }
    for iter in 0..Y_ITERATIONS {
        let down = iter % 2 == 0;
        let layers: Vec<&Vec<usize>> = if down { order.iter().collect() } else { order.iter().rev().collect() };
        for layer in layers {
            let desired: Vec<f32> = layer
                .iter()
                .map(|&i| {
                    let primary = if down { &preds[i] } else { &succs[i] };
                    let ns = if primary.is_empty() { if down { &succs[i] } else { &preds[i] } } else { primary };
                    if ns.is_empty() { y[i] } else { ns.iter().map(|&j| y[j]).sum::<f32>() / ns.len() as f32 }
                })
                .collect();
            let seps: Vec<f32> = (1..layer.len()).map(|k| separation(&items[layer[k - 1]], &items[layer[k]])).collect();
            for (k, v) in place(&desired, &seps).into_iter().enumerate() {
                if let Some(&i) = layer.get(k) {
                    y[i] = v;
                }
            }
        }
    }
    // Centre the drawing vertically and snap to whole pixels.
    let (lo, hi) = y.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    let mid = if lo <= hi { (lo + hi) / 2.0 } else { 0.0 };
    y.iter().map(|v| (v - mid).round()).collect()
}

/// Positions closest (least squares) to `desired` such that `p[k] - p[k-1] >= seps[k-1]`.
/// Substituting `z = p - offset` turns this into isotonic regression, solved by PAV.
pub(crate) fn place(desired: &[f32], seps: &[f32]) -> Vec<f32> {
    let mut offset = Vec::with_capacity(desired.len());
    let mut acc = 0.0;
    for k in 0..desired.len() {
        if k > 0 {
            acc += seps.get(k - 1).copied().unwrap_or(0.0);
        }
        offset.push(acc);
    }
    // Blocks of (sum, count) with non-decreasing means.
    let mut blocks: Vec<(f32, usize)> = Vec::new();
    for (d, o) in desired.iter().zip(&offset) {
        blocks.push((d - o, 1));
        while blocks.len() >= 2 {
            let (s2, c2) = blocks[blocks.len() - 1];
            let (s1, c1) = blocks[blocks.len() - 2];
            if s1 / c1 as f32 <= s2 / c2 as f32 {
                break;
            }
            blocks.pop();
            if let Some(last) = blocks.last_mut() {
                *last = (s1 + s2, c1 + c2);
            }
        }
    }
    let mut out = Vec::with_capacity(desired.len());
    for (s, c) in blocks {
        let mean = s / c as f32;
        out.extend(std::iter::repeat_n(mean, c));
    }
    out.iter().zip(&offset).map(|(z, o)| z + o).collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn ids(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn place_keeps_order_and_separation_closest_to_the_wish() {
        let p = place(&[0.0, 0.0, 0.0], &[10.0, 10.0]);
        assert_eq!(p, vec![-10.0, 0.0, 10.0]);
        let p = place(&[0.0, 100.0], &[10.0]);
        assert_eq!(p, vec![0.0, 100.0]);
    }

    #[test]
    fn long_edges_get_corridor_waypoints() {
        // a -> b -> c and a -> c: the long edge crosses b's column through a corridor.
        let mut l = Layout::default();
        l.compute(&ids(&["a", "b", "c"]), &[(0, 1), (1, 2), (0, 2)]);
        let w = l.waypoints(2);
        assert_eq!(w.len(), 1);
        let (bx, by) = l.position("b").unwrap();
        assert_eq!(w[0].0, bx);
        assert!((w[0].1 - by).abs() >= CARD_TO_CORRIDOR - 0.5, "corridor {w:?} vs b at {by}");
        assert!(l.waypoints(0).is_empty());
    }

    #[test]
    fn barycenter_removes_avoidable_crossings() {
        // Two parallel chains named so the initial (ID) order crosses them.
        let names = ids(&["a1", "a2", "b1", "b2"]);
        // a1 -> b2, a2 -> b1: sorted by ID the links cross; reordering removes it.
        let mut l = Layout::default();
        l.compute(&names, &[(0, 3), (1, 2)]);
        let y = |id: &str| l.position(id).unwrap().1;
        assert_eq!(y("a1") < y("a2"), y("b2") < y("b1"));
    }

    /// The demo topology: 7 services and Postgres.
    fn demo() -> (Vec<String>, Vec<(u32, u32)>) {
        let names =
            ids(&["gateway", "auth", "catalog", "orders", "inventory", "payments", "notifications", "postgres"]);
        let e = vec![(0, 1), (0, 2), (0, 3), (2, 4), (3, 4), (3, 5), (3, 6), (3, 7), (4, 7)];
        (names, e)
    }

    /// Segments of every edge: source → corridor points → target, as (x1, y1, x2, y2).
    fn segments(l: &Layout, names: &[String], edges: &[(u32, u32)]) -> Vec<(usize, (f32, f32, f32, f32))> {
        let mut out = Vec::new();
        for (ei, &(s, t)) in edges.iter().enumerate() {
            let mut pts = vec![l.position(&names[s as usize]).unwrap()];
            pts.extend_from_slice(l.waypoints(ei));
            pts.push(l.position(&names[t as usize]).unwrap());
            for w in pts.windows(2) {
                out.push((ei, (w[0].0, w[0].1, w[1].0, w[1].1)));
            }
        }
        out
    }

    #[test]
    fn demo_topology_has_no_crossings_and_corridors_clear_the_cards() {
        let (names, edges) = demo();
        let mut l = Layout::default();
        l.compute(&names, &edges);
        let segs = segments(&l, &names, &edges);
        // Links only join adjacent columns; two links in the same gap cross when their ends swap order.
        for (i, (ea, a)) in segs.iter().enumerate() {
            for (eb, b) in &segs[i + 1..] {
                let same_gap = a.0 == b.0 && a.2 == b.2;
                let share_end = (a.0, a.1) == (b.0, b.1) || (a.2, a.3) == (b.2, b.3);
                if ea != eb && same_gap && !share_end {
                    assert!((a.1 - b.1) * (a.3 - b.3) >= 0.0, "edges {ea} and {eb} cross: {a:?} {b:?}");
                }
            }
        }
        // Corridor points keep clear of every card in their column.
        for (ei, _) in edges.iter().enumerate() {
            for &(wx, wy) in l.waypoints(ei) {
                for n in &names {
                    let (x, y) = l.position(n).unwrap();
                    if x == wx {
                        assert!((y - wy).abs() >= CARD_TO_CORRIDOR - 0.5, "edge {ei} corridor at {wy} hits {n} at {y}");
                    }
                }
            }
        }
        // Cards in one column never overlap.
        for a in &names {
            for b in &names {
                let (pa, pb) = (l.position(a).unwrap(), l.position(b).unwrap());
                if a != b && pa.0 == pb.0 {
                    assert!((pa.1 - pb.1).abs() >= ROW_SPACING - 0.5, "{a} and {b} overlap");
                }
            }
        }
    }

    proptest::proptest! {
        #[test]
        fn random_graphs_keep_cards_and_corridors_apart(
            n in 1usize..14,
            raw in proptest::collection::vec((0u32..14, 0u32..14), 0..30),
        ) {
            let names: Vec<String> = (0..n).map(|i| format!("n{i:02}")).collect();
            let edges: Vec<(u32, u32)> = raw.into_iter().filter(|(s, t)| (*s as usize) < n && (*t as usize) < n).collect();
            let mut l = Layout::default();
            l.compute(&names, &edges);
            for (ei, _) in edges.iter().enumerate() {
                for &(wx, wy) in l.waypoints(ei) {
                    for id in &names {
                        let (x, y) = l.position(id).unwrap();
                        proptest::prop_assert!(x != wx || (y - wy).abs() >= CARD_TO_CORRIDOR - 0.5);
                    }
                }
            }
            for a in &names {
                for b in &names {
                    let (pa, pb) = (l.position(a).unwrap(), l.position(b).unwrap());
                    proptest::prop_assert!(a == b || pa.0 != pb.0 || (pa.1 - pb.1).abs() >= ROW_SPACING - 0.5);
                }
            }
        }
    }

    #[test]
    fn layout_is_deterministic_regardless_of_input_order() {
        let mut a = Layout::default();
        a.compute(&ids(&["x", "y", "z"]), &[(0, 1), (0, 2)]);
        let mut b = Layout::default();
        b.compute(&ids(&["z", "x", "y"]), &[(1, 2), (1, 0)]);
        for id in ["x", "y", "z"] {
            assert_eq!(a.position(id), b.position(id), "{id}");
        }
    }
}
