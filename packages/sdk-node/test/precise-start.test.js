import assert from 'node:assert/strict';
import { test } from 'node:test';
import { tracing } from '@opentelemetry/sdk-node';
import { ClockOffset, PreciseStartSpanProcessor, hrToNs, nsToHr, perfToEpochNs, refineStart } from '../src/precise-start.js';

test('refineStart trusts the monotonic clock unless the wall clock drifted away', () => {
  const ms = 1_790_000_000_123n * 1_000_000n;
  const mono = ms + 456_789n;
  // Date.now() lagging by under 2 ms is ignored.
  assert.equal(refineStart(ms, mono, -900_000), mono);
  assert.equal(refineStart(ms, mono, 1_500_000), mono);
  // A real drift or a stepped clock is followed.
  assert.equal(refineStart(ms, mono, 30_000_000), mono + 30_000_000n);
  // Already sub-millisecond: kept.
  assert.equal(refineStart(ms + 7n, mono, 0), ms + 7n);
});

test('clock offset intersects observations and restarts on a contradiction', () => {
  const c = new ClockOffset();
  // True offset 300 µs: wall = floor_ms(mono + 300 µs).
  const truth = 300_000;
  let est = 0;
  for (let i = 0; i < 50; i++) {
    const mono = 5_000_000_000n + BigInt(i * 137_000);
    const wall = ((mono + BigInt(truth)) / 1_000_000n) * 1_000_000n;
    est = c.observe(wall, mono, i * 0.137);
  }
  assert.ok(Math.abs(est - truth) < 20_000, `estimate ${est} close to ${truth}`);
  // Wall clock stepped by 1 s: the estimate follows instead of getting stuck.
  est = c.observe(6_001_000_000n, 5_000_000_000n, 10);
  assert.ok(est > 1_000_000_000 && est < 1_002_000_000);
});

test('hr time round-trips through nanoseconds', () => {
  const ns = 1_790_000_000_123_456_789n;
  assert.deepEqual(nsToHr(ns), [1_790_000_000, 123_456_789]);
  assert.equal(hrToNs(nsToHr(ns)), ns);
  assert.equal(perfToEpochNs(1.5, 1000), 1_001_500_000n);
});

test('spans of one fast request get distinct, ordered, sub-millisecond starts', async () => {
  const exporter = new tracing.InMemorySpanExporter();
  const provider = new tracing.BasicTracerProvider({
    spanProcessors: [new PreciseStartSpanProcessor(), new tracing.SimpleSpanProcessor(exporter)],
  });
  const tracer = provider.getTracer('test');
  // Let the clock-offset estimate settle, as it does after the first requests of a process.
  const warm = performance.now();
  while (performance.now() - warm < 30) tracer.startSpan('warmup').end();
  const outer = tracer.startSpan('outer');
  const busy = performance.now();
  while (performance.now() - busy < 0.3) {
    // Burn 0.3 ms so the inner span starts measurably later.
  }
  const inner = tracer.startSpan('inner');
  inner.end();
  outer.end();
  await provider.forceFlush();
  const spans = exporter.getFinishedSpans();
  const byName = Object.fromEntries(spans.map((s) => [s.name, s]));
  const outerStart = hrToNs(byName.outer.startTime);
  const innerStart = hrToNs(byName.inner.startTime);
  assert.ok(innerStart > outerStart, 'inner starts after outer');
  assert.ok(innerStart - outerStart >= 200_000n, 'at least 0.2 ms apart');
  // End = start + duration still holds.
  for (const s of spans) {
    assert.equal(hrToNs(s.endTime) - hrToNs(s.startTime), hrToNs(s.duration));
  }
  await provider.shutdown();
});

test('an explicit start time is kept', async () => {
  const exporter = new tracing.InMemorySpanExporter();
  const provider = new tracing.BasicTracerProvider({
    spanProcessors: [new PreciseStartSpanProcessor(), new tracing.SimpleSpanProcessor(exporter)],
  });
  const at = new Date(Date.UTC(2026, 0, 1, 0, 0, 0, 5));
  provider.getTracer('test').startSpan('given', { startTime: at }).end(new Date(at.getTime() + 3));
  await provider.forceFlush();
  const [span] = exporter.getFinishedSpans();
  assert.equal(hrToNs(span.startTime), BigInt(at.getTime()) * 1_000_000n);
  await provider.shutdown();
});
