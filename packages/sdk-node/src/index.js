// karbird integration for Node.js services.
//
// Thin, explicit wrapper over the official OpenTelemetry SDK:
// - traces and logs are exported with standard OTLP/HTTP protobuf;
// - HTTP server/client and `pg` are auto-instrumented;
// - request/response headers and bodies are never recorded;
// - process command-line arguments are not attached to the resource
//   (automatic resource detection is disabled on purpose); only host.name,
//   container.id and process.pid are added (see `resource.js`);
// - optional `request.started` / `request.completed` events are sent to
//   the karbird custom events endpoint;
// - span start times are refined below 1 ms (see `precise-start.js`).

import { hostname } from 'node:os';
import { context, trace } from '@opentelemetry/api';
import { logs, SeverityNumber } from '@opentelemetry/api-logs';
import { NodeSDK, tracing } from '@opentelemetry/sdk-node';
import { BatchLogRecordProcessor } from '@opentelemetry/sdk-logs';
import { OTLPTraceExporter } from '@opentelemetry/exporter-trace-otlp-proto';
import { OTLPLogExporter } from '@opentelemetry/exporter-logs-otlp-proto';
import { HttpInstrumentation } from '@opentelemetry/instrumentation-http';
import { PgInstrumentation } from '@opentelemetry/instrumentation-pg';
import { defaultResource, resourceFromAttributes } from '@opentelemetry/resources';
import { EventsClient } from './events.js';
import { PreciseStartSpanProcessor } from './precise-start.js';
import { infraAttributes } from './resource.js';

const GLOBAL_KEY = Symbol.for('karbird.sdk-node');
const OTEL_API_KEY = Symbol.for('opentelemetry.js.api.1');

const SEVERITY = {
  debug: SeverityNumber.DEBUG,
  info: SeverityNumber.INFO,
  warn: SeverityNumber.WARN,
  error: SeverityNumber.ERROR,
};

/** Returns the running instance (e.g. started by `register`), or null. */
export function getKarbird() {
  return globalThis[GLOBAL_KEY] ?? null;
}

/**
 * @param {import('./index.js').KarbirdOptions} options
 * @returns {import('./index.js').KarbirdHandle}
 */
