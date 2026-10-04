export interface KarbirdOptions {
  /** Defaults to KARBIRD_SERVICE_NAME / OTEL_SERVICE_NAME. */
  serviceName?: string;
  serviceNamespace?: string;
  serviceVersion?: string;
  /** Defaults to KARBIRD_SERVICE_INSTANCE_ID, then `<host.name>-<pid>`. */
  serviceInstanceId?: string;
  /** Deployment environment label (e.g. `local-demo`). */
  environment?: string;
  /** Base OTLP/HTTP endpoint, defaults to KARBIRD_OTLP_ENDPOINT or http://127.0.0.1:4318. */
  endpoint?: string;
  /** Scoped ingestion token, defaults to KARBIRD_INGEST_TOKEN. */
  token?: string;
  /** Emit request.started / request.completed events (default true). */
  requestEvents?: boolean;
  /** Incoming request paths that are not traced. */
  ignorePaths?: string[];
  /** `host.name`; defaults to KARBIRD_HOST_NAME, then the OS hostname. Must match the Karbird agent's host name. */
  hostName?: string;
  /** `container.id`; defaults to KARBIRD_CONTAINER_ID, then /proc/self/cgroup or mountinfo. */
  containerId?: string;
  /** Allow starting when another OpenTelemetry provider is already registered. */
  allowExistingOpenTelemetry?: boolean;
}

export type Attributes = Record<string, string | number | boolean>;

export interface KarbirdLogger {
  debug(message: string, attributes?: Attributes): void;
  info(message: string, attributes?: Attributes): void;
  warn(message: string, attributes?: Attributes): void;
  error(message: string, attributes?: Attributes): void;
}

export interface EventFields {
  entityId?: string;
  attributes?: Record<string, string>;
  payload?: unknown;
}

export interface EventsStats {
  queued: number;
  sent: number;
  rejected: number;
  dropped: number;
  retried: number;
  failed: number;
  pending: number;
}

export interface KarbirdHandle {
  logger: KarbirdLogger;
  events: { emit(eventType: string, fields?: EventFields): void };
  diagnostics(): {
    serviceName: string;
    serviceInstanceId: string;
    endpoint: string;
    events: EventsStats | null;
  };
  shutdown(): Promise<void>;
}

export function getKarbird(): KarbirdHandle | null;
export function startKarbird(options?: KarbirdOptions): KarbirdHandle;
