//! Reproducible engine benchmark on synthetic data (fixed seed, no I/O):
//!   cargo run --release -p controlhub-engine-core --example bench [-- <requests>]
//! Prints wall-clock timings for this machine; numbers are not portable.

use std::time::Instant;

use controlhub_engine_core::{Batch, Engine, EngineConfig, EventItem, LogItem, SpanItem, TimelineItem};

const SERVICES: usize = 20;

/// Small deterministic PRNG (xorshift64*), so runs are comparable.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn synthetic(from: usize, requests: usize) -> Vec<TimelineItem> {
    let mut rng = Rng(0x00C0_FFEE ^ from as u64);
    let base: i64 = 1_700_000_000_000_000_000;
    let mut items = Vec::with_capacity(requests * 4);
    for r in from..from + requests {
        let start = base + (r as i64) * 2_000_000 + rng.below(1_000_000) as i64;
        let caller = rng.below(SERVICES as u64) as usize;
        let callee = (caller + 1 + rng.below(3) as usize) % SERVICES;
        let trace_id = format!("{:032x}", (r as u128) + 1);
        let root = format!("{:016x}", (r as u64) * 2 + 1);
        let child = format!("{:016x}", (r as u64) * 2 + 2);
        let dur = 1_000_000 + rng.below(20_000_000) as i64;
        let err = rng.below(100) < 3;
        items.push(TimelineItem::Span(SpanItem {
            trace_id: trace_id.clone(),
            span_id: root.clone(),
            parent_span_id: None,
            service_id: format!("svc_{caller}"),
            service_name: format!("service-{caller}"),
            target_entity_id: None,
            name: "GET /".into(),
            kind: 2,
            start_ns: start,
            end_ns: start + dur,
            is_error: err,
        }));
        items.push(TimelineItem::Span(SpanItem {
            trace_id: trace_id.clone(),
            span_id: child,
            parent_span_id: Some(root),
            service_id: format!("svc_{callee}"),
            service_name: format!("service-{callee}"),
            target_entity_id: None,
            name: "handle".into(),
            kind: 2,
            start_ns: start + 100_000,
            end_ns: start + dur - 100_000,
            is_error: err,
        }));
        items.push(TimelineItem::Log(LogItem {
            id: format!("log-{r}"),
            service_id: format!("svc_{callee}"),
            service_name: format!("service-{callee}"),
            time_ns: start + dur / 2,
            severity: if err { 17 } else { 9 },
            trace_id: Some(trace_id.clone()),
        }));
        if r % 10 == 0 {
            items.push(TimelineItem::Event(EventItem {
                id: format!("evt-{r}"),
                event_type: "request.completed".into(),
                service_id: Some(format!("svc_{caller}")),
                service_name: Some(format!("service-{caller}")),
                time_ns: start + dur,
                trace_id: Some(trace_id),
            }));
        }
    }
    items
}

fn main() {
    let requests: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(50_000);
    let items = synthetic(0, requests);
    let n_items = items.len();
    let first = 1_700_000_000_000_000_000i64;
    let last = first + (requests as i64) * 2_000_000;

    let mut engine = Engine::new(EngineConfig {
        max_items: u32::try_from(n_items + 1).unwrap_or(u32::MAX),
        ..EngineConfig::default()
    });
    let t = Instant::now();
    if let Err(e) = engine.apply_batch(Batch { items }) {
        eprintln!("apply_batch failed: {e}");
        std::process::exit(1);
    }
    let apply = t.elapsed();

    let t = Instant::now();
    let _ = engine.seek(last, 1);
    let first_frame = engine.visible_state();
    let build = t.elapsed();

    let seeks = 1_000u64;
    let t = Instant::now();
    let mut rng = Rng(42);
    let mut visible = 0usize;
    for g in 0..seeks {
        let at = first + (rng.below((last - first) as u64)) as i64;
        let _ = engine.seek(at, g + 2);
        visible += engine.visible_state().node_indices.len();
    }
    let seek_total = t.elapsed();

    // Live: small batches appended after the recorded range, each followed by a frame.
    let live_batches = 50usize;
    let live_size = 20usize;
    let t = Instant::now();
    for b in 0..live_batches {
        let from = requests + b * live_size;
        if let Err(e) = engine.apply_batch(Batch { items: synthetic(from, live_size) }) {
            eprintln!("apply_batch failed: {e}");
            std::process::exit(1);
        }
        let now = first + ((from + live_size) as i64) * 2_000_000;
        let _ = engine.seek(now, seeks + 2 + b as u64);
        visible += engine.visible_state().node_indices.len();
    }
    let live_total = t.elapsed();
    let diag = engine.diagnostics();

    println!("items                 {n_items}");
    println!("services              {SERVICES}");
    println!("apply_batch           {:.1} ms", apply.as_secs_f64() * 1e3);
    println!("first frame (build)   {:.1} ms", build.as_secs_f64() * 1e3);
    println!("random seek + frame   {:.3} ms avg over {seeks}", seek_total.as_secs_f64() * 1e3 / seeks as f64);
    println!(
        "live batch + frame    {:.3} ms avg over {live_batches} batches of {live_size} requests",
        live_total.as_secs_f64() * 1e3 / live_batches as f64
    );
    println!("timeline rebuilds     {} (merges {})", diag.timeline_rebuilds, diag.timeline_merges);
    println!("visible nodes (first) {}", first_frame.node_indices.len());
    println!("checkpoints           {} (interval {})", diag.checkpoints, diag.checkpoint_interval);
    println!("approx memory         {:.1} MiB", diag.approx_memory_bytes as f64 / 1_048_576.0);
    println!("(checksum {visible})");
}
