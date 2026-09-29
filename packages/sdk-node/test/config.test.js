// Configuration checks that fail before the OpenTelemetry SDK starts (safe to run in-process).
import { test, beforeEach, afterEach } from 'node:test';
import assert from 'node:assert/strict';
import { getControlHub, startControlHub } from '../src/index.js';

const GLOBAL_KEY = Symbol.for('controlhub.sdk-node');
const OTEL_API_KEY = Symbol.for('opentelemetry.js.api.1');
const ENV_KEYS = ['CONTROLHUB_SERVICE_NAME', 'OTEL_SERVICE_NAME', 'CONTROLHUB_INGEST_TOKEN'];
let saved;

beforeEach(() => {
  saved = Object.fromEntries(ENV_KEYS.map((k) => [k, process.env[k]]));
  for (const k of ENV_KEYS) delete process.env[k];
});

afterEach(() => {
  for (const [k, v] of Object.entries(saved)) {
    if (v === undefined) delete process.env[k];
    else process.env[k] = v;
  }
  delete globalThis[GLOBAL_KEY];
  delete globalThis[OTEL_API_KEY];
});

test('getControlHub is null before start', () => {
  assert.equal(getControlHub(), null);
});

test('requires a service name', () => {
  assert.throws(() => startControlHub({ token: 't' }), /serviceName/);
  assert.equal(getControlHub(), null);
});

test('requires an ingest token and does not fall back to an anonymous exporter', () => {
  assert.throws(() => startControlHub({ serviceName: 'svc' }), /ingest token/);
  process.env.OTEL_SERVICE_NAME = 'from-otel-env';
  assert.throws(() => startControlHub(), /ingest token/);
  assert.equal(getControlHub(), null);
});

test('refuses to start next to a foreign OpenTelemetry setup unless allowed', () => {
  globalThis[OTEL_API_KEY] = { trace: {} };
  assert.throws(() => startControlHub({ serviceName: 'svc', token: 't' }), /already registered/);
});

test('reuses a running instance with a warning instead of starting twice', async () => {
  const existing = { marker: true };
  globalThis[GLOBAL_KEY] = existing;
  const warnings = [];
  const original = process.emitWarning;
  process.emitWarning = (message, options) => warnings.push({ message, code: options?.code });
  try {
    assert.equal(startControlHub({ serviceName: 'svc', token: 't' }), existing);
  } finally {
    process.emitWarning = original;
  }
  assert.ok(warnings.some((w) => w.code === 'CONTROLHUB_DOUBLE_INIT'));
});
