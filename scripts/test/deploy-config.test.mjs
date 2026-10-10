import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseEnv } from '../lib.mjs';
import { derive, render } from '../deploy-config.mjs';

const deployDir = join(dirname(fileURLToPath(import.meta.url)), '..', '..', 'deploy');

const input = {
  SECRET_SEED: 's'.repeat(40),
  DOMAIN: 'karbird.example.com',
  STORAGE_PRIVATE_IP: '10.0.0.12',
  OWNER_EMAIL: 'me@example.com',
  OWNER_PASSWORD: 'correct $horse "battery" staple',
  IMAGE_PREFIX: 'ghcr.io/acme',
  KARBIRD_VERSION: 'v1.2.3',
};

const identity = {
  KARBIRD_GOOGLE_CLIENT_ID: 'google-client',
  KARBIRD_GOOGLE_CLIENT_SECRET: 'google-secret',
  KARBIRD_GOOGLE_REDIRECT_URL: 'https://karbird.example.com/api/v1/auth/google/callback',
  KARBIRD_RESEND_API_KEY: 'resend-secret',
  KARBIRD_MAIL_FROM: 'Karbird <invite@karbird.example.com>',
};

const files = render(input);
const storage = parseEnv(files['storage/.env']);
const app = parseEnv(files['app/.env']);

test('renders the same files on every deploy and different ones for another seed', () => {
  assert.deepEqual(render(input), files);
  const other = render({ ...input, SECRET_SEED: 't'.repeat(40) });
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
  for (const k of ['CH_PG_ADMIN_PASSWORD', 'CH_CH_ADMIN_PASSWORD', 'CH_S3_ADMIN_SECRET_KEY', 'SECRET_SEED', 'KARBIRD_SECRET_SEED']) {
    assert.equal(app[k], undefined, k);
  }
  for (const text of Object.values(files)) {
    assert.ok(!text.includes(input.SECRET_SEED), 'the seed never leaves the runner');
  }
  assert.ok(!files['app/.env'].includes(input.OWNER_PASSWORD), 'only migrate on the storage host needs it');
  assert.equal(app.CH_PG_API_PASSWORD, storage.CH_PG_API_PASSWORD, 'both hosts agree');
  assert.equal(app.CH_S3_API_SECRET_KEY, storage.CH_S3_API_SECRET_KEY);
  assert.equal(app.STORAGE_ADDR, '10.0.0.12');
  assert.equal(storage.STORAGE_BIND_IP, '10.0.0.12', 'storage ports bind to the private address only');
});

test('the owner password is written literally, single-quoted', () => {
  assert.ok(files['storage/owner.env'].includes(`KARBIRD_OWNER_PASSWORD='${input.OWNER_PASSWORD}'`));
  assert.throws(() => render({ ...input, OWNER_PASSWORD: "it's" }), /single quotes/);
  assert.throws(() => render({ ...input, OWNER_PASSWORD: 'a\nb' }), /line breaks/);
});

test('unprefixed inputs become the KARBIRD_* names the services and Caddy read', () => {
  const owner = parseEnv(files['storage/owner.env']);
  assert.equal(owner.KARBIRD_OWNER_EMAIL, 'me@example.com');
  assert.equal(app.KARBIRD_DOMAIN, 'karbird.example.com');
  for (const k of ['DOMAIN', 'OWNER_EMAIL', 'OWNER_PASSWORD']) {
    assert.equal(app[k], undefined, k);
    assert.equal(owner[k], undefined, k);
  }
});

test('the old prefixed input names are not accepted in place of the new ones', () => {
  const { SECRET_SEED, DOMAIN, OWNER_EMAIL, OWNER_PASSWORD, ...rest } = input;
  const old = {
    ...rest,
    KARBIRD_SECRET_SEED: SECRET_SEED,
    KARBIRD_DOMAIN: DOMAIN,
    KARBIRD_OWNER_EMAIL: OWNER_EMAIL,
    KARBIRD_OWNER_PASSWORD: OWNER_PASSWORD,
  };
  assert.throws(() => render(old), /missing: SECRET_SEED, DOMAIN, OWNER_EMAIL, OWNER_PASSWORD/);
});

test('a short owner password fails before anything reaches the servers', () => {
  assert.throws(() => render({ ...input, OWNER_PASSWORD: 'elevenchars' }), /at least 12 characters/);
  assert.throws(() => render({ ...input, OWNER_PASSWORD: 'пароль-ёжик' }), /at least 12 characters/);
  assert.doesNotThrow(() => render({ ...input, OWNER_PASSWORD: 'twelve chars' }));
});

