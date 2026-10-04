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
//! Nodes may be split into groups (the machines of the infrastructure layer). Each group is a
//! horizontal band laid out on its own and placed below every band it shares columns with;
//! columns are shared, so links still join adjacent columns and never cross a card. A long edge
//! runs through the corridors of its target's band. Free groups (no frame) split into column
//! runs that sit level with their neighbours.
//!
//! Metrics and new events never move nodes; only a changed graph does.

use std::collections::HashMap;

pub(crate) const COLUMN_SPACING: f32 = 300.0;
/// Centre distance between two cards in one column (cards are 56 px tall).
pub(crate) const ROW_SPACING: f32 = 120.0;
/// Half the height of a card.
pub(crate) const CARD_HALF_H: f32 = 28.0;
/// Centre distance between a card and a corridor: 28 px half-card plus 36 px clearance.
pub(crate) const CARD_TO_CORRIDOR: f32 = 64.0;
/// Distance between two corridors in one column.
const CORRIDOR_SPACING: f32 = 22.0;
/// Vertical gap between two stacked bands (outside their reserved frame space).
pub(crate) const BAND_GAP: f32 = 48.0;
/// Crossing counting is quadratic in the links of a layer pair: above these sizes the
/// barycenter sweeps (and the costlier adjacent swaps) are skipped and the ID order is kept.
const MAX_ORDERED_ITEMS: usize = 400;
const MAX_TRANSPOSED_ITEMS: usize = 300;
const SWEEPS: usize = 24;
const Y_ITERATIONS: usize = 24;

#[derive(Debug, Default, Clone)]
pub(crate) struct Layout {
    positions: HashMap<String, (f32, f32)>,
    /// Corridor points of each edge, indexed like the `edges` passed to [`Layout::compute`].
    waypoints: Vec<Vec<(f32, f32)>>,
    /// Vertical extent of each group, indexed like the `specs` passed to [`Layout::compute_banded`].
    bands: Vec<Band>,
}

/// Where one group ended up: `top..bottom` includes the reserved space, `content_top..content_bottom`
/// holds its cards (with their full height) and corridors.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub(crate) struct Band {
    pub top: f32,
    pub content_top: f32,
    pub content_bottom: f32,
    pub bottom: f32,
    /// First and last column the band covers (its items and [`BandSpec::columns`]).
    pub columns: (usize, usize),
    /// No card or corridor in the group.
    pub empty: bool,
}

/// How one group is laid out by [`Layout::compute_banded`].
#[derive(Debug, Default, Clone)]
pub(crate) struct BandSpec {
    /// Space kept free above and below the content, e.g. for a frame title and replica chips.
    pub top: f32,
    pub bottom: f32,
    /// Columns covered besides those of the group's items (e.g. replica chips).
    pub columns: Vec<usize>,
    /// No frame: the group splits into runs of adjacent columns, and each run sits level with
    /// its neighbours wherever no band covering those columns is in the way.
    pub free: bool,
}

/// One independently laid out part of a group: the whole group, or a column run of a free one.
struct Unit {
    group: usize,
    free: bool,
    columns: Option<(usize, usize)>,
    members: Vec<usize>,
    /// Relative y of the members and their extent (cards with their full height).
    y: Vec<f32>,
    extent: Option<(f32, f32)>,
}

impl Unit {
    /// Two units may not share rows: same columns, or adjacent columns of two frames (a frame
    /// reaches past its outer cards).
    fn conflicts(&self, other: &Unit) -> bool {
        let (Some(a), Some(b)) = (self.columns, other.columns) else { return false };
        let reach = usize::from(!self.free && !other.free);
        a.0 <= b.1 + reach && b.0 <= a.1 + reach
    }
}

/// A real node or one corridor slot of a long edge.
#[derive(Debug, Clone)]
struct Item {
    real: bool,
    layer: usize,
    group: usize,
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

    pub fn band(&self, group: usize) -> Option<Band> {
        self.bands.get(group).copied()
    }

