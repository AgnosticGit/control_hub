import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { parseEnv } from '../lib.mjs';
import { SETTINGS_FILE, SETTINGS_TEMPLATE, ensureSettings, parseBootstrapArgs } from '../settings.mjs';

test('the template holds the four retention defaults the services expect', () => {
  assert.deepEqual(parseEnv(SETTINGS_TEMPLATE), {
    KARBIRD_LOGS_RETENTION_DAYS: '14',
    KARBIRD_SPANS_RETENTION_DAYS: '14',
    KARBIRD_EVENTS_RETENTION_DAYS: '30',
    KARBIRD_METRICS_RETENTION_DAYS: '14',
  });
  assert.match(SETTINGS_TEMPLATE, /--shrink-retention/);
});

test('settings.env is created once and never overwritten', () => {
  const dir = mkdtempSync(join(tmpdir(), 'karbird-settings-'));
  try {
    assert.equal(ensureSettings(dir), true);
    assert.equal(readFileSync(join(dir, SETTINGS_FILE), 'utf8'), SETTINGS_TEMPLATE);
    writeFileSync(join(dir, SETTINGS_FILE), 'KARBIRD_LOGS_RETENTION_DAYS=60\n');
    assert.equal(ensureSettings(dir), false);
    assert.equal(readFileSync(join(dir, SETTINGS_FILE), 'utf8'), 'KARBIRD_LOGS_RETENTION_DAYS=60\n', 'user edits survive');
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('bootstrap only shrinks retention when asked, and rejects anything else', () => {
  assert.deepEqual(parseBootstrapArgs([]), []);
  assert.deepEqual(parseBootstrapArgs(['--shrink-retention']), ['--shrink-retention']);
  assert.deepEqual(parseBootstrapArgs(['--shrink-retention', '--shrink-retention']), ['--shrink-retention']);
  for (const bad of ['--shrink', '--logs-retention-days=1', 'shrink-retention', '--out-dir']) {
    assert.throws(() => parseBootstrapArgs([bad]), /unknown argument/, bad);
  }
});
