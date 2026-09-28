// Bounded, batching client for `POST /api/v1/events/batch`.
//
// Delivery is at-least-once: a batch is retried with exponential backoff and
// full jitter on network errors, 429 and 5xx. When the in-memory queue is
// full the oldest events are dropped and counted (never silently).

import { randomUUID } from 'node:crypto';
import { performance } from 'node:perf_hooks';

const MAX_QUEUE = 5000;
const MAX_BATCH = 200;
const FLUSH_INTERVAL_MS = 1000;
const MAX_ATTEMPTS = 5;
const REQUEST_TIMEOUT_MS = 5000;

export function nowUnixNanos() {
  const ms = performance.timeOrigin + performance.now();
  const whole = Math.floor(ms);
  const frac = Math.round((ms - whole) * 1e6);
  return (BigInt(whole) * 1_000_000n + BigInt(frac)).toString();
}

export class EventsClient {
  /**
   * @param {{url: string, headers: Record<string,string>, serviceName: string,
   *          serviceNamespace: string, instanceId: string, fetchImpl?: typeof fetch}} opts
   */
  constructor(opts) {
    this.url = opts.url;
    this.headers = { ...opts.headers, 'content-type': 'application/json' };
    this.service = { name: opts.serviceName, namespace: opts.serviceNamespace, instance_id: opts.instanceId };
    this.fetchImpl = opts.fetchImpl ?? fetch;
    this.source = { id: `${opts.serviceName}/${opts.instanceId}`, epoch: randomUUID() };
    this.sequence = 0;
    this.queue = [];
    this.timer = null;
    this.flushing = null;
    this.counters = { queued: 0, sent: 0, rejected: 0, dropped: 0, retried: 0, failed: 0 };
  }

  start() {
    if (this.timer) return;
    this.timer = setInterval(() => void this.flush(), FLUSH_INTERVAL_MS);
    this.timer.unref();
  }

  /** @param {string} eventType */
  push(eventType, fields = {}) {
    const event = {
      event_id: randomUUID(),
      event_type: eventType,
      service: this.service,
      event_time: fields.eventTime ?? nowUnixNanos(),
      source: { ...this.source, sequence: this.sequence++ },
      attributes: fields.attributes ?? {},
    };
    if (isValidHex(fields.traceId, 32)) event.trace_id = fields.traceId;
    if (isValidHex(fields.spanId, 16)) event.span_id = fields.spanId;
    if (fields.entityId) event.entity_id = fields.entityId;
    if (fields.payload !== undefined) event.payload = fields.payload;
    if (this.queue.length >= MAX_QUEUE) {
      this.queue.shift();
      this.counters.dropped++;
    }
    this.queue.push(event);
    this.counters.queued++;
    if (this.queue.length >= MAX_BATCH) void this.flush();
  }

  stats() {
    return { ...this.counters, pending: this.queue.length };
  }

  async flush() {
    if (this.flushing) return this.flushing;
    this.flushing = (async () => {
      try {
        while (this.queue.length > 0) {
          const batch = this.queue.splice(0, MAX_BATCH);
          const ok = await this.send(batch);
          if (!ok) {
            this.counters.failed += batch.length;
            break;
          }
        }
      } finally {
        this.flushing = null;
      }
    })();
    return this.flushing;
  }

  async send(batch) {
    const body = JSON.stringify({ events: batch });
    for (let attempt = 0; attempt < MAX_ATTEMPTS; attempt++) {
      if (attempt > 0) {
        this.counters.retried++;
        const cap = Math.min(10_000, 200 * 2 ** attempt);
        await sleep(Math.random() * cap);
      }
      let res;
      try {
        res = await this.fetchImpl(this.url, {
          method: 'POST',
          headers: this.headers,
          body,
          signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
        });
      } catch {
        continue;
      }
      if (res.ok) {
        let rejected = 0;
        try {
          const json = await res.json();
          rejected = Array.isArray(json?.rejected) ? json.rejected.length : 0;
        } catch {
          // Response body is informational only.
        }
        this.counters.rejected += rejected;
        this.counters.sent += batch.length - rejected;
        return true;
      }
      await res.body?.cancel();
      if (res.status !== 429 && res.status < 500) {
        this.counters.rejected += batch.length;
        return true;
      }
    }
    return false;
  }

  async shutdown() {
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
    await this.flush();
  }
}

function isValidHex(v, len) {
  return typeof v === 'string' && v.length === len && /^[0-9a-f]+$/.test(v) && !/^0+$/.test(v);
}

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}