    /// Lay out `ids`; `edges` are `(source_index, target_index)` pairs into `ids`.
    pub fn compute(&mut self, ids: &[String], edges: &[(u32, u32)]) {
        self.compute_banded(ids, edges, &[], &[]);
    }

    /// Like [`Layout::compute`], with node `i` in group `group[i]` (missing or out of range: the
    /// last group), laid out as `specs[g]` says. Framed groups are placed in the order of their
    /// first column, then their index, each below every placed band it shares columns with;
    /// free groups follow (see [`BandSpec::free`]). With at most one group this is the plain layout.
    pub fn compute_banded(&mut self, ids: &[String], edges: &[(u32, u32)], group: &[usize], specs: &[BandSpec]) {
        let n = ids.len();
        let depth = layers(n, edges);
        let groups = specs.len().max(1);
        let group_of = |i: usize| group.get(i).copied().filter(|&g| g < groups).unwrap_or(groups - 1);
        let free = |g: usize| specs.get(g).is_some_and(|s| s.free);

        let mut items: Vec<Item> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| Item { real: true, layer: depth[i], group: group_of(i), key: id.clone() })
            .collect();
        // Links between adjacent layers, and the corridor items of each edge.
        let mut links: Vec<(usize, usize)> = Vec::new();
        let mut corridors: Vec<Vec<usize>> = vec![Vec::new(); edges.len()];
        for (ei, &(s, t)) in edges.iter().enumerate() {
            let (s, t) = (s as usize, t as usize);
            if s >= n || t >= n || depth[t] <= depth[s] {
                continue;
            }
            let mut prev = s;
            // Corridors run through the target's band, or the source's when only that has a frame.
            let g = if free(group_of(t)) && !free(group_of(s)) { group_of(s) } else { group_of(t) };
            for layer in depth[s] + 1..depth[t] {
                let key = format!("{}\0{}\0{ei:08}", ids[s], ids[t]);
                items.push(Item { real: false, layer, group: g, key });
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

        // Units: every framed group whole, every free group split into runs of adjacent columns.
        let mut units: Vec<Unit> = Vec::new();
        for g in 0..groups {
            let mut cols: Vec<usize> = items.iter().filter(|it| it.group == g).map(|it| it.layer).collect();
            if free(g) {
                cols.sort_unstable();
                cols.dedup();
                let mut runs: Vec<(usize, usize)> = Vec::new();
                for c in cols {
                    match runs.last_mut() {
                        Some(r) if c <= r.1 + 1 => r.1 = c,
                        _ => runs.push((c, c)),
                    }
                }
                for r in runs {
                    units.push(Unit {
                        group: g,
                        free: true,
                        columns: Some(r),
                        members: Vec::new(),
                        y: Vec::new(),
                        extent: None,
                    });
                }
            } else {
                cols.extend(specs.get(g).map_or(&[][..], |s| s.columns.as_slice()));
                let columns = cols.iter().min().zip(cols.iter().max()).map(|(&a, &b)| (a, b));
                units.push(Unit { group: g, free: false, columns, members: Vec::new(), y: Vec::new(), extent: None });
            }
        }
        let unit_of: Vec<usize> = items
            .iter()
            .map(|it| {
                units
                    .iter()
                    .position(|u| {
                        u.group == it.group
                            && (!u.free || u.columns.is_some_and(|c| c.0 <= it.layer && it.layer <= c.1))
                    })
                    .unwrap_or(0)
            })
            .collect();
        let half = |i: usize| if items[i].real { CARD_HALF_H } else { 0.0 };
        for (u, unit) in units.iter_mut().enumerate() {
            let mut sub: Vec<Vec<usize>> =
                order.iter().map(|l| l.iter().copied().filter(|&i| unit_of[i] == u).collect()).collect();
            let size: usize = sub.iter().map(Vec::len).sum();
            let unit_links: Vec<(usize, usize)> =
                links.iter().copied().filter(|&(a, b)| unit_of[a] == u && unit_of[b] == u).collect();
            let (preds, succs) = adjacency(items.len(), &unit_links);
            if size <= MAX_ORDERED_ITEMS {
                minimise_crossings(&mut sub, &preds, &succs, size);
            }
            unit.y = coordinates(&sub, &items, &preds, &succs);
            unit.members = sub.into_iter().flatten().collect();
            let lo = unit.members.iter().map(|&i| unit.y[i] - half(i)).reduce(f32::min);
            let hi = unit.members.iter().map(|&i| unit.y[i] + half(i)).reduce(f32::max);
            unit.extent = lo.zip(hi);
        }

        let reserve = |g: usize| specs.get(g).map_or((0.0, 0.0), |s| (s.top, s.bottom));
        // Framed units with content by first column, then free ones, then empty frames.
        let mut sequence: Vec<usize> = (0..units.len()).collect();
        sequence.sort_by_key(|&u| {
            let unit = &units[u];
            let rank = match (unit.free, unit.extent.is_some()) {
                (false, true) => 0,
                (true, _) => 1,
                (false, false) => 2,
            };
            (rank, unit.columns.map_or(usize::MAX, |c| c.0), unit.group)
        });
        let mut ys = vec![0.0f32; items.len()];
        // Per unit: (top, content_top, content_bottom, bottom) once placed.
        let mut placed: Vec<Option<(f32, f32, f32, f32)>> = vec![None; units.len()];
        for u in sequence {
            let unit = &units[u];
            let (top_res, bottom_res) = reserve(unit.group);
            let below = |only_conflicting: bool| {
                placed
                    .iter()
                    .enumerate()
                    .filter(|&(p, b)| b.is_some() && (!only_conflicting || unit.conflicts(&units[p])))
                    .filter_map(|(_, b)| b.map(|b| b.3 + BAND_GAP))
                    .reduce(f32::max)
            };
            let Some((lo, hi)) = unit.extent else {
                let top = below(false).unwrap_or(0.0);
                placed[u] = Some((top, top + top_res, top + top_res, top + top_res + bottom_res));
                continue;
            };
            let shift = (if units.len() == 1 {
                // A single band keeps the plain layout, centred on 0.
                0.0
            } else if !unit.free {
                below(true).unwrap_or(0.0) + top_res - lo
            } else {
                // Level with the placed neighbours, else below what shares its columns.
                let mut diffs = Vec::new();
                for &(a, b) in &links {
                    let (mine, other) = if unit_of[a] == u {
                        (a, b)
                    } else if unit_of[b] == u {
                        (b, a)
                    } else {
                        continue;
                    };
                    if unit_of[other] != u && placed.get(unit_of[other]).is_some_and(Option::is_some) {
                        diffs.push(ys[other] - unit.y[mine]);
                    }
                }
                let wish = if diffs.is_empty() {
                    below(true).unwrap_or(0.0) - lo
                } else {
                    diffs.iter().sum::<f32>() / diffs.len() as f32
                };
                let blocked: Vec<(f32, f32)> = placed
                    .iter()
                    .enumerate()
                    .filter(|&(p, _)| unit.conflicts(&units[p]))
                    .filter_map(|(_, b)| b.map(|b| (b.0 - BAND_GAP - hi, b.3 + BAND_GAP - lo)))
                    .collect();
                nearest_outside(wish, blocked)
            })
            .round();
            for &i in &unit.members {
                ys[i] = unit.y[i] + shift;
            }
            let (content_top, content_bottom) = (lo + shift, hi + shift);
            placed[u] = Some((content_top - top_res, content_top, content_bottom, content_bottom + bottom_res));
        }

        self.bands = (0..groups)
            .map(|g| {
                let mine: Vec<usize> = (0..units.len()).filter(|&u| units[u].group == g).collect();
                let columns = mine.iter().filter_map(|&u| units[u].columns).reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)));
                let parts: Vec<(f32, f32, f32, f32)> = mine.iter().filter_map(|&u| placed[u]).collect();
                let Some(&first) = parts.first() else { return Band { empty: true, ..Band::default() } };
                let (top, content_top, content_bottom, bottom) =
                    parts.iter().fold(first, |a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)));
                Band {
                    top,
                    content_top,
                    content_bottom,
                    bottom,
                    columns: columns.unwrap_or((0, 0)),
                    empty: mine.iter().all(|&u| units[u].extent.is_none()),
                }
            })
            .collect();

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

