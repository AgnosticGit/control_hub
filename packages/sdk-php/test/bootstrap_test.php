<?php

declare(strict_types=1);

require __DIR__ . '/../src/Bootstrap.php';

use Karbird\Sdk\Bootstrap;

function assertSameValue(string $expected, string|false $actual, string $message): void
{
    if ($expected !== $actual) {
        throw new RuntimeException($message . ': expected ' . $expected . ', got ' . var_export($actual, true));
    }
}

$id = str_repeat('a', 64);
Bootstrap::configure(
    [
        'KARBIRD_SERVICE_NAME' => 'checkout-php',
        'KARBIRD_ENVIRONMENT' => 'test',
        'KARBIRD_INGEST_TOKEN' => 'chi_test-token',
        'KARBIRD_OTLP_ENDPOINT' => 'https://karbird.example/otlp/',
        'OTEL_RESOURCE_ATTRIBUTES' => 'team=payments,host.name=old-host,process.pid=old',
    ],
    static fn (string $path): string|false => $path === '/proc/self/cgroup'
        ? "0::/system.slice/docker-{$id}.scope\n"
        : false,
    'php-worker-1',
);

assertSameValue('checkout-php', getenv('OTEL_SERVICE_NAME'), 'maps service name');
assertSameValue('https://karbird.example/otlp', getenv('OTEL_EXPORTER_OTLP_ENDPOINT'), 'normalizes OTLP base URL');
assertSameValue('true', getenv('OTEL_PHP_AUTOLOAD_ENABLED'), 'enables Composer SDK autoload');
assertSameValue('http/protobuf', getenv('OTEL_EXPORTER_OTLP_PROTOCOL'), 'uses OTLP HTTP protobuf');
assertSameValue('otlp', getenv('OTEL_TRACES_EXPORTER'), 'enables OTLP trace export');
assertSameValue('otlp', getenv('OTEL_LOGS_EXPORTER'), 'enables OTLP log export');
assertSameValue('none', getenv('OTEL_METRICS_EXPORTER'), 'leaves metrics to the host agent');
assertSameValue('team=payments,host.name=php-worker-1,process.pid=old,container.id=' . $id . ',deployment.environment.name=test', getenv('OTEL_RESOURCE_ATTRIBUTES'), 'preserves custom attributes and overlays host/container data');
assertSameValue('Authorization=Bearer%20chi_test-token', getenv('OTEL_EXPORTER_OTLP_HEADERS'), 'encodes ingest token as auth header');
$attributes = (string) getenv('OTEL_RESOURCE_ATTRIBUTES');
foreach (["team=payments", "host.name=php-worker-1", "container.id={$id}", "deployment.environment.name=test"] as $attribute) {
    if (!str_contains($attributes, $attribute)) {
        throw new RuntimeException('missing resource attribute ' . $attribute . ' from ' . $attributes);
    }
}

try {
    Bootstrap::configure([]);
    throw new RuntimeException('missing service name should fail');
} catch (RuntimeException $error) {
    if (!str_contains($error->getMessage(), 'KARBIRD_SERVICE_NAME')) {
        throw $error;
    }
}

echo "PHP SDK bootstrap tests passed\n";
