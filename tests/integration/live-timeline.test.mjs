import { test } from 'node:test';
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { API, ISO, ORIGIN, Session, encodeTraces, eventually, nowNs, otlpHttp } from './lib.mjs';

test('timeline history streams over the scoped live socket and advances only after acknowledgements', async () => {
  const session = new Session();
  await session.login('isolation@karbird.local');

  const suffix = randomUUID().replaceAll('-', '');
  const service = `timeline-${suffix.slice(0, 12)}`;
  const name = `history-${suffix.slice(0, 16)}`;
  const traceId = suffix.slice(0, 32);
  const spanId = suffix.slice(0, 16);
  const startNs = nowNs() - 10_000_000_000n;
  const endNs = startNs + 5_000_000n;
  const ingest = await otlpHttp('traces', encodeTraces(service, [{
    traceId,
    spanId,
    name,
    kind: 2,
    startNs,
    endNs,
  }]));
  assert.equal(ingest.status, 200);

  const fromNs = startNs - 5_000_000_000n;
  const toNs = endNs + 5_000_000_000n;
  await eventually(async () => {
    const query = new URLSearchParams({ from: fromNs.toString(), to: toNs.toString(), limit: '1000' });
    const result = await session.json(`${ISO}/timeline?${query}`);
    assert.equal(result.status, 200, JSON.stringify(result.body));
    assert.ok(result.body.items.some((item) => item.name === name), 'the processor has exposed the ingested span');
  });

  const url = new URL(`${ISO}/live`, API);
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
  const ws = new WebSocket(url, { headers: { origin: ORIGIN, cookie: session.cookieHeader() } });
  const requestId = `history-${randomUUID()}`;
  const frames = [];
  const result = await new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      ws.close();
      reject(new Error(`timed out waiting for timeline history; frames: ${frames.map((frame) => frame.type).join(',')}`));
    }, 20_000);
    const finish = (error, value) => {
      clearTimeout(timer);
      ws.close();
      if (error) reject(error);
      else resolve(value);
    };
    ws.onopen = () => ws.send(JSON.stringify({
      type: 'timeline_load', request_id: requestId,
      from_ns: fromNs.toString(), to_ns: toNs.toString(), limit: 50_000,
    }));
    ws.onmessage = (event) => {
      const frame = JSON.parse(event.data);
      frames.push(frame);
      if (frame.request_id !== requestId) return;
      if (frame.type === 'timeline_chunk' || frame.type === 'timeline_range_complete') {
        ws.send(JSON.stringify({ type: 'timeline_ack', request_id: requestId, sequence: frame.sequence }));
      } else if (frame.type === 'timeline_error') {
        finish(new Error(`history query failed: ${frame.message}`));
      } else if (frame.type === 'timeline_complete') {
        finish(null, frame);
      }
    };
    ws.onerror = () => finish(new Error('timeline WebSocket failed'));
  });

  const chunk = frames.find((frame) => frame.type === 'timeline_chunk' && frame.request_id === requestId);
  assert.ok(chunk, `frames: ${frames.map((frame) => frame.type).join(',')}`);
  assert.ok(chunk.items.some((item) => item.name === name), 'the scoped socket returned the requested span');
  assert.ok(frames.some((frame) => frame.type === 'timeline_range_complete' && frame.request_id === requestId));
  assert.equal(result.truncated, false);
});
