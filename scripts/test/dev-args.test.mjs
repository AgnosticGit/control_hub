import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseDevArgs } from '../dev-args.mjs';
import { DEFAULT_PUBLIC_URL } from '../public-url.mjs';

test('plain `just dev` is local only: no public URL', () => {
  assert.deepEqual(parseDevArgs([], {}), { web: true, release: false, build: true, publicUrl: null });
  // A public URL in the environment alone does not expose anything.
  assert.equal(parseDevArgs([], { KARBIRD_PUBLIC_URL: 'https://x.ngrok-free.dev' }).publicUrl, null);
});

test('`just dev ngrok` uses the configured public URL', () => {
  assert.equal(parseDevArgs(['ngrok'], {}).publicUrl, DEFAULT_PUBLIC_URL);
  assert.equal(parseDevArgs(['ngrok'], { KARBIRD_PUBLIC_URL: 'https://x.ngrok-free.dev' }).publicUrl, 'https://x.ngrok-free.dev');
});

test('existing flags still work', () => {
  assert.deepEqual(parseDevArgs(['--no-web', '--release', '--no-build'], {}), {
    web: false,
    release: true,
    build: false,
    publicUrl: null,
  });
});

test('refuses unknown arguments, ngrok without web, and ngrok with the URL switched off', () => {
  assert.throws(() => parseDevArgs(['ngrk'], {}), /unknown argument/);
  assert.throws(() => parseDevArgs(['ngrok', '--no-web'], {}), /--no-web/);
  assert.throws(() => parseDevArgs(['ngrok'], { KARBIRD_PUBLIC_URL: 'off' }), /needs a public URL/);
  assert.throws(() => parseDevArgs(['ngrok'], { KARBIRD_PUBLIC_URL: 'http://x.ngrok-free.dev' }), /https/);
});
