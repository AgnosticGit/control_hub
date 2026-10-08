import http from 'node:http';

export function postOversized(url, headers, size) {
  const u = new URL(url);
  return new Promise((resolve, reject) => {
    const req = http.request({
      host: u.hostname,
      port: u.port,
      path: u.pathname,
      method: 'POST',
      headers: { ...headers, 'content-length': String(size), expect:'100-continue' },
    });
    req.on('response', (res) => {
      res.resume();
      resolve(res.statusCode);
      req.destroy();
    });
    req.on('error', (e) => (e.code === 'ECONNRESET' || e.code === 'EPIPE' ? resolve(e.code) : reject(e)));
    req.setTimeout(15000,()=>req.destroy(new Error('oversized probe timed out')));
    req.flushHeaders();
    // Stream only after the server explicitly accepts the body; otherwise observe its final
    // header rejection without a timed write that can race a connection close on Windows.
    const chunk = Buffer.alloc(64 * 1024, 0x61);
    let sent = 0;
    const pump = () => {
      while (sent < size && !req.destroyed) {
        const n = Math.min(chunk.length, size - sent);
        sent += n;
        if (!req.write(n === chunk.length ? chunk : chunk.subarray(0, n))) {
          req.once('drain', pump);
          return;
        }
      }
      if (!req.destroyed) req.end();
    };
    req.once('continue',pump);
  });
}
