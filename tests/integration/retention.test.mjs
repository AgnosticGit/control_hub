// ClickHouse retention is configured once (KARBIRD_{LOGS,SPANS,EVENTS,METRICS}_RETENTION_DAYS, locally in
// .karbird/settings.env) and applied by bootstrap as the table TTLs. The archiver and the API
// compare the configuration with the real TTLs at startup and refuse to run when they differ.
// Run: node --test tests/integration/retention.test.mjs (with `just dev` running)
import { test, before } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { root } from './lib.mjs';

const exe = (name) => join(root, 'target', 'debug', process.platform === 'win32' ? `${name}.exe` : name);
const VARS = {
  logs: ['KARBIRD_LOGS_RETENTION_DAYS', 14],
  spans: ['KARBIRD_SPANS_RETENTION_DAYS', 14],
  events: ['KARBIRD_EVENTS_RETENTION_DAYS', 30],
  metrics: ['KARBIRD_METRICS_RETENTION_DAYS', 14],
};

/** What the services see: real environment, then .karbird/settings.env, then the default. */
function configured(signal) {
  const [name, fallback] = VARS[signal];
  const path = join(root, '.karbird', 'settings.env');
  const settings = existsSync(path)
    ? Object.fromEntries(
        readFileSync(path, 'utf8')
          .split(/\r?\n/)
          .filter((l) => l.trim() && !l.trim().startsWith('#') && l.includes('='))
          .map((l) => [l.slice(0, l.indexOf('=')).trim(), l.slice(l.indexOf('=') + 1).trim()]),
      )
    : {};
  return Number(process.env[name] ?? settings[name] ?? fallback);
}

function start(bin, args, env) {
  const r = spawnSync(bin, args, { cwd: root, encoding: 'utf8', timeout: 120_000, env: { ...process.env, ...env } });
  assert.notEqual(r.status, null, `${bin} did not exit (it must refuse to start)\n${r.stderr}`);
  return r;
}

const archiverOnce = (env) => start(exe('karbird-archiver'), ['once', '--from-ns', '0', '--to-ns', '0'], env);
const apiServe = (env) => start(exe('karbird-api'), ['serve', '--addr', '127.0.0.1:0'], env);

before(() => {
  const build = spawnSync('cargo', ['build', '-q', '-p', 'karbird-archiver', '-p', 'karbird-api'], { cwd: root, stdio: 'inherit' });
  assert.equal(build.status, 0, 'cargo build');
});

test('the archiver runs when its log retention matches the ClickHouse TTL', () => {
  const r = archiverOnce({ KARBIRD_LOGS_RETENTION_DAYS: String(configured('logs')) });
  assert.equal(r.status, 0, r.stderr);
  assert.equal(JSON.parse(r.stdout.trim().split(/\r?\n/).pop()).windows, 0);
});

test('the archiver refuses to start when the log retention differs from the ClickHouse TTL', () => {
  const days = configured('logs');
  const r = archiverOnce({ KARBIRD_LOGS_RETENTION_DAYS: String(days + 1) });
  assert.notEqual(r.status, 0);
  assert.match(r.stderr, new RegExp(`KARBIRD_LOGS_RETENTION_DAYS is ${days + 1} days`));
  assert.match(r.stderr, new RegExp(`keeps logs for ${days} days`));
  assert.match(r.stderr, /just bootstrap/);
});

for (const signal of ['logs', 'spans', 'events', 'metrics']) {
  test(`the API refuses to start when ${signal} retention differs from the ClickHouse TTL`, () => {
    const [name] = VARS[signal];
    const days = configured(signal);
    const wanted = days > 2 ? days - 1 : days + 1;
    const r = apiServe({ [name]: String(wanted), KARBIRD_ARCHIVE_HOT_WINDOW_HOURS: '24' });
    assert.notEqual(r.status, 0);
    assert.match(r.stderr, new RegExp(`${name} is ${wanted} days but ClickHouse keeps ${signal} for ${days} days`));
    assert.match(r.stderr, /just bootstrap/);
  });
}

test('the API refuses a hot window that reaches past the log retention', () => {
  const days = configured('logs');
  const r = apiServe({ KARBIRD_ARCHIVE_HOT_WINDOW_HOURS: String(days * 24) });
  assert.notEqual(r.status, 0);
  assert.match(r.stderr, /KARBIRD_ARCHIVE_HOT_WINDOW_HOURS=\d+ must be below the log retention/);
});

test('retention outside 2..3650 days is rejected before anything connects', () => {
  for (const bad of ['1', '3651', 'forever']) {
    const r = archiverOnce({ KARBIRD_LOGS_RETENTION_DAYS: bad });
    assert.notEqual(r.status, 0, bad);
    assert.match(r.stderr, /logs-retention-days|KARBIRD_LOGS_RETENTION_DAYS/, bad);
  }
});
