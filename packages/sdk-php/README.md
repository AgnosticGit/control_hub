# Karbird SDK for PHP

`karbird/sdk-php` configures the official OpenTelemetry PHP SDK to export traces and logs to
Karbird using OTLP/HTTP protobuf. The machine/container agent is separate: run the existing
`deploy/agent/` Collector once on each Docker host; it reports host and container CPU, memory,
disk and network metrics no matter which language runs in those containers.

Automatic framework instrumentation requires PHP 8+, Composer autoloading and the
`ext-opentelemetry` PHP extension. Install an OpenTelemetry auto-instrumentation package for the
framework and clients the application uses (for example `open-telemetry/opentelemetry-auto-laravel`
for Laravel). Installing this Karbird package alone configures export; the extension and matching
instrumentation packages generate framework/client spans.

## Install

For a local checkout, add a Composer path repository in the application's `composer.json`:

```json
{
  "repositories": [
    { "type": "path", "url": "../karbird/packages/sdk-php", "options": { "symlink": true } }
  ],
  "require": {
    "karbird/sdk-php": "^0.1.0"
  }
}
```

Then run `composer update karbird/sdk-php`. Once published, install `karbird/sdk-php` from Packagist without the
path repository.

## Configure before Composer autoload

Set these in the PHP-FPM pool, container environment or process manager. PHP does not load `.env`
files automatically:

```dotenv
KARBIRD_SERVICE_NAME=orders
KARBIRD_ENVIRONMENT=production
KARBIRD_OTLP_ENDPOINT=https://karbird.example.com/otlp
KARBIRD_INGEST_TOKEN=chi_replace_with_environment_token
KARBIRD_HOST_NAME=vps-app
```

Before the application's normal `vendor/autoload.php` include, load the bootstrap:

```php
<?php
require_once __DIR__ . '/vendor/karbird/sdk-php/src/Bootstrap.php';
Karbird\Sdk\Bootstrap::configure();
require_once __DIR__ . '/vendor/autoload.php';
```

Or configure PHP's `auto_prepend_file` to a small application-owned file that includes
`Bootstrap.php` and calls `Bootstrap::configure()` before the application's Composer autoloader.
The PHP OpenTelemetry SDK initializes while Composer loads it, so this ordering matters.

Bootstrap maps `KARBIRD_*` settings to the standard `OTEL_*` configuration. It sets the
`service.name`, `deployment.environment.name`, `host.name` and `container.id`
resource attributes. `container.id` is read from `/proc/self/cgroup` or `/proc/self/mountinfo`;
you can explicitly provide `KARBIRD_CONTAINER_ID` if the runtime hides those paths. Existing
`OTEL_RESOURCE_ATTRIBUTES` are preserved and Karbird values take precedence for the attributes it
owns. No process arguments, environment dump, request headers/body or SQL are collected by the
bootstrap.

The SDK exports OTLP/HTTP protobuf to `<KARBIRD_OTLP_ENDPOINT>/v1/traces` and `/v1/logs`, with
`Authorization: Bearer <token>`. Metrics remain the responsibility of the host agent; automatic
application metrics are not currently stored by Karbird.

## Machine and container data

Copy `deploy/agent/` to each machine and run the Collector as documented in
[`docs/operations/deploy.md`](../../docs/operations/deploy.md). Give it the metrics-signal ingest
token and the same `KARBIRD_HOST_NAME` configured for the PHP service. It reads host metrics and
Docker stats through the existing read-only mounts and sends them directly to OTLP ingest. There is
no PHP-specific agent process or per-PHP-service collector.

## Test

Run `composer test` from this directory. The test exercises the environment mapping, token header,
existing attributes and Docker cgroup container ID detection without contacting Karbird.
