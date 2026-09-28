//! Stable layered layout.
//!
//! Positions are memoised per entity ID: once a node is placed it never moves
//! because of new events or metric updates. New nodes are appended to the
//! column matching their call depth.

use std::collections::HashMap;

pub(crate) const COLUMN_SPACING: f32 = 300.0;
pub(crate) const ROW_SPACING: f32 = 140.0;

#[derive(Debug, Default, Clone)]
pub(crate) struct Layout {
    positions: HashMap<String, (f32, f32)>,
    column_fill: HashMap<i32, u32>,
}

impl Layout {
    pub fn position(&self, id: &str) -> Option<(f32, f32)> {
        self.positions.get(id).copied()
    }

    /// Place every node in `ids` that has no position yet.
    /// `edges` are `(source_index, target_index)` pairs into `ids`.
    pub fn place_new(&mut self, ids: &[String], edges: &[(u32, u32)]) {
        let n = ids.len();
        if ids.iter().all(|id| self.positions.contains_key(id)) {
            return;
        }
        // Longest-path depth with a bounded relaxation (cycle-safe).
        let mut depth = vec![0i32; n];
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
        // Deterministic placement order: by depth then ID.
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&a, &b| depth[a].cmp(&depth[b]).then(ids[a].cmp(&ids[b])));
        for i in order {
            let id = &ids[i];
            if self.positions.contains_key(id) {
                continue;
            }
            let col = depth[i];
            let slot = self.column_fill.entry(col).or_insert(0);
            // Alternate above/below the axis so columns stay centred.
            let k = *slot as i32;
            let offset = if k % 2 == 0 { k / 2 } else { -(k / 2 + 1) };
            *slot += 1;
            self.positions.insert(id.clone(), (col as f32 * COLUMN_SPACING, offset as f32 * ROW_SPACING));
        }
    }
}