/// The value closest to `wish` outside every open interval of `blocked`.
fn nearest_outside(wish: f32, mut blocked: Vec<(f32, f32)>) -> f32 {
    blocked.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f32, f32)> = Vec::new();
    for (s, e) in blocked {
        match merged.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }
    match merged.iter().find(|&&(s, e)| s < wish && wish < e) {
        Some(&(s, e)) if wish - s <= e - wish => s,
        Some(&(_, e)) => e,
        None => wish,
    }
}

/// Longest-path layer of each node, with a bounded relaxation (cycle-safe).
pub(crate) fn layers(n: usize, edges: &[(u32, u32)]) -> Vec<usize> {
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

/// `size` is the number of items in `order` (the vectors are indexed by global item).
fn minimise_crossings(order: &mut [Vec<usize>], preds: &[Vec<usize>], succs: &[Vec<usize>], size: usize) {
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
        if size <= MAX_TRANSPOSED_ITEMS {
            transpose(order, preds, succs);
        }
        let c = crossings(order, succs);
        if c < best_cross {
            best_cross = c;
            best = order.to_vec();
        }
    }
    order.clone_from_slice(&best);
}

/// Crossings among the links that touch `a` or `b` (two neighbours of one layer) and run
/// to `neighbours`, assuming `a` sits above `b`.
fn pair_crossings(a: usize, b: usize, neighbours: &[Vec<usize>], pos: &[usize]) -> usize {
    let na = neighbours.get(a).map_or(&[][..], Vec::as_slice);
    let nb = neighbours.get(b).map_or(&[][..], Vec::as_slice);
    na.iter().map(|&x| nb.iter().filter(|&&y| pos[y] < pos[x]).count()).sum()
}

