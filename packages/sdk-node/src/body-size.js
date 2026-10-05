// HTTP body sizes for traffic stats (`http.request.body.size` / `http.response.body.size`).
// Only the declared Content-Length is read: bodies themselves are never touched, and a
// chunked message without the header stays unknown rather than guessed.

const MAX_BODY_SIZE = Number.MAX_SAFE_INTEGER;

/** Content-Length as a non-negative integer, or undefined when absent or malformed. */
export function parseContentLength(value) {
  const raw = Array.isArray(value) ? value[0] : value;
  if (typeof raw === 'number') return Number.isSafeInteger(raw) && raw >= 0 ? raw : undefined;
  if (typeof raw !== 'string' || !/^\s*\d{1,16}\s*$/.test(raw)) return undefined;
  const n = Number(raw.trim());
  return n <= MAX_BODY_SIZE ? n : undefined;
}

/** Declared length of an outgoing message (ServerResponse or ClientRequest). */
function outgoingLength(msg) {
  if (typeof msg?.getHeader === 'function') {
    const set = parseContentLength(msg.getHeader('content-length'));
    if (set !== undefined) return set;
  }
  // Headers passed to `writeHead(status, {...})` or added by Node on `end(data)` are only in
  // the rendered header block.
  const rendered = typeof msg?._header === 'string' ? msg._header : '';
  const m = /\r\ncontent-length:[ \t]*(\d{1,16})[ \t]*\r\n/i.exec(rendered);
  return m ? parseContentLength(m[1]) : undefined;
}

/** Declared length of an incoming message (IncomingMessage). */
function incomingLength(msg) {
  return parseContentLength(msg?.headers?.['content-length']);
}

function isIncomingMessage(msg) {
  return typeof msg === 'object' && msg !== null && 'headers' in msg && typeof msg.getHeader !== 'function';
}

/**
 * Attributes for an HTTP span once the exchange is complete. Works for both sides:
 * server spans get (IncomingMessage, ServerResponse), client spans (ClientRequest, IncomingMessage).
 */
export function bodySizeAttributes(request, response) {
  const req = isIncomingMessage(request) ? incomingLength(request) : outgoingLength(request);
  const res = isIncomingMessage(response) ? incomingLength(response) : outgoingLength(response);
  const attrs = {};
  if (req !== undefined) attrs['http.request.body.size'] = req;
  if (res !== undefined) attrs['http.response.body.size'] = res;
  return attrs;
}
