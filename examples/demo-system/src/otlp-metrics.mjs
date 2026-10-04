// Minimal OTLP ExportMetricsServiceRequest encoder (protobuf wire format), so the demo infra
// agent needs no protobuf dependency. Gauges and sums of doubles only. DEMO ONLY.

const AGGREGATION_CUMULATIVE = 2;

function varint(n) {
  let v = BigInt(n);
  const out = [];
  do {
    let b = Number(v & 0x7fn);
    v >>= 7n;
    if (v > 0n) b |= 0x80;
    out.push(b);
  } while (v > 0n);
  return Buffer.from(out);
}
const tag = (field, wire) => varint((field << 3) | wire);
const lenField = (field, buf) => Buffer.concat([tag(field, 2), varint(buf.length), buf]);
const strField = (field, s) => lenField(field, Buffer.from(String(s), 'utf8'));
const varintField = (field, n) => Buffer.concat([tag(field, 0), varint(n)]);
function fixed64Field(field, n) {
  const b = Buffer.alloc(8);
  b.writeBigUInt64LE(BigInt(n));
  return Buffer.concat([tag(field, 1), b]);
}
function doubleField(field, x) {
  const b = Buffer.alloc(8);
  b.writeDoubleLE(x);
  return Buffer.concat([tag(field, 1), b]);
}
const keyValues = (field, attrs) =>
  Object.entries(attrs ?? {}).map(([k, v]) => lenField(field, Buffer.concat([strField(1, k), lenField(2, strField(1, v))])));

function point(p) {
  return Buffer.concat([
    ...keyValues(7, p.attributes),
    ...(p.startNs ? [fixed64Field(2, p.startNs)] : []),
    fixed64Field(3, p.timeNs),
    doubleField(4, p.value),
  ]);
}

function metric(m) {
  const points = m.points.map((p) => lenField(1, point(p)));
  const data =
    m.kind === 'sum'
      ? lenField(7, Buffer.concat([...points, varintField(2, AGGREGATION_CUMULATIVE), varintField(3, m.monotonic ? 1 : 0)]))
      : lenField(5, Buffer.concat(points));
  return lenField(2, Buffer.concat([strField(1, m.name), ...(m.unit ? [strField(3, m.unit)] : []), data]));
}

/**
 * resources: [{ attributes, metrics: [{ name, unit?, kind: 'gauge' | 'sum', monotonic?, points: [{ attributes?, value, timeNs, startNs? }] }] }]
 */
export function encodeMetrics(resources, scopeName = 'karbird-demo-infra-agent') {
  return Buffer.concat(
    resources.map((r) =>
      lenField(
        1,
        Buffer.concat([
          lenField(1, Buffer.concat(keyValues(1, r.attributes))),
          lenField(2, Buffer.concat([lenField(1, strField(1, scopeName)), ...r.metrics.map(metric)])),
        ]),
      ),
    ),
  );
}
