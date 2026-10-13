import { describe, expect, it } from "vitest";
import { countItems, countLiveBatchItems, liveItemEnvelope, liveRetryDelayMs, liveTopologySnapshot, newerSeek, parseLiveAction, parseLiveEdgeChange, parseLiveHead, parseLiveInfrastructure, parseLiveRecord, parseLiveSignals, parseTimelineFrame, timelineChunkEnvelope, timelineChunkItemsJson, timelineTail } from "./worker-utils";

describe("timeline WebSocket frames", () => {
  it("routes chunk metadata without materializing the large item array", () => {
    const text = JSON.stringify({
      type: "timeline_chunk", request_id: "r-1", sequence: 2, from_ns: "10", to_ns: "20",
      items: [{ type: "log", body: 'brackets ] } and "quote"' }],
    });
    expect(parseTimelineFrame(text)).toEqual({ type: "chunk", requestId: "r-1", sequence: 2, fromNs: "10", toNs: "20" });
    expect(JSON.parse(timelineChunkEnvelope(text) ?? "null")).toEqual({ items: [{ type: "log", body: 'brackets ] } and "quote"' }] });
    expect(JSON.parse(timelineChunkItemsJson(text) ?? "null")).toEqual([{ type: "log", body: 'brackets ] } and "quote"' }]);
  });

  it("rejects malformed chunk headers and non-decimal time ranges", () => {
    expect(parseTimelineFrame('{"type":"timeline_chunk","request_id":"r","sequence":0,"from_ns":"x","to_ns":"2","items":[]}')).toBeNull();
    expect(parseTimelineFrame('{"type":"timeline_chunk","request_id":"r","sequence":-1,"from_ns":"1","to_ns":"2","items":[]}')).toBeNull();
  });

  it("parses completion metadata and request-scoped errors", () => {
    expect(parseTimelineFrame(JSON.stringify({
      type: "timeline_complete", request_id: "r-2", truncated: false,
      completeness: { processing_caught_up: true }, logs_source: "hot",
    }))).toEqual({
      type: "complete", requestId: "r-2", truncated: false,
      completeness: { processing_caught_up: true }, logsSource: "hot",
    });
    expect(parseTimelineFrame('{"type":"timeline_error","request_id":"r-2","message":"storage unavailable"}'))
      .toEqual({ type: "error", requestId: "r-2", message: "storage unavailable" });
    expect(parseTimelineFrame('{"type":"timeline_range_complete","request_id":"r-2","sequence":3,"from_ns":"10","to_ns":"20"}'))
      .toEqual({ type: "range_complete", requestId: "r-2", sequence: 3, fromNs: "10", toNs: "20" });
  });

  it("keeps an explicit oldest-history boundary distinct from an unknown boundary", () => {
    expect(parseTimelineFrame('{"type":"timeline_complete","request_id":"r","truncated":false,"oldest_ns":"1790000000000000000"}'))
      .toMatchObject({ type: "complete", oldestNs: "1790000000000000000" });
    expect(parseTimelineFrame('{"type":"timeline_complete","request_id":"r","truncated":false,"oldest_ns":null}'))
      .toMatchObject({ type: "complete", oldestNs: null });
    expect(parseTimelineFrame('{"type":"timeline_complete","request_id":"r","truncated":false,"oldest_ns":"-1"}')).toBeNull();
  });
});

