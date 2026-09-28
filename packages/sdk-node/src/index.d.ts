export interface ControlHubOptions {
  /** Defaults to CONTROLHUB_SERVICE_NAME / OTEL_SERVICE_NAME. */
  serviceName?: string;
  serviceNamespace?: string;
  serviceVersion?: string;
  serviceInstanceId?: string;
  /** Deployment environment label (e.g. `local-demo`). */
  environment?: string;
  /** Base OTLP/HTTP endpoint, defaults to CONTROLHUB_OTLP_ENDPOINT or http://127.0.0.1:4318. */
  endpoint?: string;
  /** Scoped ingestion token, defaults to CONTROLHUB_INGEST_TOKEN. */
  token?: string;
  /** Emit request.started / request.completed events (default true). */
  requestEvents?: boolean;
  /** Incoming request paths that are not traced. */
  ignorePaths?: string[];
  /** Allow starting when another OpenTelemetry provider is already registered. */
  allowExistingOpenTelemetry?: boolean;
}

export type Attributes = Record<string, string | number | boolean>;

export interface ControlHubLogger {
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

export interface ControlHubHandle {
  logger: ControlHubLogger;
  events: { emit(eventType: string, fields?: EventFields): void };
  diagnostics(): {
    serviceName: string;
    serviceInstanceId: string;
    endpoint: string;
    events: EventsStats | null;
  };
  shutdown(): Promise<void>;
}

export function getControlHub(): ControlHubHandle | null;
export function startControlHub(options?: ControlHubOptions): ControlHubHandle;