test('S3 identities match the derived keys and stay scoped to the archive bucket', () => {
  const s3 = JSON.parse(files['storage/seaweedfs-s3.json']);
  const api = s3.identities.find((i) => i.name === 'api');
  assert.deepEqual(api.credentials, [{ accessKey: storage.CH_S3_API_ACCESS_KEY, secretKey: storage.CH_S3_API_SECRET_KEY }]);
  assert.ok(api.actions.every((a) => a.endsWith(':karbird-archive')));
});

test('defaults: retention 14/14/30/14, ACME email = owner email; overrides are kept', () => {
  assert.deepEqual(
    [app.KARBIRD_LOGS_RETENTION_DAYS, app.KARBIRD_SPANS_RETENTION_DAYS, app.KARBIRD_EVENTS_RETENTION_DAYS, app.KARBIRD_METRICS_RETENTION_DAYS],
    ['14', '14', '30', '14'],
  );
  const metrics = parseEnv(render({ ...input, KARBIRD_METRICS_RETENTION_DAYS: '3' })['app/.env']);
  assert.equal(metrics.KARBIRD_METRICS_RETENTION_DAYS, '3');
  assert.equal(app.ACME_EMAIL, 'me@example.com');
  const custom = parseEnv(render({ ...input, KARBIRD_LOGS_RETENTION_DAYS: '7', ACME_EMAIL: 'ops@example.com' })['app/.env']);
  assert.equal(custom.KARBIRD_LOGS_RETENTION_DAYS, '7');
  assert.equal(custom.ACME_EMAIL, 'ops@example.com');
  assert.equal(storage.KARBIRD_LOGS_RETENTION_DAYS, '14', 'migrate applies the same retention the services check');
});

test('retention overrides stay within the documented 2..3650 day range', () => {
  for (const value of ['0', '1', '3651', '9999']) {
    assert.throws(
      () => render({ ...input, KARBIRD_LOGS_RETENTION_DAYS: value }),
      /KARBIRD_LOGS_RETENTION_DAYS must be between 2 and 3650 days/,
      value,
    );
  }
  assert.doesNotThrow(() => render({ ...input, KARBIRD_LOGS_RETENTION_DAYS: '2' }));
  assert.doesNotThrow(() => render({ ...input, KARBIRD_LOGS_RETENTION_DAYS: '3650' }));
});

test('sign-up is open by default on a deployment, can be closed, and only reaches the app host', () => {
  assert.equal(app.KARBIRD_SIGNUP, 'open');
  assert.equal(app.KARBIRD_SIGNUP_PER_HOUR, '20');
  assert.equal(storage.KARBIRD_SIGNUP, undefined);
  const closed = parseEnv(render({ ...input, KARBIRD_SIGNUP: 'closed', KARBIRD_SIGNUP_PER_HOUR: '5' })['app/.env']);
  assert.equal(closed.KARBIRD_SIGNUP, 'closed');
  assert.equal(closed.KARBIRD_SIGNUP_PER_HOUR, '5');
  assert.equal(parseEnv(render({ ...input, KARBIRD_SIGNUP: '' })['app/.env']).KARBIRD_SIGNUP, 'open', 'an unset variable is empty');
  for (const bad of ['yes', 'Open', 'closed\nKARBIRD_ENV=dev']) {
    assert.throws(() => render({ ...input, KARBIRD_SIGNUP: bad }), /KARBIRD_SIGNUP must be open or closed/, bad);
  }
  for (const bad of ['0', '-1', 'many', '1e3']) {
    assert.throws(() => render({ ...input, KARBIRD_SIGNUP_PER_HOUR: bad }), /KARBIRD_SIGNUP_PER_HOUR/, bad);
  }
});

