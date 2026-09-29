import { test } from 'node:test';
import assert from 'node:assert/strict';
import { DEFAULT_PUBLIC_URL, allowedOrigins, publicUrl } from '../public-url.mjs';

test('publicUrl defaults to the ngrok address and normalises to an origin', () => {
  assert.equal(publicUrl({}), DEFAULT_PUBLIC_URL);
  assert.equal(publicUrl({ CONTROLHUB_PUBLIC_URL: ' https://Example.ngrok-free.dev/ ' }), 'https://example.ngrok-free.dev');
});

test('publicUrl can be switched off', () => {
  assert.equal(publicUrl({ CONTROLHUB_PUBLIC_URL: 'off' }), null);
  assert.equal(publicUrl({ CONTROLHUB_PUBLIC_URL: '' }), null);
});

test('publicUrl refuses anything but a bare https origin', () => {
  for (const bad of [
    'http://x.ngrok-free.dev',
    'https://x.ngrok-free.dev/app',
    'https://x.ngrok-free.dev/?a=1',
    'https://x.ngrok-free.dev/#h',
    'https://user:pw@x.ngrok-free.dev',
    'javascript:alert(1)',
    'not a url',
  ]) {
    assert.throws(() => publicUrl({ CONTROLHUB_PUBLIC_URL: bad }), undefined, bad);
  }
});

test('allowedOrigins keeps the local origins and appends the public one once', () => {
  const pub = 'https://x.ngrok-free.dev';
  assert.equal(allowedOrigins(pub, {}), `http://localhost:3000,http://127.0.0.1:3000,${pub}`);
  assert.equal(allowedOrigins(null, {}), 'http://localhost:3000,http://127.0.0.1:3000');
  assert.equal(allowedOrigins(pub, { CONTROLHUB_ALLOWED_ORIGINS: ` ${pub} , https://a.test,` }), `${pub},https://a.test`);
});