describe("timelineTail", () => {
  it("reads the metadata after the items array without parsing the items", () => {
    // The items part is deliberately invalid JSON: only the tail may be parsed.
    const text =
      '{"items":[{"type":"span", BROKEN ],"from_ns":"100","to_ns":"200","spans_truncated":true,"logs_truncated":false,"completeness":{"processing_caught_up":true}}';
    expect(timelineTail(text)).toEqual({
      from_ns: "100",
      to_ns: "200",
      spans_truncated: true,
      logs_truncated: false,
      completeness: { processing_caught_up: true },
    });
  });

  it("reads where the log items came from", () => {
    const text = '{"items":[ BROKEN ],"from_ns":"1","to_ns":"2","logs_truncated":true,"logs_source":"mixed","completeness":{}}';
    expect(timelineTail(text)).toMatchObject({ logs_source: "mixed", logs_truncated: true });
  });

  it("uses the last marker when an item contains the same characters", () => {
    const text = '{"items":[{"type":"log","body":"],\\"from_ns\\":"}],"from_ns":"1","to_ns":"2"}';
    expect(timelineTail(text)).toMatchObject({ from_ns: "1", to_ns: "2" });
  });

  it("falls back to a full parse for other field orders", () => {
    expect(timelineTail('{"from_ns":"5","items":[],"to_ns":"6"}')).toMatchObject({ from_ns: "5", to_ns: "6" });
  });

  it("throws on invalid JSON without a tail", () => {
    expect(() => timelineTail("not json")).toThrow();
  });
});

describe("countItems", () => {
  it("counts typed items", () => {
    expect(countItems('{"items":[]}')).toBe(0);
    expect(countItems('{"items":[{"type":"span"},{"type":"log"},{"type":"event"}]}')).toBe(3);
  });

  it("counts a live batch including its own type field", () => {
    expect(countItems('{"type":"batch","resume":"9","items":[{"type":"span"}]}')).toBe(2);
  });
});

describe("parseLiveHead", () => {
  it("reads type and resume token", () => {
    expect(parseLiveHead('{"type":"batch","resume":"42","items":[]}')).toEqual({ type: "batch", resume: "42" });
    expect(parseLiveHead('{"type":"heartbeat"}')).toEqual({ type: "heartbeat", resume: undefined });
  });

  it("ignores messages that do not start with the expected head", () => {
    expect(parseLiveHead('{"resume":"1","type":"batch"}')).toBeNull();
    expect(parseLiveHead(' {"type":"batch"}')).toBeNull();
    expect(parseLiveHead('{"type":"<script>"}')).toBeNull();
    expect(parseLiveHead("")).toBeNull();
  });

  it("only accepts numeric resume tokens", () => {
    expect(parseLiveHead('{"type":"gap","resume":"abc"}')).toEqual({ type: "gap", resume: undefined });
  });

  it("only looks at the first 128 characters", () => {
    expect(parseLiveHead(`${" ".repeat(128)}{"type":"batch"}`)).toBeNull();
  });
});

