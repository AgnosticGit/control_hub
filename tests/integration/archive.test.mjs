// Log archive end to end (ADR-0015): OTLP logs -> ClickHouse -> `controlhub-archiver once` ->
// object storage -> API `source=archive`. The archive must return exactly what ClickHouse
// returns (lossless), with filters, cursors and tenant isolation. Writes only into the isolation
// tenant's `isolation-test` environment, into the last closed hour.
// Run: cargo build -p controlhub-archiver && node --test tests/integration/archive.test.mjs
import { test, before } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { join } from 'node:path';
import { API, DEMO, ISO, Session, encodeLogs, eventually, hex, otlpHttp, root } from './lib.mjs';

const HOUR = 3_600_000_000_000n;
const bin = join(root, 'target', 'debug', process.platform === 'win32' ? 'controlhub-archiver.exe' : 'controlhub-archiver');
const run = hex(6);
const service = `archive-it-${run}`;
const traceId = hex(16);
const windowStart = (BigInt(Date.now()) * 1_000_000n / HOUR) * HOUR - HOUR;
const windowEnd = windowStart + HOUR;
const range = `from=${windowStart}&to=${windowEnd}`;

// Several templates with changing numbers and IDs, one with markup, one with the template marker
// character, unicode and an error line; together they exercise every segment column.
const logs = Array.from({ length: 48 }, (_, i) => {
  const kind = i % 6;
  const body = [
    `order ${1000 + i} created for user u_${i % 5} run ${run}`,
    `payment ${hex(8)}f authorised amount ${i * 7}.${i % 10} EUR run ${run}`,
    `GET /api/orders/${i} 200 ${i % 13}ms run ${run}`,
    `<b>cache</b> miss key=item:${i} ünïcödé run ${run}`,
    `raw \u0001 marker ${i} run ${run}`,
    `inventory reservation failed for sku-${i % 3}: timeout after ${3000 + i}ms run ${run}`,
  ][kind];
  return {
    timeNs: windowStart + 60_000_000_000n + BigInt(i) * 1_000_000_000n,
    severity: kind === 5 ? 17 : 9,
    severityText: kind === 5 ? 'ERROR' : 'INFO',
    body,
    attributes: { 'http.route': kind === 2 ? '/api/orders/:id' : '', attempt: String(i % 3) },
    ...(i % 4 === 0 ? { traceId, spanId: hex(8) } : {}),
  };
});

let iso;
let viewer;

function archiver(args) {
  const r = spawnSync(bin, args, { cwd: root, encoding: 'utf8', timeout: 180_000 });
  assert.equal(r.status, 0, `controlhub-archiver ${args.join(' ')}\n${r.stderr}`);
  return JSON.parse(r.stdout.trim().split(/\r?\n/).pop());
}

const archiveOnce = (from = windowStart, to = windowEnd) =>
  archiver([
    'once',
    '--tenant', 'isolation-tenant',
    '--project', 'isolation',
    '--environment', 'isolation-test',
    '--from-ns', String(from),
    '--to-ns', String(to),
    '--min-age-secs', '0',
  ]);

// An hour past the API's hot window (13 days by default) but still inside ClickHouse's 14-day
// TTL: `auto` reads it from the archive, as it would for any older range.
const DAY = 24n * HOUR;
const oldStart = ((BigInt(Date.now()) * 1_000_000n - 13n * DAY) / HOUR) * HOUR - 2n * HOUR;
const oldEnd = oldStart + HOUR;
const oldRun = hex(6);
const oldService = `archive-old-${oldRun}`;
const oldTrace = hex(16);
const oldLogs = Array.from({ length: 12 }, (_, i) => ({
  timeNs: oldStart + 120_000_000_000n + BigInt(i) * 1_000_000_000n,
  severity: i % 4 === 3 ? 17 : 9,
  severityText: i % 4 === 3 ? 'ERROR' : 'INFO',
  body: i % 2 === 0 ? `old order ${i} shipped run ${oldRun}` : `old cache warmed in ${i}ms run ${oldRun}`,
  ...(i === 5 ? { traceId: oldTrace, spanId: hex(8) } : {}),
}));

