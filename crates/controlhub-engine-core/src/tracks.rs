//! Layered timeline tracks: per-bucket aggregates over a time window plus,
//! when few enough, the individual spans of each service as clips.

use std::collections::{HashMap, HashSet};

use controlhub_domain::EntityKind;

use crate::Engine;
use crate::skew::SkewCache;
use crate::types::{EventMarker, ServiceTrack, SpanClip, SpanItem, TimelineTracks};

pub const MAX_TRACK_BUCKETS: u32 = 2_000;
pub const MAX_CLIP_CAP: u32 = 2_000;
const MAX_MARKERS: usize = 300;
const MAX_DEPTH: u8 = 8;

/// A call is counted once, on the callee: server and consumer spans, plus roots.
fn is_request(s: &SpanItem) -> bool {
    s.kind == 2 || s.kind == 5 || s.parent_span_id.is_none()
}

/// Nearest-rank p95.
fn p95(values: &mut [f32]) -> Option<f32> {
    if values.is_empty() {
        return None;
    }
    let k = ((values.len() as f64 * 0.95).ceil() as usize).saturating_sub(1).min(values.len() - 1);
    let (_, v, _) = values.select_nth_unstable_by(k, |a, b| a.total_cmp(b));
    Some(*v)
}

impl Engine {
    /// Aggregates loaded items over `[from_ns, to_ns)` into `buckets` equal buckets. Services
    /// hidden by the current filters are left out of every track. Clips are returned for a
    /// service only when all of its spans in the window fit under `clip_cap`.
    pub fn timeline_tracks(&self, from_ns: i64, to_ns: i64, buckets: u32, clip_cap: u32) -> TimelineTracks {
        let buckets = buckets.clamp(1, MAX_TRACK_BUCKETS);
        let clip_cap = clip_cap.min(MAX_CLIP_CAP) as usize;
        let mut out = TimelineTracks { from_ns, to_ns, buckets, ..Default::default() };
        if to_ns <= from_ns {
            return out;
        }
        let n = buckets as usize;
        let width = i128::from(to_ns) - i128::from(from_ns);
        let bucket = |t: i64| -> Option<usize> {
            if t < from_ns || t >= to_ns {
                return None;
            }
            let b = (i128::from(t) - i128::from(from_ns)) * n as i128 / width;
            usize::try_from(b).ok().filter(|b| *b < n)
        };
        let rel = |t: i64| (i128::from(t) - i128::from(from_ns)) as f64 / 1e6;

        out.requests = vec![0; n];
        out.errors = vec![0; n];
        out.events = vec![0; n];
        let mut durations: Vec<Vec<f32>> = vec![Vec::new(); n];

        let mut track_of: HashMap<u32, usize> = HashMap::new();
        for (i, node) in self.nodes.iter().enumerate() {
            if node.kind == EntityKind::Service && self.passes_filter(i) {
                track_of.insert(i as u32, out.services.len());
                out.services.push(ServiceTrack {
                    node: i as u32,
                    name: node.name.clone(),
                    counts: vec![0; n],
                    ..Default::default()
                });
            }
        }
        let mut in_view: Vec<Vec<&SpanItem>> = vec![Vec::new(); out.services.len()];
        // Muted services keep their own row but are left out of the aggregate rows.
        let muted: HashSet<&str> = self.filters.muted_services.iter().map(String::as_str).collect();

        for s in self.spans.values() {
            let Some(&ti) = self.node_index.get(&s.service_id).and_then(|node| track_of.get(node)) else {
                continue;
            };
            let request = is_request(s) && !muted.contains(s.service_id.as_str());
            if let Some(b) = bucket(s.start_ns) {
                if let Some(c) = out.services.get_mut(ti).and_then(|t| t.counts.get_mut(b)) {
                    *c += 1;
                }
                if request {
                    out.requests[b] += 1;
                    if s.is_error {
                        out.errors[b] += 1;
                    }
                }
            }
            if request && let Some(b) = bucket(s.end_ns) {
                durations[b].push(((s.end_ns - s.start_ns) as f64 / 1e6) as f32);
            }
            if s.end_ns >= from_ns
                && s.start_ns < to_ns
                && let (Some(track), Some(list)) = (out.services.get_mut(ti), in_view.get_mut(ti))
            {
                track.spans_in_view += 1;
                if list.len() <= clip_cap {
                    list.push(s);
                }
            }
        }

        let hidden = |service_id: &str| {
            muted.contains(service_id)
                || self.node_index.get(service_id).is_some_and(|node| !track_of.contains_key(node))
        };
        for l in self.logs.values() {
            if l.severity >= 17
                && !hidden(&l.service_id)
                && let Some(b) = bucket(l.time_ns)
            {
                out.errors[b] += 1;
            }
        }
        for e in self.events.values() {
            if e.service_id.as_deref().is_some_and(hidden) {
                continue;
            }
            let Some(b) = bucket(e.time_ns) else { continue };
            out.events[b] += 1;
            if !e.event_type.starts_with("request.") {
                out.markers_total += 1;
                out.markers.push(EventMarker {
                    time_ms: rel(e.time_ns),
                    event_type: e.event_type.clone(),
                    service_name: e.service_name.clone(),
                });
            }
        }
        out.markers.sort_by(|a, b| a.time_ms.total_cmp(&b.time_ms).then_with(|| a.event_type.cmp(&b.event_type)));
        out.markers.truncate(MAX_MARKERS);

        out.latency_samples = durations.iter().map(|d| d.len() as u32).collect();
        out.latency_p95_ms = durations.iter_mut().map(|d| p95(d)).collect();

        let mut skew = SkewCache::new();
        for (ti, list) in in_view.iter().enumerate() {
            let Some(track) = out.services.get_mut(ti) else { continue };
            if track.spans_in_view as usize > clip_cap {
                continue;
            }
            track.clipped = true;
            for s in list {
                // Clips line up with the request hops on the map (same clock-skew shift).
                let k = self.skew_ns(s, &mut skew);
                out.clips.push(SpanClip {
                    track: ti as u32,
                    start_ms: rel(s.start_ns.saturating_add(k)),
                    end_ms: rel(s.end_ns.saturating_add(k)),
                    depth: self.span_depth(s),
                    error: s.is_error,
                    name: s.name.clone(),
                    trace_id: s.trace_id.clone(),
                });
            }
        }
        out.clips.sort_by(|a, b| {
            a.track
                .cmp(&b.track)
                .then_with(|| a.start_ms.total_cmp(&b.start_ms))
                .then_with(|| a.end_ms.total_cmp(&b.end_ms))
                .then_with(|| a.name.cmp(&b.name))
        });
        out
    }

    /// Ancestors of `s` in the same service, up to the first span of another service.
    fn span_depth(&self, s: &SpanItem) -> u8 {
        let mut depth = 0;
        let mut cur = s;
        while depth < MAX_DEPTH {
            let Some(parent) = cur.parent_span_id.as_deref() else { break };
            let Some(p) = self.spans.get(&(cur.trace_id.clone(), parent.to_owned())) else { break };
            if p.service_id != s.service_id {
                break;
            }
            depth += 1;
            cur = p;
        }
        depth
    }
}
