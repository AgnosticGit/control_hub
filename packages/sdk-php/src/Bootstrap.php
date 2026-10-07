<?php

declare(strict_types=1);

namespace Karbird\Sdk;

/**
 * Maps Karbird settings to the official OpenTelemetry PHP SDK configuration.
 * Call configure() before requiring vendor/autoload.php so the SDK autoloader
 * sees the OTLP settings. This class deliberately reads only declared KARBIRD_*
 * settings and the host/container identity files; it never reads application env
 * dumps, request headers, bodies, SQL or process command lines.
 */
final class Bootstrap
{
    /**
     * @param array<string, string|false>|null $environment Test seam; defaults to process environment.
     * @param callable(string): string|false|null $readFile Test seam for /proc identity files.
     */
    public static function configure(
        ?array $environment = null,
        ?callable $readFile = null,
        ?string $hostName = null,
    ): void {
        $get = static function (string $key) use ($environment): ?string {
            if ($environment !== null) {
                $value = $environment[$key] ?? false;
            } else {
                $value = getenv($key);
                if ($value === false && isset($_SERVER[$key]) && is_string($_SERVER[$key])) {
                    $value = $_SERVER[$key];
                }
            }

            return is_string($value) && $value !== '' ? $value : null;
        };
        $set = static function (string $key, string $value): void {
            putenv($key . '=' . $value);
            $_ENV[$key] = $value;
            $_SERVER[$key] = $value;
        };

        $serviceName = $get('OTEL_SERVICE_NAME') ?? $get('KARBIRD_SERVICE_NAME');
        if ($serviceName === null) {
            throw new \RuntimeException('Set KARBIRD_SERVICE_NAME or OTEL_SERVICE_NAME before starting Karbird SDK.');
        }
        if ($get('OTEL_SERVICE_NAME') === null) {
            $set('OTEL_SERVICE_NAME', $serviceName);
        }

        $endpoint = rtrim($get('OTEL_EXPORTER_OTLP_ENDPOINT') ?? $get('KARBIRD_OTLP_ENDPOINT') ?? 'http://127.0.0.1:4318', '/');
        self::setDefault($get, $set, 'OTEL_PHP_AUTOLOAD_ENABLED', 'true');
        self::setDefault($get, $set, 'OTEL_TRACES_EXPORTER', 'otlp');
        self::setDefault($get, $set, 'OTEL_LOGS_EXPORTER', 'otlp');
        self::setDefault($get, $set, 'OTEL_METRICS_EXPORTER', 'none');
        self::setDefault($get, $set, 'OTEL_EXPORTER_OTLP_PROTOCOL', 'http/protobuf');
        self::setDefault($get, $set, 'OTEL_EXPORTER_OTLP_ENDPOINT', $endpoint);
        self::setDefault($get, $set, 'OTEL_PROPAGATORS', 'tracecontext,baggage');

        $token = $get('KARBIRD_INGEST_TOKEN');
        if ($token !== null) {
            $headers = self::parsePairs($get('OTEL_EXPORTER_OTLP_HEADERS') ?? '');
            foreach (array_keys($headers) as $name) {
                if (strcasecmp($name, 'Authorization') === 0) {
                    unset($headers[$name]);
                }
            }
            $headers['Authorization'] = 'Bearer ' . $token;
            $encoded = [];
            foreach ($headers as $key => $value) {
                $encoded[] = rawurlencode($key) . '=' . rawurlencode($value);
            }
            $set('OTEL_EXPORTER_OTLP_HEADERS', implode(',', $encoded));
        } elseif ($get('OTEL_EXPORTER_OTLP_HEADERS') === null) {
            throw new \RuntimeException('Set KARBIRD_INGEST_TOKEN or OTEL_EXPORTER_OTLP_HEADERS before starting Karbird SDK.');
        }

        $attrs = self::parsePairs($get('OTEL_RESOURCE_ATTRIBUTES') ?? '');
        $host = $get('KARBIRD_HOST_NAME') ?? $hostName ?? gethostname();
        if (is_string($host) && $host !== '') {
            $attrs['host.name'] = $host;
        }
        $container = $get('KARBIRD_CONTAINER_ID') ?? self::containerId($readFile);
        if ($container !== null) {
            $attrs['container.id'] = $container;
        }
        foreach ([
            'service.namespace' => 'KARBIRD_SERVICE_NAMESPACE',
            'service.version' => 'KARBIRD_SERVICE_VERSION',
            'deployment.environment.name' => 'KARBIRD_ENVIRONMENT',
        ] as $attribute => $setting) {
            $value = $get($setting);
            if ($value !== null) {
                $attrs[$attribute] = $value;
            }
        }
        $encodedAttrs = [];
        foreach ($attrs as $key => $value) {
            $encodedAttrs[] = rawurlencode($key) . '=' . rawurlencode($value);
        }
        $set('OTEL_RESOURCE_ATTRIBUTES', implode(',', $encodedAttrs));
    }

    /** @param callable(string): string|false|null $readFile */
    private static function containerId(?callable $readFile): ?string
    {
        $read = $readFile ?? static function (string $path): string|false {
            $content = @file_get_contents($path);
            return $content === false ? false : $content;
        };
        foreach (['/proc/self/cgroup', '/proc/self/mountinfo'] as $path) {
            $contents = $read($path);
            if (!is_string($contents)) {
                continue;
            }
            foreach (preg_split('/\r?\n/', $contents) ?: [] as $line) {
                $pattern = $path === '/proc/self/cgroup'
                    ? '~[/-]([a-f0-9]{64})(?:\.scope)?$~i'
                    : '~/(?:docker/containers|containers|sandboxes)/([a-f0-9]{64})/~i';
                if (preg_match($pattern, $line, $matches) === 1) {
                    return strtolower($matches[1]);
                }
            }
        }

        return null;
    }

    /** @return array<string, string> */
    private static function parsePairs(string $value): array
    {
        $pairs = [];
        foreach (explode(',', $value) as $pair) {
            $separator = strpos($pair, '=');
            if ($separator === false || $separator === 0) {
                continue;
            }
            $key = rawurldecode(substr($pair, 0, $separator));
            $item = rawurldecode(substr($pair, $separator + 1));
            if ($key !== '') {
                $pairs[$key] = $item;
            }
        }

        return $pairs;
    }

    /** @param callable(string): ?string $get @param callable(string, string): void $set */
    private static function setDefault(callable $get, callable $set, string $key, string $value): void
    {
        if ($get($key) === null) {
            $set($key, $value);
        }
    }
}
