// Copy-paste configuration for sending telemetry to one karbird environment.
// Pure functions: the page renders the text, it never evaluates or executes it.

export const TOKEN_PLACEHOLDER = "<your-ingestion-token>";
export const SDK_PACKAGE = "@karbird/sdk-node";
export const PHP_SDK_PACKAGE = "karbird/sdk-php";
/** `module.register`, used by the SDK's preload entry point, exists from this version on. */
export const SDK_MIN_NODE = "20.6";

export interface SnippetInput {
  /** Base OTLP/HTTP endpoint, e.g. `https://example.test/otlp`. */
  endpoint: string;
  /** Ingestion secret, or null to show a placeholder. */
  token: string | null;
  serviceName: string;
}

/** OTLP endpoint behind the web app's `/otlp` proxy, on the same public address as the site. */
export function otlpEndpoint(origin: string): string {
  return `${origin.replace(/\/+$/, "")}/otlp`;
}

/** Service names are resource attributes and end up in shell/env files: keep them boring. */
export function safeServiceName(raw: string): string {
  const s = raw
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 64);
  return s || "my-service";
}

const token = (i: SnippetInput) => i.token ?? TOKEN_PLACEHOLDER;

export function sdkInstall(): string {
  return `npm install ${SDK_PACKAGE}`;
}

/** Local monorepo installation until karbird/sdk-php is published to Packagist. */
export function phpSdkInstall(): string {
  return [
    `composer config repositories.karbird '{"type":"path","url":"../karbird/packages/sdk-php","options":{"symlink":true}}'`,
    `composer require ${PHP_SDK_PACKAGE}:^0.1.0`,
  ].join("\n");
}

/** PHP-FPM/container environment; PHP SDK bootstrap forwards these to OpenTelemetry. */
export function phpSdkEnv(i: SnippetInput): string {
  return [
    `KARBIRD_SERVICE_NAME=${safeServiceName(i.serviceName)}`,
    `KARBIRD_ENVIRONMENT=<environment-name>`,
    `KARBIRD_HOST_NAME=<same-host-name-as-karbird-collector>`,
    `KARBIRD_OTLP_ENDPOINT=${i.endpoint}`,
    `KARBIRD_INGEST_TOKEN=${token(i)}`,
  ].join("\n");
}

/** Must run before Composer's autoloader initializes the OpenTelemetry SDK. */
export function phpSdkBootstrap(): string {
  return [
    `<?php`,
    `require_once __DIR__ . '/vendor/karbird/sdk-php/src/Bootstrap.php';`,
    `Karbird\\Sdk\\Bootstrap::configure();`,
    `require_once __DIR__ . '/vendor/autoload.php';`,
  ].join("\n");
}

/** The variables `@karbird/sdk-node` reads at start-up. */
export function sdkEnv(i: SnippetInput): string {
  return [
    `KARBIRD_SERVICE_NAME=${safeServiceName(i.serviceName)}`,
    `KARBIRD_OTLP_ENDPOINT=${i.endpoint}`,
    `KARBIRD_INGEST_TOKEN=${token(i)}`,
  ].join("\n");
}

/** Preloading the SDK patches `http` and `pg` before the application imports them (CommonJS and ESM). */
export function sdkStart(entry = "server.js"): string {
  return `node --import ${SDK_PACKAGE}/register ${entry}`;
}

/** Trace-correlated logs and custom events from application code, through the instance `register` started. */
export function sdkCode(): string {
  return [
    `import { getKarbird } from "${SDK_PACKAGE}";`,
    "",
    "const karbird = getKarbird();",
    "",
    "// Logs are linked to the request being handled.",
    'karbird?.logger.info("order created", { "order.items": 3 });',
    "",
    "// Events appear on the timeline next to the traces.",
    'karbird?.events.emit("deployment.finished", { attributes: { version: "1.4.2" } });',
  ].join("\n");
}

/** One-shot check from a terminal: an empty protobuf export is a valid request. */
export function curlCheck(i: SnippetInput): string {
  return [
    `curl -i -X POST "${i.endpoint}/v1/traces" \\`,
    `  -H "authorization: Bearer ${token(i)}" \\`,
    '  -H "content-type: application/x-protobuf" \\',
    '  --data-binary ""',
  ].join("\n");
}