async function list(session, source, extra = '', q = run) {
  const r = await session.json(`${ISO}/logs?${range}&q=${encodeURIComponent(q)}&limit=1000&source=${source}${extra}`);
  assert.equal(r.status, 200, JSON.stringify(r.body));
  return r.body;
}

async function pages(source, limit) {
  const out = [];
  let cursor = null;
  for (let i = 0; i < 20; i++) {
    const r = await iso.json(`${ISO}/logs?${range}&q=${run}&limit=${limit}&source=${source}${cursor ? `&cursor=${cursor}` : ''}`);
    assert.equal(r.status, 200, JSON.stringify(r.body));
    out.push(...r.body.logs);
    cursor = r.body.next_cursor;
    if (!cursor) return out;
  }
  throw new Error('pagination did not end');
}

before(async () => {
  const build = spawnSync('cargo', ['build', '-q', '-p', 'controlhub-archiver'], { cwd: root, stdio: 'inherit' });
  assert.equal(build.status, 0, 'cargo build -p controlhub-archiver');
  iso = new Session();
  await iso.login('isolation@controlhub.local');
  viewer = new Session();
  await viewer.login('viewer@controlhub.local');

  const r = await otlpHttp('logs', encodeLogs(service, logs));
  assert.equal(r.status, 200, `ingest: ${r.status}`);
  const old = await otlpHttp('logs', encodeLogs(oldService, oldLogs));
  assert.equal(old.status, 200, `ingest old: ${old.status}`);
  await eventually(
    async () => {
      const hot = await list(iso, 'hot');
      assert.equal(hot.logs.length, logs.length, `hot logs visible: ${hot.logs.length}`);
      const oldHot = await iso.json(`${ISO}/logs?from=${oldStart}&to=${oldEnd}&q=${oldRun}&source=hot`);
      assert.equal(oldHot.body?.logs?.length, oldLogs.length, 'old logs reached ClickHouse');
    },
    { timeoutMs: 60_000 },
  );
});

test('the archiver stores the window and the archive returns exactly what ClickHouse returns', async () => {
  const summary = archiveOnce();
  assert.ok(summary.windows === 1 && summary.segments >= 1, JSON.stringify(summary));
  assert.ok(summary.rows >= logs.length, `rows ${summary.rows}`);
  assert.ok(summary.stored_bytes > 0 && summary.stored_bytes < summary.source_bytes, JSON.stringify(summary));

  const hot = await list(iso, 'hot');
  const cold = await list(iso, 'archive');
  assert.equal(hot.source, 'hot');
  assert.equal(cold.source, 'archive');
  assert.equal(cold.logs.length, logs.length);
  assert.deepEqual(cold.logs, hot.logs, 'lossless: every field, the same order');

  const bodies = new Set(cold.logs.map((l) => l.body));
  for (const l of logs) assert.ok(bodies.has(l.body), `body survived: ${JSON.stringify(l.body)}`);
});

test('archived rows carry template IDs that group lines of one kind', async () => {
  const { logs: rows } = await list(iso, 'archive');
  const byKind = new Map();
  for (const r of rows) {
    const kind = r.body.split(' ')[0];
    byKind.set(kind, new Set([...(byKind.get(kind) ?? []), r.template_id]));
  }
  for (const [kind, ids] of byKind) {
    if (kind === 'raw') continue; // bodies with the marker character are kept whole, one "template" each
    assert.equal(ids.size, 1, `${kind}: ${[...ids]}`);
    assert.notEqual([...ids][0], '0');
  }
  const order = [...byKind.get('order')][0];
  const payment = [...byKind.get('payment')][0];
  assert.notEqual(order, payment);
  assert.match(order, /^\d+$/, 'u64 as a decimal string');
});

