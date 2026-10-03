import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseEnv } from '../lib.mjs';
import { derive, render } from '../deploy-config.mjs';

const deployDir = join(dirname(fileURLToPath(import.meta.url)), '..', '..', 'deploy');

const input = {
  CONTROLHUB_SECRET_SEED: 's'.repeat(40),
  CONTROLHUB_DOMAIN: 'controlhub.example.com',
  STORAGE_PRIVATE_IP: '10.0.0.12',
  CONTROLHUB_OWNER_EMAIL: 'me@example.com',
  CONTROLHUB_OWNER_PASSWORD: 'correct $horse "battery" staple',
  IMAGE_PREFIX: 'ghcr.io/acme',
  CONTROLHUB_VERSION: 'v1.2.3',
};

const files = render(input);
const storage = parseEnv(files['storage/.env']);
const app = parseEnv(files['app/.env']);

test('renders the same files on every deploy and different ones for another seed', () => {
  assert.deepEqual(render(input), files);
  const other = render({ ...input, CONTROLHUB_SECRET_SEED: 't'.repeat(40) });
  assert.notEqual(parseEnv(other['storage/.env']).CH_PG_API_PASSWORD, storage.CH_PG_API_PASSWORD);
});

test('every derived secret is distinct, long and safe for URLs, SQL and env files', () => {
  const secrets = Object.entries(storage).filter(([k]) => /PASSWORD|SECRET_KEY/.test(k)).map(([, v]) => v);
  assert.ok(secrets.length >= 14);
  assert.equal(new Set(secrets).size, secrets.length, 'no two purposes share a secret');
  for (const s of secrets) assert.match(s, /^[A-Za-z0-9_-]{32}$/);
  assert.equal(derive('x'.repeat(32), 'pg-api'), derive('x'.repeat(32), 'pg-api'));
});

test('the app host gets service credentials but never the admin passwords or the seed', () => {
  for (const k of ['CH_PG_ADMIN_PASSWORD', 'CH_CH_ADMIN_PASSWORD', 'CH_S3_ADMIN_SECRET_KEY', 'CONTROLHUB_SECRET_SEED']) {
    assert.equal(app[k], undefined, k);
  }
  for (const text of Object.values(files)) {
    assert.ok(!text.includes(input.CONTROLHUB_SECRET_SEED), 'the seed never leaves the runner');
  }
  assert.ok(!files['app/.env'].includes(input.CONTROLHUB_OWNER_PASSWORD), 'only migrate on the storage host needs it');
  assert.equal(app.CH_PG_API_PASSWORD, storage.CH_PG_API_PASSWORD, 'both hosts agree');
  assert.equal(app.CH_S3_API_SECRET_KEY, storage.CH_S3_API_SECRET_KEY);
  assert.equal(app.STORAGE_ADDR, '10.0.0.12');
  assert.equal(storage.STORAGE_BIND_IP, '10.0.0.12', 'storage ports bind to the private address only');
});

test('the owner password is written literally, single-quoted', () => {
  assert.ok(files['storage/owner.env'].includes(`CONTROLHUB_OWNER_PASSWORD='${input.CONTROLHUB_OWNER_PASSWORD}'`));
  assert.throws(() => render({ ...input, CONTROLHUB_OWNER_PASSWORD: "it's" }), /single quotes/);
  assert.throws(() => render({ ...input, CONTROLHUB_OWNER_PASSWORD: 'a\nb' }), /line breaks/);
});

test('S3 identities match the derived keys and stay scoped to the archive bucket', () => {
  const s3 = JSON.parse(files['storage/seaweedfs-s3.json']);
  const api = s3.identities.find((i) => i.name === 'api');
  assert.deepEqual(api.credentials, [{ accessKey: storage.CH_S3_API_ACCESS_KEY, secretKey: storage.CH_S3_API_SECRET_KEY }]);
  assert.ok(api.actions.every((a) => a.endsWith(':controlhub-archive')));
});

test('defaults: retention 14/14/30, ACME email = owner email; overrides are kept', () => {
  assert.deepEqual(
    [app.CONTROLHUB_LOGS_RETENTION_DAYS, app.CONTROLHUB_SPANS_RETENTION_DAYS, app.CONTROLHUB_EVENTS_RETENTION_DAYS],
    ['14', '14', '30'],
  );
  assert.equal(app.ACME_EMAIL, 'me@example.com');
  const custom = parseEnv(render({ ...input, CONTROLHUB_LOGS_RETENTION_DAYS: '7', ACME_EMAIL: 'ops@example.com' })['app/.env']);
  assert.equal(custom.CONTROLHUB_LOGS_RETENTION_DAYS, '7');
  assert.equal(custom.ACME_EMAIL, 'ops@example.com');
  assert.equal(storage.CONTROLHUB_LOGS_RETENTION_DAYS, '14', 'migrate applies the same retention the services check');
});

test('every variable a production compose file reads is rendered for its host or has a default', () => {
  const owner = parseEnv(files['storage/owner.env']);
  for (const [compose, env] of [
    ['compose.storage.yaml', { ...storage, ...owner }],
    ['compose.app.yaml', app],
  ]) {
    const text = readFileSync(join(deployDir, compose), 'utf8');
    const refs = [...text.matchAll(/(?<!\$)\$\{([A-Z0-9_]+)(:?-[^}]*)?/g)];
    assert.ok(refs.length > 5, compose);
    for (const [, name, fallback] of refs) {
      assert.ok(name in env || fallback?.includes('-'), `${compose} reads \${${name}}, which the deploy never renders`);
    }
  }
  const caddy = readFileSync(join(deployDir, 'Caddyfile'), 'utf8');
  for (const [, name] of caddy.matchAll(/\{\$([A-Z0-9_]+)\}/g)) assert.ok(name in app, `Caddyfile reads ${name}`);
});

test('rejects missing or malformed inputs with the variable name', () => {
  assert.throws(() => render({ ...input, CONTROLHUB_SECRET_SEED: '' }), /missing: CONTROLHUB_SECRET_SEED/);
  assert.throws(() => render({ ...input, CONTROLHUB_SECRET_SEED: 'short' }), /at least 32/);
  for (const domain of ['https://controlhub.example.com', 'controlhub.example.com/app', 'localhost', 'a b.com']) {
    assert.throws(() => render({ ...input, CONTROLHUB_DOMAIN: domain }), /CONTROLHUB_DOMAIN/, domain);
  }
  for (const ip of ['10.0.0', '10.0.0.256', 'storage.internal']) {
    assert.throws(() => render({ ...input, STORAGE_PRIVATE_IP: ip }), /STORAGE_PRIVATE_IP/, ip);
  }
  assert.throws(() => render({ ...input, IMAGE_PREFIX: 'ghcr.io/Acme' }), /IMAGE_PREFIX/);
  assert.throws(() => render({ ...input, CONTROLHUB_VERSION: 'v1 2' }), /CONTROLHUB_VERSION/);
  assert.throws(() => render({ ...input, CONTROLHUB_EVENTS_RETENTION_DAYS: 'forever' }), /whole number/);
});
