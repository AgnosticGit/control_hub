// `controlhub` CLI against the fixture project and the running stack:
// discover → plan → apply (dry run, apply, idempotent) → run the instrumented app → verify → context.
// Uses the isolation tenant so the demo environment stays clean.
// Run: cargo build -p controlhub-cli && node --test "tests/integration/*.test.mjs"
import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { randomUUID } from 'node:crypto';
import { API, OTLP, root, Session, ISO } from './lib.mjs';

const bin = join(root, 'target', 'debug', process.platform === 'win32' ? 'controlhub.exe' : 'controlhub');
const sdkPath = join(root, 'packages', 'sdk-node');
const service = `fixture-${randomUUID().slice(0, 8)}`;
let dir;
let work;
let apiToken;
let ingestToken;
let app;
const tokenIds = [];
let owner;

function cli(args, env = {}) {
  const r = spawnSync(bin, args, {
    env: {
      ...process.env,
      CONTROLHUB_API_URL: API,
      CONTROLHUB_OTLP_ENDPOINT: OTLP,
      CONTROLHUB_API_TOKEN: apiToken ?? '',
      CONTROLHUB_INGEST_TOKEN: ingestToken ?? '',
      ...env,
    },
    encoding: 'utf8',
    timeout: 120_000,
  });
  let json = null;
  try {
    json = JSON.parse(r.stdout);
  } catch {
    // non-JSON output (errors go to stderr)
  }
  return { code: r.status, json, stdout: r.stdout, stderr: r.stderr };
}

before(async () => {
  const build = spawnSync('cargo', ['build', '-q', '-p', 'controlhub-cli'], { cwd: root, stdio: 'inherit' });
  assert.equal(build.status, 0, 'cargo build -p controlhub-cli');
  work = mkdtempSync(join(tmpdir(), 'controlhub-cli-'));
  dir = join(work, 'shop-api');
  cpSync(join(root, 'tests', 'fixtures', 'node-http-app'), dir, { recursive: true });

  owner = new Session();
  await owner.login('isolation@controlhub.local');
  for (const kind of ['api', 'ingest']) {
    const r = await owner.mutate(`${ISO}/tokens`, { kind, name: `cli-test-${kind}`, expires_in_days: 1 });
    assert.equal(r.status, 200, JSON.stringify(r.body));
    tokenIds.push(r.body.token.id);
    if (kind === 'api') apiToken = r.body.secret;
    else ingestToken = r.body.secret;
  }
});

after(async () => {
  app?.kill();
  for (const id of tokenIds) await owner.mutate(`${ISO}/tokens/${id}/revoke`, { reason: 'cli test cleanup' });
  if (work) rmSync(work, { recursive: true, force: true });
});

test('discover reports the project without reading .env values', () => {
  writeFileSync(join(dir, '.env'), 'CONTROLHUB_INGEST_TOKEN=chi_must-not-be-read\n');
  const r = cli(['discover', '--path', dir]);
  assert.equal(r.code, 0, r.stderr);
  assert.equal(r.json.ecosystem, 'node');
  assert.equal(r.json.entry, 'src/server.js');
  assert.deepEqual(r.json.env_files, ['.env']);
  assert.equal(r.json.env_ignored_by_git, false);
  assert.ok(!r.stdout.includes('must-not-be-read'));
});

test('plan and apply: dry run, apply, idempotent re-apply, drift refusal', () => {
  const planFile = join(work, 'plan.json');
  const plan = cli(['plan', '--path', dir, '--service-name', service, '--environment', 'isolation-test', '--sdk-spec', `file:${sdkPath}`, '--out', planFile]);
  assert.equal(plan.code, 0, plan.stderr);
  assert.deepEqual(plan.json.steps.map((s) => s.file), ['package.json', '.env.example', '.gitignore']);

  const before = readFileSync(join(dir, 'package.json'), 'utf8');
  const dry = cli(['apply', '--path', dir, '--plan', planFile]);
  assert.equal(dry.code, 2, 'dry run needs --yes');
  assert.match(dry.stderr, /\+\s+"start": "node --import @controlhub\/sdk-node\/register src\/server.js"/);
  assert.equal(readFileSync(join(dir, 'package.json'), 'utf8'), before, 'dry run writes nothing');

  const applied = cli(['apply', '--path', dir, '--plan', planFile, '--yes']);
  assert.equal(applied.code, 0, applied.stderr);
  assert.ok(applied.json.steps.every((s) => s.state === 'applied'));
  const pkg = JSON.parse(readFileSync(join(dir, 'package.json'), 'utf8'));
  assert.equal(pkg.scripts.start, 'node --import @controlhub/sdk-node/register src/server.js');
  assert.equal(pkg.dependencies['@controlhub/sdk-node'], `file:${sdkPath}`);
  const envExample = readFileSync(join(dir, '.env.example'), 'utf8');
  assert.match(envExample, new RegExp(`CONTROLHUB_SERVICE_NAME=${service}`));
  assert.match(envExample, /CONTROLHUB_INGEST_TOKEN=\n/);
  assert.ok(!envExample.includes('chi_'), 'no real token in .env.example');
  assert.match(readFileSync(join(dir, '.gitignore'), 'utf8'), /^\.env$/m);

  const again = cli(['apply', '--path', dir, '--plan', planFile, '--yes']);
  assert.equal(again.code, 0);
  assert.ok(again.json.steps.every((s) => s.state === 'already_applied'));

  writeFileSync(join(work, 'stale.json'), JSON.stringify({ ...plan.json, steps: plan.json.steps.map((s) => ({ ...s, after: s.after + '\n' })) }));
  const stale = cli(['apply', '--path', dir, '--plan', join(work, 'stale.json'), '--yes']);
  assert.equal(stale.code, 2, 'files changed since that plan');
  assert.deepEqual(stale.json.changed_files.sort(), ['.env.example', '.gitignore', 'package.json']);

  const evil = { ...plan.json, steps: [{ ...plan.json.steps[0], file: '../escape.json' }] };
  writeFileSync(join(work, 'evil.json'), JSON.stringify(evil));
  const rejected = cli(['apply', '--path', dir, '--plan', join(work, 'evil.json'), '--yes']);
  assert.equal(rejected.code, 1);
  assert.match(rejected.stderr, /not allowed to modify/);
});

