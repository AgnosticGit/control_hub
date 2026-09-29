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
                durations[b].push((s.end_ns.saturating_sub(s.start_ns) as f64 / 1e6) as f32);
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::too_many_arguments)]

    use std::collections::BTreeMap;

    use super::*;
    use crate::types::{Batch, EngineConfig, EventItem, Filters, LogItem, TimelineItem};
    use proptest::prelude::*;

    const MS: i64 = 1_000_000;

    fn span(
        trace: &str,
        id: &str,
        parent: Option<&str>,
        svc: &str,
        kind: u8,
        start: i64,
        end: i64,
        err: bool,
    ) -> SpanItem {
        SpanItem {
            trace_id: trace.into(),
            span_id: id.into(),
            parent_span_id: parent.map(Into::into),
            service_id: format!("svc_{svc}"),
            service_name: svc.into(),
            target_entity_id: None,
            name: format!("{svc} {id}"),
            kind,
            start_ns: start,
            end_ns: end,
            is_error: err,
        }
    }

    fn engine(items: Vec<TimelineItem>) -> Engine {
        let mut e = Engine::new(EngineConfig::default());
        e.apply_batch(Batch { items }).unwrap();
        e
    }

    fn naive_p95(mut v: Vec<f32>) -> Option<f32> {
        if v.is_empty() {
            return None;
        }
        v.sort_by(f32::total_cmp);
        let rank = (v.len() * 95).div_ceil(100);
        Some(v[rank - 1])
    }

    #[test]
    fn p95_small_inputs() {
        assert_eq!(p95(&mut []), None);
        assert_eq!(p95(&mut [4.0]), Some(4.0));
        assert_eq!(p95(&mut [3.0, 1.0]), Some(3.0));
        assert_eq!(p95(&mut [f32::INFINITY, 1.0, 2.0]), Some(f32::INFINITY));
        // `len * 0.95` in floating point must never round to the wrong rank.
        for len in 1..=3_000usize {
            let mut v: Vec<f32> = (1..=len).rev().map(|i| i as f32).collect();
            assert_eq!(p95(&mut v), Some(((len * 95).div_ceil(100)) as f32), "len {len}");
        }
    }

    #[test]
    fn bucket_and_clip_limits_are_clamped() {
        let e = engine(vec![TimelineItem::Span(span("t", "a", None, "api", 2, 0, MS, false))]);
        let t = e.timeline_tracks(0, 10 * MS, 0, u32::MAX);
        assert_eq!((t.buckets, t.requests.len()), (1, 1));
        let t = e.timeline_tracks(0, 10 * MS, u32::MAX, 0);
        assert_eq!((t.buckets, t.requests.len()), (MAX_TRACK_BUCKETS, MAX_TRACK_BUCKETS as usize));
        assert!(t.clips.is_empty() && !t.services[0].clipped, "clip cap 0 returns no clips");
        // Reversed windows are empty but keep their bounds.
        let t = e.timeline_tracks(10 * MS, 0, 5, 5);
        assert_eq!((t.from_ns, t.to_ns, t.requests.len()), (10 * MS, 0, 0));
    }

    #[test]
    fn window_is_half_open() {
        let e = engine(vec![
            TimelineItem::Span(span("t", "a", None, "api", 2, 0, 5 * MS, false)),
            TimelineItem::Span(span("t", "b", None, "api", 2, 10 * MS, 12 * MS, false)),
        ]);
        let t = e.timeline_tracks(0, 10 * MS, 2, 10);
        assert_eq!(t.requests, vec![1, 0], "a span starting at `to` is outside");
        assert_eq!(t.latency_samples, vec![0, 1], "a span ending at 5 ms falls in the second half");
        // The span ending exactly at `from` still overlaps the window.
        let t = e.timeline_tracks(5 * MS, 10 * MS, 1, 10);
        assert_eq!(t.services[0].spans_in_view, 1);
    }

    #[test]
    fn markers_are_capped_sorted_and_counted() {
        let mut items: Vec<TimelineItem> = (0..(MAX_MARKERS as i64 + 50))
            .rev()
            .map(|i| {
                TimelineItem::Event(EventItem {
                    id: format!("e{i}"),
                    event_type: "deploy".into(),
                    service_id: None,
                    service_name: None,
                    time_ns: i * MS,
                    trace_id: None,
                })
            })
            .collect();
        items.push(TimelineItem::Event(EventItem {
            id: "r".into(),
            event_type: "request.started".into(),
            service_id: None,
            service_name: None,
            time_ns: 0,
            trace_id: None,
        }));
        let e = engine(items);
        let t = e.timeline_tracks(0, 1_000 * MS, 4, 0);
        assert_eq!(t.markers_total, MAX_MARKERS as u32 + 50);
        assert_eq!(t.markers.len(), MAX_MARKERS);
        assert!(t.markers.windows(2).all(|w| w[0].time_ms <= w[1].time_ms));
        assert_eq!(t.markers[0].time_ms, 0.0);
        assert_eq!(t.events.iter().sum::<u32>(), MAX_MARKERS as u32 + 51, "request events count, no marker");
    }

    #[test]
    fn hidden_and_muted_services_leave_logs_and_events_out() {
        let log = |id: &str, svc: &str| {
            TimelineItem::Log(LogItem {
                id: id.into(),
                service_id: format!("svc_{svc}"),
                service_name: svc.into(),
                time_ns: MS,
                severity: 17,
                trace_id: None,
            })
        };
        let ev = |id: &str, svc: Option<&str>| {
            TimelineItem::Event(EventItem {
                id: id.into(),
                event_type: "deploy".into(),
                service_id: svc.map(|s| format!("svc_{s}")),
                service_name: svc.map(Into::into),
                time_ns: MS,
                trace_id: None,
            })
        };
        let mut e = engine(vec![log("1", "a"), log("2", "b"), ev("3", Some("a")), ev("4", Some("b")), ev("5", None)]);
        let t = e.timeline_tracks(0, 10 * MS, 1, 0);
        assert_eq!((t.errors[0], t.events[0], t.markers_total), (2, 3, 3));
        e.set_filters(Filters { search: "a".into(), ..Filters::default() });
        let t = e.timeline_tracks(0, 10 * MS, 1, 0);
        assert_eq!((t.errors[0], t.events[0], t.markers_total), (1, 2, 2), "unattributed events always count");
        e.set_filters(Filters { muted_services: vec!["svc_a".into()], ..Filters::default() });
        let t = e.timeline_tracks(0, 10 * MS, 1, 0);
        assert_eq!((t.errors[0], t.events[0]), (1, 2));
        assert_eq!(t.services.len(), 2, "muted services keep their row");
    }

    #[test]
    fn depth_counts_same_service_ancestors_and_is_bounded() {
        let mut items = vec![TimelineItem::Span(span("t", "s0", None, "api", 2, 0, 100 * MS, false))];
        for i in 1..20 {
            let parent = format!("s{}", i - 1);
            items.push(TimelineItem::Span(span(
                "t",
                &format!("s{i}"),
                Some(&parent),
                "api",
                1,
                i * MS,
                100 * MS,
                false,
            )));
        }
        // A cycle must not loop.
        items.push(TimelineItem::Span(span("c", "x", Some("y"), "api", 1, 0, MS, false)));
        items.push(TimelineItem::Span(span("c", "y", Some("x"), "api", 1, 0, MS, false)));
        let e = engine(items);
        let t = e.timeline_tracks(0, 200 * MS, 1, 100);
        let depth = |name: &str| t.clips.iter().find(|c| c.name == name).unwrap().depth;
        assert_eq!((depth("api s0"), depth("api s3"), depth("api s19")), (0, 3, MAX_DEPTH));
        assert_eq!(depth("api x"), MAX_DEPTH);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(96))]

        #[test]
        fn p95_is_nearest_rank(mut v in prop::collection::vec(-1e6f32..1e6, 0..300)) {
            let want = naive_p95(v.clone());
            prop_assert_eq!(p95(&mut v), want);
        }

        /// Buckets, request counts, error counts, latency samples and p95 against a naive scan.
        #[test]
        fn aggregates_match_a_naive_scan(
            raw in prop::collection::vec((0u32..3, 0u8..6, any::<bool>(), -50i64..250, 0i64..80, any::<bool>()), 0..80),
            from in -20i64..50,
            len in 1i64..250,
            buckets in 1u32..12,
            muted in any::<bool>(),
        ) {
            let spans: Vec<SpanItem> = raw.iter().enumerate().map(|(i, &(svc, kind, root, start, dur, err))| {
                let parent = (!root).then_some("p");
                span("t", &format!("s{i}"), parent, ["a", "b", "c"][svc as usize], kind, start * MS, (start + dur) * MS, err)
            }).collect();
            let mut e = engine(spans.iter().cloned().map(TimelineItem::Span).collect());
            if muted {
                e.set_filters(Filters { muted_services: vec!["svc_b".into()], ..Filters::default() });
            }
            let (from, to) = (from * MS, (from + len) * MS);
            let t = e.timeline_tracks(from, to, buckets, MAX_CLIP_CAP);
            let n = buckets as usize;
            let bucket = |x: i64| (x >= from && x < to).then(|| ((i128::from(x - from) * n as i128) / i128::from(to - from)) as usize);

            let mut requests = vec![0u32; n];
            let mut errors = vec![0u32; n];
            let mut durations: Vec<Vec<f32>> = vec![Vec::new(); n];
            let mut per_service: BTreeMap<String, (Vec<u32>, u32)> = BTreeMap::new();
            for s in &spans {
                let entry = per_service.entry(s.service_name.clone()).or_insert_with(|| (vec![0; n], 0));
                let request = is_request(s) && !(muted && s.service_id == "svc_b");
                if let Some(b) = bucket(s.start_ns) {
                    entry.0[b] += 1;
                    if request {
                        requests[b] += 1;
                        errors[b] += u32::from(s.is_error);
                    }
                }
                if request && let Some(b) = bucket(s.end_ns) {
                    durations[b].push(((s.end_ns - s.start_ns) as f64 / 1e6) as f32);
                }
                if s.end_ns >= from && s.start_ns < to {
                    entry.1 += 1;
                }
            }
            prop_assert_eq!(&t.requests, &requests);
            prop_assert_eq!(&t.errors, &errors);
            prop_assert_eq!(t.latency_samples.clone(), durations.iter().map(|d| d.len() as u32).collect::<Vec<_>>());
            prop_assert_eq!(t.latency_p95_ms.clone(), durations.into_iter().map(naive_p95).collect::<Vec<_>>());
            for track in &t.services {
                let (counts, in_view) = per_service.get(&track.name).cloned().unwrap_or((vec![0; n], 0));
                prop_assert_eq!(&track.counts, &counts, "counts of {}", track.name);
                prop_assert_eq!(track.spans_in_view, in_view);
                prop_assert!(track.clipped);
                let clips = t.clips.iter().filter(|c| t.services[c.track as usize].name == track.name).count();
                prop_assert_eq!(clips as u32, in_view, "clips of {}", track.name);
            }
            prop_assert!(t.clips.windows(2).all(|w| (w[0].track, w[0].start_ms) <= (w[1].track, w[1].start_ms)));
        }

        /// A service's clips appear only when all of its spans in view fit under the cap.
        #[test]
        fn clip_cap_is_all_or_nothing(count in 0usize..30, cap in 0u32..30) {
            let items = (0..count)
                .map(|i| TimelineItem::Span(span(&format!("t{i}"), "r", None, "api", 2, i as i64 * MS, (i as i64 + 1) * MS, false)))
                .collect();
            let e = engine(items);
            let t = e.timeline_tracks(0, 100 * MS, 10, cap);
            let fits = count <= cap as usize;
            if count > 0 {
                prop_assert_eq!(t.services[0].clipped, fits);
                prop_assert_eq!(t.services[0].spans_in_view as usize, count);
            }
            prop_assert_eq!(t.clips.len(), if fits { count } else { 0 });
        }
    }
}
