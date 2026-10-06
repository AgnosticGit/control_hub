import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, relative } from 'node:path';
import { exitCode, loadEnvFile, needsShell, parseEnv, quote, root, run, running, supervise, withFixtures } from '../lib.mjs';

test('root is the repository root', () => {
  assert.ok(existsSync(join(root, 'justfile')));
  assert.ok(existsSync(join(root, 'pnpm-workspace.yaml')));
});

test('parseEnv reads KEY=value lines, skipping comments and blanks', () => {
  const env = parseEnv(['# comment', '', 'A=1', '  B = two ', 'URL=postgres://u:p@h/db?x=1', 'EMPTY=', '=nokey', 'NOEQUALS', '\r'].join('\r\n'));
  assert.deepEqual(env, { A: '1', 'B ': ' two', URL: 'postgres://u:p@h/db?x=1', EMPTY: '' });
});

test('parseEnv keeps everything after the first = and does not interpret quotes', () => {
  assert.deepEqual(parseEnv('TOKEN=a=b=c\nQ="quoted"'), { TOKEN: 'a=b=c', Q: '"quoted"' });
});

test('loadEnvFile returns {} for a missing file and parses an existing one', () => {
  assert.deepEqual(loadEnvFile('.karbird/definitely-missing-unit-test.env'), {});
  const dir = mkdtempSync(join(tmpdir(), 'karbird-lib-test-'));
  try {
    const file = join(dir, 'x.env');
    writeFileSync(file, 'K=v\n');
    assert.deepEqual(loadEnvFile(relative(root, file)), { K: 'v' });
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('needsShell only for package-manager shims on Windows', () => {
  const expected = process.platform === 'win32';
  for (const cmd of ['pnpm', 'npm', 'npx']) assert.equal(needsShell(cmd), expected, cmd);
  for (const cmd of ['node', 'docker', 'cargo', 'pnpm.exe', 'sh']) assert.equal(needsShell(cmd), false, cmd);
});

test('quote leaves safe arguments alone and wraps the rest for the shell', () => {
  for (const a of ['build', '--filter', '@karbird/web', 'C:\\dev\\x', 'a=b', 'v1.2.3+x', 'http://h:1/p']) assert.equal(quote(a), a);
  assert.equal(quote('two words'), '"two words"');
  assert.equal(quote('a&b'), '"a&b"');
  assert.equal(quote('x|y'), '"x|y"');
  assert.equal(quote('say "hi"'), '"say \\"hi\\""');
  assert.equal(quote(''), '""');
});

test('run resolves on exit 0 and rejects with the command on failure', async () => {
  await run(process.execPath, ['-e', 'process.exit(0)'], { stdio: 'ignore' });
  await assert.rejects(run(process.execPath, ['-e', 'process.exit(3)'], { stdio: 'ignore' }), /exited with 3/);
});

test('withFixtures returns the body exit code and always tears down', async () => {
  const calls = [];
  const step = (name, result) => async () => {
    calls.push(name);
    if (result instanceof Error) throw result;
    return result;
  };
  const quiet = () => {};
  assert.equal(await withFixtures({ setup: step('setup'), body: step('body', 0), teardown: step('down'), log: quiet }), 0);
  assert.equal(await withFixtures({ setup: step('setup'), body: step('body', 3), teardown: step('down'), log: quiet }), 3);
  assert.deepEqual(calls, ['setup', 'body', 'down', 'setup', 'body', 'down']);

  calls.length = 0;
  const failedSetup = await withFixtures({ setup: step('setup', new Error('db down')), body: step('body', 0), teardown: step('down'), log: quiet });
  assert.equal(failedSetup, 1);
  assert.deepEqual(calls, ['setup', 'down'], 'a half-created fixture is still removed; tests do not run');

  const failedCleanup = await withFixtures({ setup: step('setup'), body: step('body', 0), teardown: step('down', new Error('x')), log: quiet });
  assert.equal(failedCleanup, 1, 'accounts left behind make the run fail');
  const both = await withFixtures({ setup: step('setup'), body: step('body', 4), teardown: step('down', new Error('x')), log: quiet });
  assert.equal(both, 4);
});

test('exitCode resolves with the exit code instead of rejecting', async () => {
  assert.equal(await exitCode(process.execPath, ['-e', 'process.exit(5)'], { stdio: 'ignore' }), 5);
  assert.equal(await exitCode(process.execPath, ['-e', ''], { stdio: 'ignore' }), 0);
});

test('running is empty for a group without a PID file', () => {
  assert.deepEqual(running('unit-test-group-that-never-exists'), []);
});

const idle = (name) => ({ name, cmd: process.execPath, args: ['-e', 'setInterval(() => {}, 1000)'] });
const pidsOf = (group) => JSON.parse(readFileSync(join(root, '.karbird', 'run', `${group}.json`), 'utf8'));

test('supervise: stopping one process on purpose keeps the others and the PID file current', async () => {
  const group = `unit-supervise-${process.pid}`;
  const exits = [];
  const sup = supervise([idle('a'), idle('b')], group, { exit: (c) => exits.push(c), signals: false, quiet: true });
  try {
    const firstA = sup.pid('a');
    assert.ok(sup.running('a') && sup.running('b'));
    assert.deepEqual(pidsOf(group).map((e) => e.name), ['a', 'b']);

    assert.equal(await sup.stop('a'), true);
    assert.equal(sup.running('a'), false);
    assert.equal(sup.running('b'), true);
    assert.deepEqual(pidsOf(group).map((e) => e.name), ['b']);
    assert.equal(await sup.stop('a'), false, 'stopping a stopped process is a no-op');

    sup.start(idle('a'));
    assert.notEqual(sup.pid('a'), firstA);
    assert.deepEqual(pidsOf(group).map((e) => e.name).sort(), ['a', 'b']);
    assert.throws(() => sup.start(idle('a')), /already running/);
    await new Promise((r) => setTimeout(r, 200));
    assert.deepEqual(exits, [], 'nothing shut down');
  } finally {
    await sup.stop('a');
    await sup.stop('b');
    rmSync(join(root, '.karbird', 'run', `${group}.json`), { force: true });
  }
});

test('supervise: a process exiting on its own still stops everything', async () => {
  const group = `unit-supervise-crash-${process.pid}`;
  let resolveExit;
  const exited = new Promise((r) => (resolveExit = r));
  const sup = supervise([idle('steady')], group, { exit: (c) => resolveExit(c), signals: false, quiet: true });
  sup.start({ name: 'crash', cmd: process.execPath, args: ['-e', 'process.exit(3)'] });
  assert.equal(await exited, 3);
  assert.equal(existsSync(join(root, '.karbird', 'run', `${group}.json`)), false, 'PID file removed on shutdown');
  assert.throws(() => sup.start(idle('late')), /shutting down/);
  await sup.stop('steady');
});
