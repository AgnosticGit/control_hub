//! WASM facade. Heavy inputs (topology, batches) are passed as the raw JSON
//! response text and parsed with serde_json inside WASM, so the Worker never
//! materialises millions of JS objects. Render buffers leave as typed arrays
//! (one copy out of WASM memory; the Worker then transfers them to the UI
//! thread without further copies).

use js_sys::{Float32Array, Object, Reflect, Uint32Array};
use karbird_engine_core::{Engine, EngineConfig, Filters, SeekOutcome, Viewport};
use wasm_bindgen::prelude::*;

fn js_err(e: impl std::fmt::Display) -> JsValue {
    JsValue::from(js_sys::Error::new(&e.to_string()))
}

fn to_js<T: serde::Serialize>(v: &T) -> Result<JsValue, JsValue> {
    let ser = serde_wasm_bindgen::Serializer::json_compatible();
    v.serialize(&ser).map_err(js_err)
}

fn set(obj: &Object, key: &str, value: &JsValue) -> Result<(), JsValue> {
    Reflect::set(obj, &JsValue::from_str(key), value).map(|_| ())
}

/// Empty or whitespace-only text means defaults.
fn parse_config(config_json: &str) -> Result<EngineConfig, serde_json::Error> {
    if config_json.trim().is_empty() { Ok(EngineConfig::default()) } else { serde_json::from_str(config_json) }
}

/// Version of the engine ABI; the TS wrapper refuses to run against a mismatching build.
#[wasm_bindgen(js_name = engineAbiVersion)]
pub fn engine_abi_version() -> u32 {
    4
}

#[wasm_bindgen]
#[derive(Debug)]
pub struct WebEngine {
    inner: Engine,
}

#[wasm_bindgen]
impl WebEngine {
    /// `config_json`: serialized `EngineConfig` (empty string = defaults).
    #[wasm_bindgen(constructor)]
    pub fn new(config_json: &str) -> Result<WebEngine, JsValue> {
        let config = parse_config(config_json).map_err(js_err)?;
        Ok(WebEngine { inner: Engine::new(config) })
    }

    #[wasm_bindgen(js_name = loadTopology)]
    pub fn load_topology(&mut self, json: &str) -> Result<(), JsValue> {
        self.inner.load_topology_json(json).map_err(js_err)
    }

    #[wasm_bindgen(js_name = applyBatch)]
    pub fn apply_batch(&mut self, json: &str) -> Result<(), JsValue> {
        self.inner.apply_batch_json(json).map_err(js_err)
    }

    #[wasm_bindgen(js_name = evictBefore)]
    pub fn evict_before(&mut self, before_ns: i64) {
        self.inner.evict_before(before_ns);
    }

    /// Returns `false` when the generation is stale and the seek was ignored.
    pub fn seek(&mut self, time_ns: i64, generation: u64) -> bool {
        self.inner.seek(time_ns, generation) == SeekOutcome::Applied
    }

    #[wasm_bindgen(js_name = setViewport)]
    pub fn set_viewport(&mut self, x: f32, y: f32, width: f32, height: f32, scale: f32) {
        self.inner.set_viewport(Viewport { x, y, width, height, scale });
    }

    #[wasm_bindgen(js_name = setFilters)]
    pub fn set_filters(&mut self, filters: JsValue) -> Result<(), JsValue> {
        let f: Filters = serde_wasm_bindgen::from_value(filters).map_err(js_err)?;
        self.inner.set_filters(f);
        Ok(())
    }

    /// `json`: raw `InfrastructureSnapshot` response text.
    #[wasm_bindgen(js_name = setInfrastructure)]
    pub fn set_infrastructure(&mut self, json: &str) -> Result<(), JsValue> {
        self.inner.set_infrastructure_json(json).map_err(js_err)
    }