test('filters and cursors behave the same on the archive', async () => {
  const errors = await list(iso, 'archive', '&min_severity=17');
  assert.equal(errors.logs.length, logs.filter((l) => l.severity === 17).length);
  assert.ok(errors.logs.every((l) => l.severity_number === 17));

  const traced = await list(iso, 'archive', `&trace_id=${traceId}`);
  assert.equal(traced.logs.length, logs.filter((l) => l.traceId).length);
  assert.ok(traced.logs.every((l) => l.trace_id === traceId));

  // Case folding is ASCII-only in both stores (ClickHouse `positionCaseInsensitive`).
  const folded = `ünïcödé RUN ${run.toUpperCase()}`;
  const text = await list(iso, 'archive', '', folded);
  assert.equal(text.logs.length, 8, 'ASCII letters match case-insensitively');
  assert.deepEqual(text.logs, (await list(iso, 'hot', '', folded)).logs, 'same matches as ClickHouse');
  const upper = `ÜNÏCÖDÉ RUN ${run}`;
  assert.deepEqual((await list(iso, 'archive', '', upper)).logs, (await list(iso, 'hot', '', upper)).logs);

  const paged = await pages('archive', 7);
  const hotPaged = await pages('hot', 7);
  assert.equal(paged.length, logs.length);
  assert.deepEqual(paged.map((l) => l.id), hotPaged.map((l) => l.id), 'same pages, same order');
});

test('archiving a window again replaces its segment and loses nothing', async () => {
  const again = archiveOnce();
  assert.ok(again.rows >= logs.length);
  const cold = await list(iso, 'archive');
  assert.deepEqual(cold.logs, (await list(iso, 'hot')).logs);
});

test('recent ranges are served from ClickHouse by default', async () => {
  const auto = await list(iso, 'auto');
  assert.equal(auto.source, 'hot', 'the last hour is inside the hot window');
  assert.equal(auto.logs.length, logs.length);
});

test('the archive is scoped to the caller: other tenants see nothing', async () => {
  const r = await viewer.json(`${ISO}/logs?${range}&source=archive`);
  assert.equal(r.status, 404, 'a demo viewer cannot read the isolation archive');
  const own = await viewer.json(`${DEMO}/logs?${range}&q=${run}&source=archive`);
  assert.equal(own.status, 200, JSON.stringify(own.body));
  assert.equal(own.body.logs.length, 0, 'isolation rows never appear in the demo scope');
});

test('lines of one kind can be selected by template ID, in both stores', async () => {
  const { logs: rows } = await list(iso, 'archive');
  const order = rows.find((r) => r.body.startsWith('order ')).template_id;
  const want = rows.filter((r) => r.template_id === order);
  assert.equal(want.length, 8, 'one in six lines is an order line');
  const cold = await list(iso, 'archive', `&template_id=${order}`);
  const hot = await list(iso, 'hot', `&template_id=${order}`);
  assert.deepEqual(cold.logs.map((l) => l.id), want.map((l) => l.id));
  assert.deepEqual(cold.logs, hot.logs, 'same rows from ClickHouse');
  assert.equal((await list(iso, 'archive', '&template_id=1')).logs.length, 0, 'an unknown kind matches nothing');

  const bad = await iso.json(`${ISO}/logs?${range}&template_id=-1`);
  assert.equal(bad.status, 400);
  assert.match(bad.body.message, /template_id must be a decimal u64/);
});

test('the catalog index skips nothing it should not: unknown traces and severities match nothing', async () => {
  assert.equal((await list(iso, 'archive', `&trace_id=${hex(16)}`)).logs.length, 0);
  assert.equal((await list(iso, 'archive', '&min_severity=21')).logs.length, 0);
  // Every indexed filter still finds its rows (no false negatives).
  const traced = await list(iso, 'archive', `&trace_id=${traceId}&min_severity=9`);
  assert.equal(traced.logs.length, logs.filter((l) => l.traceId).length);
});

test('ranges older than the hot window are served from the archive by default', async () => {
  const summary = archiveOnce(oldStart, oldEnd);
  assert.ok(summary.rows >= oldLogs.length, JSON.stringify(summary));

  const auto = await iso.json(`${ISO}/logs?from=${oldStart}&to=${oldEnd}&q=${oldRun}`);
  assert.equal(auto.status, 200, JSON.stringify(auto.body));
  assert.equal(auto.body.source, 'archive');
  assert.equal(auto.body.logs.length, oldLogs.length);
  const hot = await iso.json(`${ISO}/logs?from=${oldStart}&to=${oldEnd}&q=${oldRun}&source=hot`);
  assert.deepEqual(auto.body.logs, hot.body.logs, 'the archive returns what ClickHouse still holds');

  const traced = await iso.json(`${ISO}/logs?from=${oldStart}&to=${oldEnd}&trace_id=${oldTrace}`);
  assert.deepEqual(traced.body.logs.map((l) => l.body), [oldLogs[5].body], 'the trace bloom filter keeps the segment');

  // A range that crosses the hot window edge stitches both stores.
  const edge = await iso.json(`${ISO}/logs?from=${oldStart}&to=${oldEnd + 2n * DAY}&q=${oldRun}`);
  assert.equal(edge.status, 200, JSON.stringify(edge.body));
  assert.equal(edge.body.source, 'mixed');
  assert.equal(edge.body.logs.length, oldLogs.length);
});