describe("live side-channel payloads", () => {
  it("accepts committed action updates and initial snapshot entries", () => {
    expect(parseLiveAction('{"type":"action","snapshot":true,"plan":{"id":"p1","state":"running"}}'))
      .toEqual({ plan: { id: "p1", state: "running" }, snapshot: true });
    expect(parseLiveAction('{"type":"action","snapshot":false,"plan":{"id":"p2"}}'))
      .toEqual({ plan: { id: "p2" }, snapshot: false });
    expect(parseLiveAction('{"type":"action","plan":[]}')).toBeNull();
    expect(parseLiveAction("invalid")).toBeNull();
  });

  it("parses edge deltas and rejects malformed cursors or contribution sizes", () => {
    const change = {
      id: "trace:span:calls:api:db", tenant_id: "tenant", project_id: "project", environment_id: "prod",
      source: "api", target: "db", relation: "calls", trace_id: "trace", span_id: "span",
      time_ns: "1790000000000000001", storage_cursor_ns: "1790000000000000002", request_bytes: 12, response_bytes: null,
    };
    expect(parseLiveEdgeChange(JSON.stringify({ type: "edge_change", resume: "42", change })))
      .toEqual({ resume: "42", change });
    expect(parseLiveEdgeChange(JSON.stringify({ type: "edge_change", resume: "x", change }))).toBeNull();
    expect(parseLiveEdgeChange(JSON.stringify({ type: "edge_change", resume: "42", change: { ...change, request_bytes: "12" } }))).toBeNull();
  });

  it("parses one independently framed live record and makes its timeline item consumable by WASM", () => {
    const text = JSON.stringify({
      type: "span", resume: "42", storage_cursor: "40",
      item: { type: "span", span_id: "s1" },
      record: { span_id: "s1", trace_id: "t1" },
    });
    expect(parseLiveRecord(text)).toEqual({
      kind: "span", resume: "42", storageCursor: "40", item: { type: "span", span_id: "s1" }, record: { span_id: "s1", trace_id: "t1" },
    });
    expect(JSON.parse(liveItemEnvelope(text) ?? "null")).toEqual({ items: [{ type: "span", span_id: "s1" }] });
    expect(parseLiveRecord('{"type":"batch","items":[]}')).toBeNull();
  });

  it("extracts only typed records from the same batch frame", () => {
    const text = JSON.stringify({
      type: "batch", resume: "42", items: [{ type: "log" }],
      signals: [
        { kind: "log", record: { id: "l1", body: "hello" } },
        { kind: "unknown", resume: "42", record: { id: "x" } },
        { kind: "event", resume: "42", record: null },
      ],
    });
    expect(parseLiveSignals(text)).toEqual([{ kind: "log", resume: "42", storageCursor: "42", record: { id: "l1", body: "hello" } }]);
    expect(countLiveBatchItems(text)).toBe(1);
  });

  it("does not mistake a signal's type attribute for a timeline item", () => {
    expect(countLiveBatchItems('{"type":"batch","items":[{"type":"log"}],"signals":[{"kind":"log","record":{"attributes":{"type":"exception"}}}]}')).toBe(1);
  });

  it("accepts a valid infrastructure snapshot and ignores malformed data", () => {
    expect(parseLiveInfrastructure('{"type":"infrastructure","resume":"1","snapshot":{"hosts":[],"containers":[]}}')).toEqual({ hosts: [], containers: [] });
    expect(parseLiveInfrastructure('{"type":"infrastructure","snapshot":null}')).toBeNull();
    expect(parseLiveInfrastructure("invalid")).toBeNull();
  });

  it("extracts a topology snapshot raw for WASM without parsing the graph on the UI thread", () => {
    const text = '{"type":"topology","resume":"42","snapshot":{"nodes":[{"id":"svc"}],"edges":[]}}';
    expect(JSON.parse(liveTopologySnapshot(text) ?? "null")).toEqual({ nodes: [{ id: "svc" }], edges: [] });
    expect(liveTopologySnapshot('{"type":"batch","snapshot":{}}')).toBeNull();
    expect(liveTopologySnapshot('{"type":"topology","resume":"42"}')).toBeNull();
  });
});

describe("liveRetryDelayMs", () => {
  it("backs off exponentially with jitter between 50 % and 100 %", () => {
    expect(liveRetryDelayMs(1, 1)).toBe(1000);
    expect(liveRetryDelayMs(1, 0)).toBe(500);
    expect(liveRetryDelayMs(2, 1)).toBe(2000);
    expect(liveRetryDelayMs(5, 1)).toBe(16_000);
  });

  it("is capped at 30 s", () => {
    expect(liveRetryDelayMs(6, 1)).toBe(30_000);
    expect(liveRetryDelayMs(100, 1)).toBe(30_000);
    expect(liveRetryDelayMs(100, 0)).toBe(15_000);
  });
});

describe("newerSeek", () => {
  const s = (generation: number) => ({ timeNs: String(generation), generation });

  it("keeps the newest pending seek", () => {
    expect(newerSeek(null, s(1))).toEqual(s(1));
    expect(newerSeek(s(1), s(2))).toEqual(s(2));
    expect(newerSeek(s(3), s(2))).toEqual(s(3));
    expect(newerSeek(s(3), { timeNs: "x", generation: 3 })).toEqual({ timeNs: "x", generation: 3 });
  });
});