export function startKarbird(options = {}) {
  const existing = globalThis[GLOBAL_KEY];
  if (existing) {
    process.emitWarning('karbird SDK is already started; reusing the existing instance', {
      code: 'KARBIRD_DOUBLE_INIT',
    });
    return existing;
  }
  if (!options.allowExistingOpenTelemetry && hasForeignOtelProvider()) {
    throw new Error(
      'An OpenTelemetry tracer provider is already registered in this process. ' +
        'Remove the other setup or pass allowExistingOpenTelemetry: true and configure exporters yourself.',
    );
  }

  const env = process.env;
  const serviceName = options.serviceName ?? env.KARBIRD_SERVICE_NAME ?? env.OTEL_SERVICE_NAME;
  if (!serviceName) {
    throw new Error('serviceName (or KARBIRD_SERVICE_NAME) is required');
  }
  const endpoint = trimSlash(options.endpoint ?? env.KARBIRD_OTLP_ENDPOINT ?? 'http://127.0.0.1:4318');
  const token = options.token ?? env.KARBIRD_INGEST_TOKEN;
  if (!token) {
    throw new Error('ingest token (KARBIRD_INGEST_TOKEN) is required');
  }
  const serviceNamespace = options.serviceNamespace ?? env.KARBIRD_SERVICE_NAMESPACE ?? '';
  const infra = infraAttributes({ hostName: options.hostName, containerId: options.containerId });
  const instanceId =
    options.serviceInstanceId ?? env.KARBIRD_SERVICE_INSTANCE_ID ?? `${infra['host.name'] ?? hostname()}-${process.pid}`;
  const headers = { authorization: `Bearer ${token}` };

  const attributes = {
    ...infra,
    'service.name': serviceName,
    'service.instance.id': instanceId,
  };
  if (serviceNamespace) attributes['service.namespace'] = serviceNamespace;
  const version = options.serviceVersion ?? env.KARBIRD_SERVICE_VERSION;
  if (version) attributes['service.version'] = version;
  const environment = options.environment ?? env.KARBIRD_ENVIRONMENT;
  if (environment) attributes['deployment.environment.name'] = environment;

  const ignorePaths = new Set(options.ignorePaths ?? ['/health', '/healthz', '/ready']);
  const events =
    options.requestEvents === false
      ? null
      : new EventsClient({
          url: `${endpoint}/api/v1/events/batch`,
          headers,
          serviceName,
          serviceNamespace,
          instanceId,
        });

  const httpInstrumentation = new HttpInstrumentation({
    ignoreIncomingRequestHook: (req) => ignorePaths.has(pathOf(req.url)),
    requestHook: (span, req) => {
      if (!events || !isIncoming(req)) return;
      const ctx = span.spanContext();
      events.push('request.started', {
        traceId: ctx.traceId,
        spanId: ctx.spanId,
        attributes: { 'http.request.method': String(req.method ?? ''), 'url.path': pathOf(req.url) },
      });
    },
    responseHook: (span, res) => {
      if (!events || !isServerResponse(res)) return;
      const ctx = span.spanContext();
      events.push('request.completed', {
        traceId: ctx.traceId,
        spanId: ctx.spanId,
        attributes: { 'http.response.status_code': String(res.statusCode) },
      });
    },
  });

  const sdk = new NodeSDK({
    autoDetectResources: false,
    resource: defaultResource().merge(resourceFromAttributes(attributes)),
    spanProcessors: [
      new PreciseStartSpanProcessor(),
      new tracing.BatchSpanProcessor(new OTLPTraceExporter({ url: `${endpoint}/v1/traces`, headers })),
    ],
    logRecordProcessors: [
      new BatchLogRecordProcessor({
        exporter: new OTLPLogExporter({ url: `${endpoint}/v1/logs`, headers }),
      }),
    ],
    // Machine and container metrics come from the Karbird agent, not from the SDK;
    // without an explicit empty list NodeSDK would start a default OTLP metrics exporter from env.
    metricReaders: [],
    instrumentations: [
      httpInstrumentation,
      new PgInstrumentation({ enhancedDatabaseReporting: false }),
    ],
  });
  sdk.start();
  events?.start();

  const otelLogger = logs.getLogger(serviceName);
  const emit = (level, message, attrs) => {
    otelLogger.emit({
      severityNumber: SEVERITY[level],
      severityText: level.toUpperCase(),
      body: String(message),
      attributes: attrs,
      context: context.active(),
    });
  };

  let stopped = false;
  const handle = {
    logger: {
      debug: (m, a) => emit('debug', m, a),
      info: (m, a) => emit('info', m, a),
      warn: (m, a) => emit('warn', m, a),
      error: (m, a) => emit('error', m, a),
    },
    events: {
      emit: (eventType, fields = {}) => {
        const span = trace.getActiveSpan();
        const ctx = span?.spanContext();
        events?.push(eventType, {
          traceId: ctx?.traceId,
          spanId: ctx?.spanId,
          ...fields,
        });
      },
    },
    diagnostics: () => ({
      serviceName,
      serviceInstanceId: instanceId,
      endpoint,
      events: events?.stats() ?? null,
    }),
    shutdown: async () => {
      if (stopped) return;
      stopped = true;
      await events?.shutdown();
      await sdk.shutdown();
      delete globalThis[GLOBAL_KEY];
    },
  };
  globalThis[GLOBAL_KEY] = handle;
  return handle;
}

function hasForeignOtelProvider() {
  const api = globalThis[OTEL_API_KEY];
  return Boolean(api && api.trace);
}

function trimSlash(s) {
  return s.endsWith('/') ? s.slice(0, -1) : s;
}

function pathOf(url) {
  if (!url) return '';
  const q = url.indexOf('?');
  return q === -1 ? url : url.slice(0, q);
}

function isIncoming(req) {
  return typeof req === 'object' && req !== null && 'httpVersion' in req && 'socket' in req && !('path' in req);
}

function isServerResponse(res) {
  return typeof res === 'object' && res !== null && 'statusCode' in res && 'req' in res && 'writeHead' in res;
}
