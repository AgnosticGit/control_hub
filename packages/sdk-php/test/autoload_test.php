<?php

declare(strict_types=1);

putenv('KARBIRD_SERVICE_NAME=php-sdk-test');
putenv('KARBIRD_INGEST_TOKEN=chi_test-token');
putenv('OTEL_TRACES_EXPORTER=none');
putenv('OTEL_LOGS_EXPORTER=none');

require __DIR__ . '/../src/Bootstrap.php';
Karbird\Sdk\Bootstrap::configure();
require __DIR__ . '/../vendor/autoload.php';

$tracer = OpenTelemetry\API\Globals::tracerProvider()->getTracer('karbird-sdk-php-test');
$span = $tracer->spanBuilder('bootstrap.registered')->startSpan();
$span->end();

echo "PHP SDK autoload test passed\n";