test('the instrumented fixture app is verified end to end and appears in context', async () => {
  // Stand-in for `npm install`: link the local SDK checkout (it is not published yet).
  mkdirSync(join(dir, 'node_modules', '@controlhub'), { recursive: true });
  symlinkSync(sdkPath, join(dir, 'node_modules', '@controlhub', 'sdk-node'), 'junction');

  const start = JSON.parse(readFileSync(join(dir, 'package.json'), 'utf8')).scripts.start.split(' ');
  assert.equal(start[0], 'node');
  app = spawn(process.execPath, start.slice(1), {
    cwd: dir,
    env: {
      ...process.env,
      PORT: '0',
      CONTROLHUB_SERVICE_NAME: service,
      CONTROLHUB_ENVIRONMENT: 'isolation-test',
      CONTROLHUB_OTLP_ENDPOINT: OTLP,
      CONTROLHUB_INGEST_TOKEN: ingestToken,
    },
  });
  let stderr = '';
  app.stderr.on('data', (d) => (stderr += d));
  const port = await new Promise((resolve, reject) => {
    const t = setTimeout(() => reject(new Error(`app did not start: ${stderr}`)), 20_000);
    app.stdout.on('data', (d) => {
      const m = /listening (\d+)/.exec(String(d));
      if (m) {
        clearTimeout(t);
        resolve(Number(m[1]));
      }
    });
    app.on('exit', (code) => reject(new Error(`app exited ${code}: ${stderr}`)));
  });
  for (let i = 0; i < 10; i++) await fetch(`http://127.0.0.1:${port}/orders/${i}`);
  await fetch(`http://127.0.0.1:${port}/orders/fail`);

  const verify = cli(['verify', '--path', dir, '--service-name', service, '--require-app', '--timeout-secs', '60']);
  assert.equal(verify.code, 0, `${verify.stdout}\n${verify.stderr}`);
  assert.equal(verify.json.scope.project_id, 'isolation');
  assert.equal(verify.json.pipeline.broker_acknowledged, true);
  assert.equal(typeof verify.json.pipeline.visible_after_ms, 'number');
  assert.equal(verify.json.app.observed, true);
  assert.ok(verify.json.app.requests >= 11);

  const ctx = cli(['context', '--minutes', '10']);
  assert.equal(ctx.code, 0, ctx.stderr);
  assert.match(ctx.json.notice, /untrusted/);
  assert.equal(ctx.json.scope.environment_id, 'isolation-test');
  const svc = ctx.json.services.find((s) => s.name === service);
  assert.ok(svc && svc.requests >= 11 && svc.errors >= 1, JSON.stringify(ctx.json.services));
  assert.ok(ctx.json.recent_error_traces.some((t) => t.root_service === service));

  const doctor = cli(['doctor', '--path', dir]);
  assert.equal(doctor.code, 0, doctor.stdout);
  assert.ok(doctor.json.checks.every((c) => c.ok), JSON.stringify(doctor.json.checks));
});

test('CLI refuses misplaced tokens and cleartext remote URLs', () => {
  const swapped = cli(['context'], { CONTROLHUB_API_TOKEN: ingestToken });
  assert.equal(swapped.code, 1);
  assert.match(swapped.stderr, /must be a `cha_…` token/);
  const remote = cli(['context', '--api-url', 'http://hub.example.com']);
  assert.equal(remote.code, 1);
  assert.match(remote.stderr, /refusing to send tokens over plain http/);
});
