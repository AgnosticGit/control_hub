# Instrument a PHP service

Karbird's PHP SDK is `karbird/sdk-php`, a thin setup layer for the official OpenTelemetry PHP SDK.
It exports OTLP traces and logs. Framework and client spans require `ext-opentelemetry` plus the
matching OpenTelemetry auto-instrumentation Composer libraries; choose integrations for the PHP
framework and clients actually used by the service.

## Install from this checkout

Add a path repository and require the local SDK:

```sh
composer config repositories.karbird path ../karbird/packages/sdk-php
composer require karbird/sdk-php:^0.1.0
```

Install the PHP extension and relevant instrumentations separately. For example, a Laravel app
can use `open-telemetry/opentelemetry-auto-laravel`; see the official
[PHP auto-instrumentation guide](https://opentelemetry.io/docs/zero-code/php/auto/) for extension
requirements and available Composer integrations.

Set `KARBIRD_SERVICE_NAME`, `KARBIRD_ENVIRONMENT`, `KARBIRD_OTLP_ENDPOINT` and
`KARBIRD_INGEST_TOKEN` in PHP-FPM's pool/container environment or secret manager. Configure
`KARBIRD_HOST_NAME` to the same value used by the Karbird agent on the machine. Include
`packages/sdk-php/src/Bootstrap.php` and call `Karbird\Sdk\Bootstrap::configure()` before
`vendor/autoload.php` so the OpenTelemetry SDK sees the Karbird settings at Composer startup.
Detailed steps and resource attribute behavior are in [the SDK README](../../packages/sdk-php/README.md).

## Infrastructure agent

The existing OpenTelemetry Collector agent in `deploy/agent/` is language agnostic. Run one agent
on each Docker host; it collects the host and all visible containers, including PHP-FPM, and does
not inspect PHP application memory, environment variables or request data. The PHP SDK adds
`host.name` and `container.id` to application spans so the map can join the application to those
machine/container metrics.