/// Swaps neighbours inside a layer while that strictly reduces the total crossings. Swapping
/// two neighbours only changes the crossings between their own links, so each test is local.
fn transpose(order: &mut [Vec<usize>], preds: &[Vec<usize>], succs: &[Vec<usize>]) {
    let mut pos = positions_of(order, preds.len());
    for _ in 0..8 {
        let mut improved = false;
        for layer in order.iter_mut() {
            for k in 0..layer.len().saturating_sub(1) {
                let (a, b) = (layer[k], layer[k + 1]);
                let before = pair_crossings(a, b, preds, &pos) + pair_crossings(a, b, succs, &pos);
                let after = pair_crossings(b, a, preds, &pos) + pair_crossings(b, a, succs, &pos);
                if after < before {
                    layer.swap(k, k + 1);
                    pos[a] = k + 1;
                    pos[b] = k;
                    improved = true;
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
    let (lo, hi) = order.iter().flatten().fold((f32::MAX, f32::MIN), |(lo, hi), &i| (lo.min(y[i]), hi.max(y[i])));
    let mid = if lo <= hi { (lo + hi) / 2.0 } else { 0.0 };
    let mut y: Vec<f32> = y.iter().map(|v| (v - mid).round()).collect();
    // Rounding two neighbours half a pixel apart in opposite directions loses a pixel of
    // separation; push the lower one back down (separations are whole pixels).
    for layer in order {
        for k in 1..layer.len() {
            let (a, b) = (layer[k - 1], layer[k]);
            let min = y[a] + separation(&items[a], &items[b]);
            if y[b] < min {
                y[b] = min;
            }
        }
    }
    y
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

    #[test]
    fn corridors_keep_clear_of_cards_after_snapping_to_pixels() {
        // Minimal case found by the property test below.
        let names: Vec<String> = (0..9).map(|i| format!("n{i:02}")).collect();
        let edges = [(0, 8), (7, 0), (4, 6), (1, 5), (3, 2), (2, 4), (6, 5), (5, 8), (1, 4)];
        let mut l = Layout::default();
        l.compute(&names, &edges);
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

    fn framed(top: f32, bottom: f32) -> BandSpec {
        BandSpec { top, bottom, ..BandSpec::default() }
    }

    fn free_band() -> BandSpec {
        BandSpec { free: true, ..BandSpec::default() }
    }

    /// Corridors clear every card in their column, cards in one column never overlap, every
    /// group's cards sit inside its band, framed bands that could touch never share rows and
    /// no card of a free group lies in the rows of a framed band covering its column.
    fn assert_banded_invariants(
        l: &Layout,
        names: &[String],
        edges: &[(u32, u32)],
        group: &[usize],
        specs: &[BandSpec],
    ) {
        let groups = specs.len();
        for (ei, _) in edges.iter().enumerate() {
            for &(wx, wy) in l.waypoints(ei) {
                for id in names {
                    let (x, y) = l.position(id).unwrap();
                    assert!(
                        x != wx || (y - wy).abs() >= CARD_TO_CORRIDOR - 0.5,
                        "edge {ei} corridor at {wy} hits {id} at {y}"
                    );
                }
            }
        }
        for a in names {
            for b in names {
                let (pa, pb) = (l.position(a).unwrap(), l.position(b).unwrap());
                assert!(a == b || pa.0 != pb.0 || (pa.1 - pb.1).abs() >= ROW_SPACING - 0.5, "{a} and {b} overlap");
            }
        }
        for (i, id) in names.iter().enumerate() {
            let b = l.band(group[i].min(groups - 1)).unwrap();
            let (_, y) = l.position(id).unwrap();
            assert!(
                y - CARD_HALF_H >= b.content_top - 0.5 && y + CARD_HALF_H <= b.content_bottom + 0.5,
                "{id} outside its band"
            );
        }
        let frames: Vec<Band> =
            (0..groups).filter(|&g| !specs[g].free).filter_map(|g| l.band(g)).filter(|b| !b.empty).collect();
        for (i, a) in frames.iter().enumerate() {
            for b in &frames[i + 1..] {
                let near = a.columns.0 <= b.columns.1 + 1 && b.columns.0 <= a.columns.1 + 1;
                let apart = a.bottom + BAND_GAP <= b.top + 0.5 || b.bottom + BAND_GAP <= a.top + 0.5;
                assert!(!near || apart, "bands overlap: {a:?} {b:?}");
            }
        }
        for (i, id) in names.iter().enumerate() {
            if !specs[group[i].min(groups - 1)].free {
                continue;
            }
            let (x, y) = l.position(id).unwrap();
            let c = (x / COLUMN_SPACING).round() as usize;
            for f in frames.iter().filter(|f| f.columns.0 <= c && c <= f.columns.1) {
                let apart = y + CARD_HALF_H + BAND_GAP <= f.top + 0.5 || y - CARD_HALF_H >= f.bottom + BAND_GAP - 0.5;
                assert!(apart, "free {id} at {y} lies in {f:?}");
            }
        }
    }

    #[test]
    fn banded_layout_stacks_groups_and_keeps_columns() {
        let (names, edges) = demo();
        // gateway, auth, orders, inventory on machine 0; catalog, payments, notifications on 1; postgres nowhere.
        let group = [0, 0, 1, 0, 0, 1, 1, 2];
        let specs = [framed(44.0, 28.0), framed(44.0, 64.0), framed(0.0, 0.0)];
        let mut l = Layout::default();
        l.compute_banded(&names, &edges, &group, &specs);
        assert_banded_invariants(&l, &names, &edges, &group, &specs);
        // Columns are the same as in the plain layout.
        let mut plain = Layout::default();
        plain.compute(&names, &edges);
        for n in &names {
            assert_eq!(l.position(n).unwrap().0, plain.position(n).unwrap().0, "{n}");
        }
        let b0 = l.band(0).unwrap();
        assert_eq!(b0.content_top - b0.top, 44.0);
        assert_eq!(b0.bottom - b0.content_bottom, 28.0);
        // Long edges run through the corridors of their target's band.
        for (ei, &(_, t)) in edges.iter().enumerate() {
            let b = l.band(group[t as usize]).unwrap();
            for &(_, wy) in l.waypoints(ei) {
                assert!(wy >= b.content_top && wy <= b.content_bottom, "edge {ei} corridor outside its band");
            }
        }
    }

    /// The demo behind an external client, with Postgres: client and Postgres are on no machine.
    fn demo_with_client() -> (Vec<String>, Vec<(u32, u32)>, Vec<usize>) {
        let names = ids(&[
            "client",
            "gateway",
            "auth",
            "catalog",
            "orders",
            "inventory",
            "payments",
            "notifications",
            "postgres",
        ]);
        let edges = vec![(0, 1), (0, 4), (1, 2), (1, 3), (1, 4), (3, 5), (4, 5), (4, 6), (4, 7), (4, 8), (5, 8)];
        (names, edges, vec![2, 0, 0, 1, 0, 0, 1, 1, 2])
    }

    #[test]
    fn free_runs_sit_level_with_their_neighbours_beside_the_frames() {
        let (names, edges, group) = demo_with_client();
        let specs = [framed(44.0, 28.0), framed(44.0, 64.0), free_band()];
        let mut l = Layout::default();
        l.compute_banded(&names, &edges, &group, &specs);
        assert_banded_invariants(&l, &names, &edges, &group, &specs);
        let b0 = l.band(0).unwrap();
        let y = |id: &str| l.position(id).unwrap().1;
        // Every neighbour of the client and of Postgres is in machine 0 (orders → postgres runs
        // through machine 0's corridors, as only its source has a frame).
        for id in ["client", "postgres"] {
            assert!(y(id) >= b0.content_top && y(id) <= b0.content_bottom, "{id} at {} outside {b0:?}", y(id));
        }
        for &(_, wy) in l.waypoints(9) {
            assert!(wy >= b0.content_top && wy <= b0.content_bottom, "orders → postgres corridor outside machine 0");
        }
        // Lower than stacking them as one more band.
        let height = |l: &Layout| {
            let ys: Vec<f32> = names.iter().map(|n| l.position(n).unwrap().1).collect();
            ys.iter().copied().fold(f32::MIN, f32::max) - ys.iter().copied().fold(f32::MAX, f32::min)
        };
        let mut stacked = Layout::default();
        stacked.compute_banded(&names, &edges, &group, &[framed(44.0, 28.0), framed(44.0, 64.0), framed(0.0, 0.0)]);
        assert!(height(&l) + ROW_SPACING <= height(&stacked), "{} vs {}", height(&l), height(&stacked));
    }

    #[test]
    fn frames_on_separate_columns_share_rows() {
        // a -> b -> c -> d with a, b on machine 0 and d on machine 1; c on no machine keeps them apart.
        let names = ids(&["a", "b", "c", "d"]);
        let edges = [(0, 1), (1, 2), (2, 3)];
        let group = [0, 0, 2, 1];
        let specs = [framed(44.0, 28.0), framed(44.0, 28.0), free_band()];
        let mut l = Layout::default();
        l.compute_banded(&names, &edges, &group, &specs);
        assert_banded_invariants(&l, &names, &edges, &group, &specs);
        let (b0, b1) = (l.band(0).unwrap(), l.band(1).unwrap());
        assert_eq!((b0.columns, b1.columns), ((0, 1), (3, 3)));
        assert_eq!(b0.top, b1.top);
        // Next-door frames would touch: those stack.
        let mut l = Layout::default();
        l.compute_banded(&names, &edges, &[0, 0, 1, 1], &specs[..2]);
        let (b0, b1) = (l.band(0).unwrap(), l.band(1).unwrap());
        assert!(b0.bottom + BAND_GAP <= b1.top + 0.5);
    }

    #[test]
    fn nearest_outside_picks_the_closer_free_side() {
        assert_eq!(nearest_outside(5.0, vec![]), 5.0);
        assert_eq!(nearest_outside(5.0, vec![(0.0, 20.0)]), 0.0);
        assert_eq!(nearest_outside(15.0, vec![(0.0, 20.0)]), 20.0);
        // Overlapping intervals merge: neither inner end is free.
        assert_eq!(nearest_outside(9.0, vec![(10.0, 30.0), (0.0, 12.0)]), 0.0);
        assert_eq!(nearest_outside(25.0, vec![(10.0, 30.0), (0.0, 12.0)]), 30.0);
        assert_eq!(nearest_outside(0.0, vec![(0.0, 10.0)]), 0.0);
    }

    #[test]
    fn a_single_band_is_the_plain_layout() {
        let (names, edges) = demo();
        let mut a = Layout::default();
        a.compute(&names, &edges);
        let mut b = Layout::default();
        b.compute_banded(&names, &edges, &[0; 8], &[framed(44.0, 28.0)]);
        for n in &names {
            assert_eq!(a.position(n), b.position(n), "{n}");
        }
        assert_eq!(b.band(0).map(|b| b.empty), Some(false));
    }

    #[test]
    fn empty_bands_take_no_room_and_are_marked() {
        let (names, edges) = demo();
        let mut l = Layout::default();
        l.compute_banded(&names, &edges, &[0; 8], &[framed(10.0, 10.0), framed(44.0, 28.0)]);
        let b = l.band(1).unwrap();
        assert!(b.empty);
        assert_eq!(b.content_bottom, b.content_top);
        assert!(b.top >= l.band(0).unwrap().bottom + BAND_GAP - 0.5);
    }

    proptest::proptest! {
        #[test]
        fn random_banded_graphs_keep_cards_corridors_and_bands_apart(
            n in 1usize..14,
            raw in proptest::collection::vec((0u32..14, 0u32..14), 0..30),
            groups in 1usize..4,
            raw_group in proptest::collection::vec(0usize..4, 14),
            last_free in proptest::bool::ANY,
        ) {
            let names: Vec<String> = (0..n).map(|i| format!("n{i:02}")).collect();
            let edges: Vec<(u32, u32)> = raw.into_iter().filter(|(s, t)| (*s as usize) < n && (*t as usize) < n).collect();
            let group: Vec<usize> = raw_group[..n].iter().map(|g| g % groups).collect();
            let specs: Vec<BandSpec> = (0..groups)
                .map(|g| if last_free && g + 1 == groups { free_band() } else { framed(44.0, 28.0 + g as f32 * 36.0) })
                .collect();
            let mut l = Layout::default();
            l.compute_banded(&names, &edges, &group, &specs);
            assert_banded_invariants(&l, &names, &edges, &group, &specs);
            let mut again = Layout::default();
            again.compute_banded(&names, &edges, &group, &specs);
            for id in &names {
                proptest::prop_assert_eq!(l.position(id), again.position(id));
            }
        }
    }

    #[test]
    fn local_transpose_finds_the_same_crossing_free_order() {
        // Three parallel chains in a scrambled ID order: the swaps must untangle them all.
        let names = ids(&["a1", "a2", "a3", "b3", "b1", "b2"]);
        let edges = [(0, 4), (1, 5), (2, 3)];
        let mut l = Layout::default();
        l.compute(&names, &edges);
        let y = |id: &str| l.position(id).unwrap().1;
        let mut left = [("a1", "b1"), ("a2", "b2"), ("a3", "b3")];
        left.sort_by(|a, b| y(a.0).total_cmp(&y(b.0)));
        assert!(left.windows(2).all(|w| y(w[0].1) < y(w[1].1)), "chains cross");
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
