//! WASM facade. Heavy inputs (topology, batches) are passed as the raw JSON
//! response text and parsed with serde_json inside WASM, so the Worker never
//! materialises millions of JS objects. Render buffers leave as typed arrays
//! (one copy out of WASM memory; the Worker then transfers them to the UI
//! thread without further copies).

use controlhub_engine_core::{Engine, EngineConfig, Filters, SeekOutcome, Viewport};
use js_sys::{Float32Array, Object, Reflect, Uint32Array};
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

/// Version of the engine ABI; the TS wrapper refuses to run against a mismatching build.
#[wasm_bindgen(js_name = engineAbiVersion)]
pub fn engine_abi_version() -> u32 {
    2
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
        let config: EngineConfig = if config_json.trim().is_empty() {
            EngineConfig::default()
        } else {
            serde_json::from_str(config_json).map_err(js_err)?
        };
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
