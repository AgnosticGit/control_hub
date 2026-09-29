import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, relative } from 'node:path';
import { loadEnvFile, needsShell, parseEnv, quote, root, run, running } from '../lib.mjs';

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
  assert.deepEqual(loadEnvFile('.controlhub/definitely-missing-unit-test.env'), {});
  const dir = mkdtempSync(join(tmpdir(), 'controlhub-lib-test-'));
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
  for (const a of ['build', '--filter', '@controlhub/web', 'C:\\dev\\x', 'a=b', 'v1.2.3+x', 'http://h:1/p']) assert.equal(quote(a), a);
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

test('running is empty for a group without a PID file', () => {
  assert.deepEqual(running('unit-test-group-that-never-exists'), []);
});