    #[wasm_bindgen(js_name = setHopWindow)]
    pub fn set_hop_window(&mut self, before_ns: i64, after_ns: i64) {
        self.inner.set_hop_window(before_ns, after_ns);
    }

    #[wasm_bindgen(js_name = prevEventTime)]
    pub fn prev_event_time(&mut self, t: i64) -> Option<i64> {
        self.inner.prev_event_time(t)
    }

    #[wasm_bindgen(js_name = nextEventTime)]
    pub fn next_event_time(&mut self, t: i64) -> Option<i64> {
        self.inner.next_event_time(t)
    }

    /// `{ nodeData: Float32Array, nodeIndices: Uint32Array, edgeData, edgeIndices, changedNodes, hopData, summary }`
    #[wasm_bindgen(js_name = getVisibleState)]
    pub fn get_visible_state(&mut self) -> Result<JsValue, JsValue> {
        let vs = self.inner.visible_state();
        let obj = Object::new();
        set(&obj, "nodeData", &Float32Array::from(vs.node_data.as_slice()).into())?;
        set(&obj, "nodeIndices", &Uint32Array::from(vs.node_indices.as_slice()).into())?;
        set(&obj, "edgeData", &Float32Array::from(vs.edge_data.as_slice()).into())?;
        set(&obj, "edgeIndices", &Uint32Array::from(vs.edge_indices.as_slice()).into())?;
        set(&obj, "changedNodes", &Uint32Array::from(vs.changed_nodes.as_slice()).into())?;
        set(&obj, "hopData", &Float32Array::from(vs.hop_data.as_slice()).into())?;
        set(&obj, "summary", &to_js(&vs.summary)?)?;
        Ok(obj.into())
    }

    #[wasm_bindgen(js_name = getTopology)]
    pub fn get_topology(&self) -> Result<JsValue, JsValue> {
        to_js(&self.inner.topology())
    }

    /// `at_ns`: optional time; defaults to the current selected time.
    #[wasm_bindgen(js_name = getNodeMetrics)]
    pub fn get_node_metrics(&mut self, node_id: &str, at_ns: Option<i64>) -> Result<JsValue, JsValue> {
        match self.inner.node_metrics(node_id, at_ns) {
            Some(m) => to_js(&m),
            None => Ok(JsValue::NULL),
        }
    }

    /// Traffic over the edge `source_id → target_id`; `null` when there is no such edge.
    #[wasm_bindgen(js_name = getEdgeMetrics)]
    pub fn get_edge_metrics(
        &mut self,
        source_id: &str,
        target_id: &str,
        at_ns: Option<i64>,
    ) -> Result<JsValue, JsValue> {
        match self.inner.edge_metrics(source_id, target_id, at_ns) {
            Some(m) => to_js(&m),
            None => Ok(JsValue::NULL),
        }
    }

    pub fn compare(&mut self, a_ns: i64, b_ns: i64) -> Result<JsValue, JsValue> {
        to_js(&self.inner.compare(a_ns, b_ns))
    }

    #[wasm_bindgen(js_name = timelineTracks)]
    pub fn timeline_tracks(&self, from_ns: i64, to_ns: i64, buckets: u32, clip_cap: u32) -> Result<JsValue, JsValue> {
        to_js(&self.inner.timeline_tracks(from_ns, to_ns, buckets, clip_cap))
    }

    pub fn pick(&self, x: f32, y: f32) -> Option<String> {
        self.inner.pick(x, y)
    }

    #[wasm_bindgen(js_name = getDiagnostics)]
    pub fn get_diagnostics(&mut self) -> Result<JsValue, JsValue> {
        to_js(&self.inner.diagnostics())
    }
}