test('the timeline shows archived logs for old ranges and says traces are not archived', async () => {
  const r = await iso.json(`${ISO}/timeline?from=${oldStart}&to=${oldEnd}`);
  assert.equal(r.status, 200, JSON.stringify(r.body));
  assert.equal(r.body.logs_source, 'archive');
  const mine = r.body.items.filter((i) => i.type === 'log' && i.service_name === oldService);
  assert.equal(mine.length, oldLogs.length, 'every archived log is a timeline item');
  assert.ok(r.body.completeness.notes.some((n) => /not archived/.test(n)), JSON.stringify(r.body.completeness.notes));

  const topo = await iso.json(`${ISO}/topology?from=${oldStart}&to=${oldEnd}`);
  assert.equal(topo.status, 200);
  assert.ok(topo.body.completeness.notes.some((n) => /not archived/.test(n)), 'the map explains its gap too');

  const recent = await iso.json(`${ISO}/timeline?${range}`);
  assert.equal(recent.body.logs_source, 'hot');
  assert.ok(!recent.body.completeness.notes.some((n) => /not archived/.test(n)), 'no note inside the hot window');
});

test('log reads that may use the archive accept 31 days; ClickHouse-only reads keep 7', async () => {
  const to = BigInt(Date.now()) * 1_000_000n;
  const days = (n) => `from=${to - BigInt(n) * DAY}&to=${to}`;
  for (const source of ['auto', 'archive']) {
    // The segment cap may end a page early; the cursor then continues where the read stopped.
    const found = [];
    let cursor = null;
    for (let i = 0; i < 50; i++) {
      const ok = await iso.json(`${ISO}/logs?${days(31)}&q=${oldRun}&source=${source}${cursor ? `&cursor=${cursor}` : ''}`);
      assert.equal(ok.status, 200, `${source}: ${JSON.stringify(ok.body)}`);
      assert.equal(ok.body.completeness.truncated, Boolean(ok.body.next_cursor));
      found.push(...ok.body.logs);
      cursor = ok.body.next_cursor;
      if (!cursor) break;
    }
    assert.equal(cursor, null, `${source}: paging ends`);
    assert.equal(found.length, oldLogs.length, `${source}: the old window is inside 31 days`);
    assert.equal(new Set(found.map((l) => l.id)).size, found.length, `${source}: no row twice across pages`);
    const tooLong = await iso.json(`${ISO}/logs?from=${to - 31n * DAY - 1n}&to=${to}&source=${source}`);
    assert.equal(tooLong.status, 400);
    assert.match(tooLong.body.message, /31 days/);
  }
  const hot = await iso.json(`${ISO}/logs?${days(8)}&source=hot`);
  assert.equal(hot.status, 400);
  assert.match(hot.body.message, /7 days/);
  const timeline = await iso.json(`${ISO}/timeline?${days(8)}`);
  assert.equal(timeline.status, 400, 'the timeline keeps its 7-day limit');
  const anon = await fetch(`${API}${ISO}/logs?${days(31)}`);
  assert.equal(anon.status, 401);
});

test('archive reads need a session and a valid source', async () => {
  const anon = await fetch(`${API}${ISO}/logs?${range}&source=archive`);
  assert.equal(anon.status, 401);
  const bad = await iso.json(`${ISO}/logs?${range}&source=cold`);
  assert.equal(bad.status, 400);
  assert.equal(bad.body.code, 'bad_request');
  assert.match(bad.body.message, /source must be auto, hot or archive/);
});
