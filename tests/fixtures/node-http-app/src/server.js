// Plain Node HTTP service used as an integration fixture. It knows nothing about ControlHub;
// instrumentation comes only from the start script that `controlhub apply` rewrites.
const http = require('node:http');

const port = Number(process.env.PORT ?? 0);

const server = http.createServer((req, res) => {
  if (req.url === '/health') {
    res.writeHead(200, { 'content-type': 'text/plain' }).end('ok');
    return;
  }
  if (req.url?.startsWith('/orders/')) {
    const id = req.url.slice('/orders/'.length);
    const status = id === 'fail' ? 500 : 200;
    res.writeHead(status, { 'content-type': 'application/json' }).end(JSON.stringify({ id, ok: status === 200 }));
    return;
  }
  res.writeHead(404).end();
});

server.listen(port, '127.0.0.1', () => {
  const addr = server.address();
  console.log(`listening ${typeof addr === 'object' && addr ? addr.port : port}`);
});
