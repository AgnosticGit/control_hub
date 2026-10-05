import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer, request as httpRequest } from 'node:http';
import { bodySizeAttributes, parseContentLength } from '../src/body-size.js';

test('parseContentLength accepts only non-negative integers', () => {
  assert.equal(parseContentLength('512'), 512);
  assert.equal(parseContentLength(' 0 '), 0);
  assert.equal(parseContentLength(['42', '43']), 42);
  assert.equal(parseContentLength(7), 7);
  for (const bad of [undefined, null, '', '-1', '1.5', '12abc', 'NaN', -3, 1.2, '99999999999999999', {}]) {
    assert.equal(parseContentLength(bad), undefined, `rejects ${JSON.stringify(bad)}`);
  }
});

/**
 * Runs one real HTTP exchange and returns the attributes computed on both sides once each
 * side has finished, exactly when the instrumentation calls `applyCustomAttributesOnSpan`.
 */
async function exchange({ requestBody, respond }) {
  let serverAttrs;
  const server = createServer((req, res) => {
    req.resume();
    req.on('end', () => {
      res.on('finish', () => (serverAttrs = bodySizeAttributes(req, res)));
      respond(res);
    });
  });
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  const { port } = server.address();
  try {
    const clientAttrs = await new Promise((resolve, reject) => {
      const headers = requestBody === undefined ? {} : { 'content-length': Buffer.byteLength(requestBody) };
      const req = httpRequest({ host: '127.0.0.1', port, method: 'POST', path: '/x', headers }, (res) => {
        res.resume();
        res.on('end', () => resolve(bodySizeAttributes(req, res)));
      });
      req.on('error', reject);
      req.end(requestBody);
    });
    return { serverAttrs, clientAttrs };
  } finally {
    server.close();
  }
}

test('both sides see request and response sizes from Content-Length', async () => {
  const { serverAttrs, clientAttrs } = await exchange({
    requestBody: 'x'.repeat(300),
    respond: (res) => {
      const body = 'y'.repeat(1200);
      res.writeHead(200, { 'content-type': 'text/plain', 'content-length': Buffer.byteLength(body) });
      res.end(body);
    },
  });
  const expected = { 'http.request.body.size': 300, 'http.response.body.size': 1200 };
  assert.deepEqual(serverAttrs, expected);
  assert.deepEqual(clientAttrs, expected);
});

test('a length set with setHeader or added by Node on end(data) is picked up', async () => {
  const { serverAttrs } = await exchange({
    requestBody: 'abc',
    respond: (res) => {
      res.statusCode = 200;
      res.end('hello');
    },
  });
  assert.deepEqual(serverAttrs, { 'http.request.body.size': 3, 'http.response.body.size': 5 });
});

test('chunked bodies without a declared length stay unknown instead of zero', async () => {
  const { serverAttrs, clientAttrs } = await exchange({
    requestBody: undefined,
    respond: (res) => {
      res.writeHead(200, { 'content-type': 'text/plain' });
      res.write('part1');
      res.end('part2');
    },
  });
  assert.equal(serverAttrs['http.response.body.size'], undefined);
  assert.equal(clientAttrs['http.response.body.size'], undefined);
});

test('never reads anything but the length', () => {
  const req = { headers: { 'content-length': '10', authorization: 'Bearer secret', cookie: 'a=b' } };
  const res = { getHeader: (h) => (h === 'content-length' ? '20' : 'secret'), _header: 'HTTP/1.1 200 OK\r\n\r\n' };
  assert.deepEqual(bodySizeAttributes(req, res), { 'http.request.body.size': 10, 'http.response.body.size': 20 });
  assert.deepEqual(bodySizeAttributes(undefined, null), {});
});