/// Native tests cover every method whose success path creates no JS value. Anything that builds a
/// `JsValue` (errors, typed arrays, `serde_wasm_bindgen`) aborts outside wasm32, so those paths are
/// exercised through the engine-core API they delegate to and by the browser tests.
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use karbird_engine_core as core;

    const TS_INDEX: &str = include_str!("../../../packages/web-engine/src/index.ts");

    fn ts_const(name: &str) -> u32 {
        let prefix = format!("export const {name} = ");
        let line =
            TS_INDEX.lines().find(|l| l.starts_with(&prefix)).unwrap_or_else(|| panic!("{name} missing in index.ts"));
        line[prefix.len()..].trim_end_matches(';').trim().parse().unwrap()
    }

    fn ts_status(name: &str) -> f32 {
        let line = TS_INDEX.lines().find(|l| l.starts_with("export const STATUS = ")).unwrap();
        let at = line.find(&format!("{name}: ")).unwrap() + name.len() + 2;
        line[at..].chars().take_while(char::is_ascii_digit).collect::<String>().parse().unwrap()
    }

    const BATCH: &str = r#"{"items":[
        {"type":"span","trace_id":"t1","span_id":"a","service_id":"svc_gw","service_name":"gw","name":"GET /","kind":2,"start_ns":"1000000000","end_ns":"3000000000","is_error":false},
        {"type":"span","trace_id":"t1","span_id":"b","parent_span_id":"a","service_id":"svc_orders","service_name":"orders","name":"order","kind":2,"start_ns":"1500000000","end_ns":"2500000000","is_error":true},
        {"type":"log","id":"l1","service_id":"svc_orders","service_name":"orders","time_ns":"2000000000","severity":17}
    ]}"#;

    fn engine() -> WebEngine {
        let mut e = WebEngine::new("").unwrap();
        e.load_topology(r#"{"nodes":[],"edges":[]}"#).unwrap();
        e.apply_batch(BATCH).unwrap();
        e
    }

    #[test]
    fn abi_and_buffer_layout_match_the_typescript_wrapper() {
        assert_eq!(engine_abi_version(), ts_const("ENGINE_ABI_VERSION"), "bump both sides together");
        for (name, value) in [
            ("NODE_STRIDE", core::NODE_STRIDE as u32),
            ("EDGE_STRIDE", core::EDGE_STRIDE as u32),
            ("HOP_STRIDE", core::HOP_STRIDE as u32),
            ("HOP_ERROR", core::HOP_ERROR),
            ("HOP_LEAF", core::HOP_LEAF),
            ("HOP_HIGHLIGHT", core::HOP_HIGHLIGHT),
            ("HOP_CLAMPED", core::HOP_CLAMPED),
            ("HOP_DIMMED", core::HOP_DIMMED),
            ("FLAG_HIGHLIGHT", core::FLAG_HIGHLIGHT),
            ("FLAG_INFERRED", core::FLAG_INFERRED),
            ("FLAG_DECLARED", core::FLAG_DECLARED),
            ("FLAG_RESOURCE", core::FLAG_RESOURCE),
            ("FLAG_PROBLEM", core::FLAG_PROBLEM),
            ("FLAG_DIMMED", core::FLAG_DIMMED),
            ("FLAG_EXTERNAL", core::FLAG_EXTERNAL),
        ] {
            assert_eq!(ts_const(name), value, "{name}");
        }
    }

    #[test]
    fn status_encoding_matches_the_typescript_wrapper() {
        let mut e = engine();
        assert!(e.seek(2_600_000_000, 1));
        let vs = e.inner.visible_state();
        let topo = e.inner.topology();
        let status = |id: &str| {
            let idx = topo.nodes.iter().find(|n| n.id == id).unwrap().index;
            let k = vs.node_indices.iter().position(|&i| i == idx).unwrap();
            vs.node_data[k * core::NODE_STRIDE + 3]
        };
        assert_eq!(status("svc_orders"), ts_status("failing"));
        assert_eq!(status("svc_gw"), ts_status("healthy"), "one request in flight");
        assert!(e.seek(0, 2));
        let vs = e.inner.visible_state();
        assert!(vs.node_data.chunks(core::NODE_STRIDE).all(|c| c[3] == ts_status("unknown")));
    }

    #[test]
    fn config_parsing() {
        assert_eq!(parse_config("").unwrap(), EngineConfig::default());
        assert_eq!(parse_config(" \n\t").unwrap(), EngineConfig::default());
        assert_eq!(parse_config("{}").unwrap(), EngineConfig::default(), "missing fields take defaults");
        let c = parse_config(r#"{"max_items":5,"rolling_window_ns":"1000","hop_before_ns":"-1"}"#).unwrap();
        assert_eq!((c.max_items, c.rolling_window_ns, c.hop_before_ns), (5, 1000, -1));
        for bad in ["{", "null", r#"{"max_items":-1}"#, r#"{"rolling_window_ns":"x"}"#, r#"{"max_hops":1.5}"#] {
            assert!(parse_config(bad).is_err(), "accepted {bad}");
        }
        // A negative hop window from the config is clamped by the engine.
        let e = WebEngine::new(r#"{"hop_before_ns":"-5","max_items":3}"#).unwrap();
        assert_eq!(e.inner.config().max_items, 3);
    }

    #[test]
    fn methods_delegate_to_the_engine() {
        let mut e = engine();
        assert_eq!(e.inner.item_count(), 3);
        assert!(e.seek(2_000_000_000, 5));
        assert!(!e.seek(0, 4), "stale generation is reported as false");
        assert_eq!(e.inner.current_time(), Some(2_000_000_000));
        assert_eq!(e.prev_event_time(1_500_000_000), Some(1_000_000_000));
        assert_eq!(e.next_event_time(1_500_000_000), Some(2_000_000_000));
        assert_eq!(e.next_event_time(3_000_000_000), None);

        e.set_viewport(1e6, 1e6, 10.0, 10.0, 1.0);
        assert_eq!(e.inner.visible_state().summary.visible_nodes, 0);
        e.set_viewport(0.0, 0.0, 0.0, 0.0, 1.0);
        assert_eq!(e.inner.visible_state().summary.visible_nodes, 2);

        e.set_hop_window(i64::MIN, i64::MAX);
        let vs = e.inner.visible_state();
        assert_eq!(vs.summary.hops_in_window, 1, "the gw -> orders call");

        let topo = e.inner.topology();
        let n = &topo.nodes[0];
        assert_eq!(e.pick(n.x, n.y).as_deref(), Some(n.id.as_str()));
        assert_eq!(e.pick(n.x + 1e6, n.y), None);

        e.evict_before(2_600_000_000);
        assert_eq!(e.inner.item_count(), 1, "only the gateway span ends after the cutoff");
    }

    #[test]
    fn infrastructure_snapshot_text_reaches_the_engine() {
        let mut e = engine();
        e.inner.set_filters(Filters { infrastructure: true, ..Filters::default() });
        e.set_infrastructure(
            r#"{"at_ns":"2000000000","window_ns":"120000000000","hosts":[{"id":"vps-app","name":"vps-app","has_metrics":true,"cpu_utilization":0.5,"containers":[]}],
                "instances":[{"service_id":"svc_gw","service_name":"gw","instance_id":"i1","host_id":"vps-app","match_reason":"container_id","requests":1}],"hints":[],"notes":[]}"#,
        )
        .unwrap();
        let view = e.inner.topology().infra.unwrap();
        assert_eq!(view.frames.len(), 1);
        assert_eq!(view.frames[0].label, "vps-app · CPU 50%");
    }

    #[test]
    fn batches_from_the_worker_accumulate_and_dedupe() {
        let mut e = engine();
        e.apply_batch(BATCH).unwrap();
        e.apply_batch(r#"{"items":[]}"#).unwrap();
        let d = e.inner.diagnostics();
        assert_eq!((d.spans, d.logs, d.duplicates_ignored), (2, 1, 3));
    }
}
