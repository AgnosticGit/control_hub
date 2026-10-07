// `karbird-api migrate` runs on every production deploy (deploy/compose.storage.yaml). Unlike the
// dev `bootstrap` it must be repeatable without side effects: no demo seed, no token rotation, no
// files, and an existing owner's password is never reset from the deploy secret.
// Run: node --test tests/integration/migrate.test.mjs (with `just dev` running)
import { test, before } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { API, ORIGIN, creds, encodeLogs, nowNs, otlpHttp, ownerAccount, root } from './lib.mjs';

const owner = ownerAccount();
const skip = owner ? false : 'no .karbird/owner.env on this machine (bootstrap requires it)';
const exe = join(root, 'target', 'debug', process.platform === 'win32' ? 'karbird-api.exe' : 'karbird-api');
const credentialsPath = join(root, '.karbird', 'local-credentials.json');

function envFile(name) {
  return Object.fromEntries(
    readFileSync(join(root, '.karbird', name), 'utf8')
      .split(/\r?\n/)
      .filter((l) => l.trim() && !l.trim().startsWith('#') && l.includes('='))
      .map((l) => [l.slice(0, l.indexOf('=')).trim(), l.slice(l.indexOf('=') + 1).trim()]),
  );
}

function migrate(email, password) {
  const r = spawnSync(exe, ['migrate'], {
    cwd: root,
    encoding: 'utf8',
    timeout: 120_000,
    env: {
      ...process.env,
      ...envFile('infra.env'),
      // The assertion below observes the migration's informational outcome.
      // A developer's inherited `RUST_LOG=warn` must not make that result
      // disappear from the child process output.
      RUST_LOG: 'info',
      KARBIRD_OWNER_EMAIL: email,
      KARBIRD_OWNER_PASSWORD: password,
    },
  });
  assert.notEqual(r.status, null, `migrate did not exit\n${r.stderr}`);
  return { status: r.status, out: `${r.stdout}\n${r.stderr}` };
}

async function login(password) {
  const res = await fetch(`${API}/api/v1/session/login`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', origin: ORIGIN },
    body: JSON.stringify({ email: owner.email, password }),
  });
  return res.status;
}

before(() => {
  const build = spawnSync('cargo', ['build', '-q', '-p', 'karbird-api'], { cwd: root, stdio: 'inherit' });
  assert.equal(build.status, 0, 'cargo build');
});

test('migrate is repeatable and never resets the existing owner password', { skip }, async () => {
  // A deploy secret that differs from the real password: the account exists, so it is ignored.
  const deploySecret = `deploy-secret-${Date.now()}-long-enough`;
  const credentialsBefore = readFileSync(credentialsPath, 'utf8');
  const mtimeBefore = statSync(credentialsPath).mtimeMs;

  for (let i = 0; i < 2; i++) {
    const r = migrate(owner.email, deploySecret);
    assert.equal(r.status, 0, r.out);
    assert.match(r.out, /owner account exists; password left unchanged/);
    assert.ok(!r.out.includes(deploySecret), 'the password never appears in the output');
  }

  assert.equal(await login(deploySecret), 401, 'the deploy secret did not become the password');
  assert.equal(await login(owner.password), 200, 'the original password still works');

  assert.equal(readFileSync(credentialsPath, 'utf8'), credentialsBefore, 'migrate writes no credentials file');
  assert.equal(statSync(credentialsPath).mtimeMs, mtimeBefore);
  const accepted = await otlpHttp(
    'logs',
    encodeLogs('migrate-check', [{ body: 'after migrate', timeNs: nowNs() }]),
    { token: creds.ingest_tokens['demo-local'] },
  );
  assert.equal(accepted.status, 200, 'the existing ingest token was not rotated');
});

test('migrate refuses an owner password below the sign-up minimum before touching storage', { skip }, () => {
  const r = migrate(owner.email, 'short-pw');
  assert.notEqual(r.status, 0);
  assert.match(r.out, /KARBIRD_OWNER_PASSWORD must have at least 12 characters/);
  assert.ok(!r.out.includes('postgres migrations applied'), 'nothing ran before the check');
});

test('migrate refuses a missing owner configuration', () => {
  const r = migrate('', '');
  assert.notEqual(r.status, 0);
  assert.match(r.out, /KARBIRD_OWNER_EMAIL/);
});