test('OAuth and Resend credentials stay on the app host and incomplete pairs fail', () => {
  const configured = render({ ...input, ...identity });
  const configuredApp = parseEnv(configured['app/.env']);
  assert.equal(configuredApp.KARBIRD_GOOGLE_CLIENT_ID, identity.KARBIRD_GOOGLE_CLIENT_ID);
  assert.equal(configuredApp.KARBIRD_GOOGLE_CLIENT_SECRET, identity.KARBIRD_GOOGLE_CLIENT_SECRET);
  assert.equal(configuredApp.KARBIRD_RESEND_API_KEY, identity.KARBIRD_RESEND_API_KEY);
  assert.equal(configuredApp.KARBIRD_MAIL_ENABLED, 'true');
  assert.ok(!configured['storage/.env'].includes(identity.KARBIRD_GOOGLE_CLIENT_SECRET));
  assert.ok(!configured['storage/.env'].includes(identity.KARBIRD_RESEND_API_KEY));
  assert.throws(() => render({ ...input, KARBIRD_GOOGLE_CLIENT_ID: 'only-one' }), /Google OAuth requires/);
  assert.throws(() => render({ ...input, KARBIRD_RESEND_API_KEY: 'only-one' }), /email delivery requires/);
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

test('storage deployment restores ClickHouse defaults and disables internal system log tables', () => {
  const compose = readFileSync(join(deployDir, 'compose.storage.yaml'), 'utf8');
  const workflow = readFileSync(join(deployDir, '..', '.github', 'workflows', 'deploy.yml'), 'utf8');
  const settings = readFileSync(join(deployDir, 'clickhouse-system-logs.xml'), 'utf8');

  assert.match(compose, /\.\/clickhouse-system-logs\.xml:\/etc\/clickhouse-server\/config\.d\/karbird-system-logs\.xml:ro/);
  assert.match(compose, /karbird\.clickhouse-config-sha: \$\{KARBIRD_CLICKHOUSE_CONFIG_SHA:-local\}/);
  assert.match(workflow, /deploy\/clickhouse-system-logs\.xml/);
  assert.match(workflow, /KARBIRD_CLICKHOUSE_CONFIG_SHA=.*sha256sum clickhouse-system-logs\.xml/);
  assert.doesNotMatch(settings, /<background_schedule_pool_size>|<background_schedule_pool_initial_size>|<max_thread_pool_size>/);
  for (const log of ['query_log', 'query_thread_log', 'query_metric_log', 'processors_profile_log', 'text_log', 'trace_log', 'metric_log', 'asynchronous_metric_log', 'part_log', 'error_log', 'crash_log']) {
    assert.match(settings, new RegExp(`<${log} remove="1"\\s*\\/>`), `${log} is disabled`);
  }
});

test('full data reset is opt-in and removes Compose volumes on both hosts before deployment', () => {
  const workflow = readFileSync(join(deployDir, '..', '.github', 'workflows', 'deploy.yml'), 'utf8');

  assert.match(workflow, /reset_data:\s*\n\s*description:.*(reset|delete|clear).*(data|volumes)/i);
  assert.match(workflow, /reset_data:\s*\n\s*description:[^\n]*\n\s*type: boolean\n\s*default: false/);
  assert.match(workflow, /name: Reset Karbird data\n\s+if: inputs\.reset_data/);
  assert.match(workflow, /ssh app .*docker compose -f compose\.app\.yaml down --volumes --remove-orphans/);
  assert.match(workflow, /ssh storage .*docker compose -f compose\.storage\.yaml down --volumes --remove-orphans/);
  assert.ok(workflow.indexOf('name: Reset Karbird data') < workflow.indexOf('name: Storage host'));
});

test('rejects missing or malformed inputs with the variable name', () => {
  assert.throws(() => render({ ...input, SECRET_SEED: '' }), /missing: SECRET_SEED/);
  assert.throws(() => render({ ...input, SECRET_SEED: 'short' }), /SECRET_SEED must be at least 32/);
  for (const domain of ['https://karbird.example.com', 'karbird.example.com/app', 'localhost', 'a b.com']) {
    assert.throws(() => render({ ...input, DOMAIN: domain }), /^Error: DOMAIN must be a host name/, domain);
  }
  for (const ip of ['10.0.0', '10.0.0.256', 'storage.internal']) {
    assert.throws(() => render({ ...input, STORAGE_PRIVATE_IP: ip }), /STORAGE_PRIVATE_IP/, ip);
  }
  assert.throws(() => render({ ...input, IMAGE_PREFIX: 'ghcr.io/Acme' }), /IMAGE_PREFIX/);
  assert.throws(() => render({ ...input, KARBIRD_VERSION: 'v1 2' }), /KARBIRD_VERSION/);
  assert.throws(() => render({ ...input, KARBIRD_EVENTS_RETENTION_DAYS: 'forever' }), /whole number/);
  assert.throws(() => render({ ...input, KARBIRD_METRICS_RETENTION_DAYS: '1.5' }), /whole number/);
});
