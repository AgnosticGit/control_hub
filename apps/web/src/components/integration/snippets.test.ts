import { describe, expect, it } from "vitest";
import {
  PHP_SDK_PACKAGE,
  SDK_PACKAGE,
  TOKEN_PLACEHOLDER,
  curlCheck,
  otlpEndpoint,
  phpSdkBootstrap,
  phpSdkEnv,
  phpSdkInstall,
  safeServiceName,
  sdkCode,
  sdkEnv,
  sdkInstall,
  sdkStart,
} from "./snippets";

const input = { endpoint: "https://ch.example/otlp", token: "chi_secret", serviceName: "Orders API" };

function envMap(text: string) {
  return Object.fromEntries(
    text
      .split("\n")
      .filter((l) => /^[A-Z_]+=/.test(l))
      .map((l) => [l.slice(0, l.indexOf("=")), l.slice(l.indexOf("=") + 1)]),
  );
}

describe("integration snippets", () => {
  it("points OTLP at the site's /otlp proxy", () => {
    expect(otlpEndpoint("https://slit-dandy-scallion.ngrok-free.dev")).toBe("https://slit-dandy-scallion.ngrok-free.dev/otlp");
    expect(otlpEndpoint("http://localhost:3000/")).toBe("http://localhost:3000/otlp");
  });

  it("configures exactly the variables the karbird SDK reads", () => {
    expect(envMap(sdkEnv(input))).toEqual({
      KARBIRD_SERVICE_NAME: "orders-api",
      KARBIRD_OTLP_ENDPOINT: "https://ch.example/otlp",
      KARBIRD_INGEST_TOKEN: "chi_secret",
    });
  });

  it("installs and preloads only the karbird SDK, never raw OpenTelemetry packages", () => {
    expect(sdkInstall()).toBe(`npm install ${SDK_PACKAGE}`);
    expect(sdkStart()).toBe(`node --import ${SDK_PACKAGE}/register server.js`);
    expect(sdkStart("dist/main.js")).toBe(`node --import ${SDK_PACKAGE}/register dist/main.js`);
    expect(sdkCode()).toContain(`import { getKarbird } from "${SDK_PACKAGE}";`);
    for (const text of [sdkInstall(), sdkStart(), sdkCode(), sdkEnv(input)]) {
      expect(text).not.toMatch(/@opentelemetry|OTEL_/);
    }
  });

  it("shows PHP Composer installation, environment and early bootstrap instructions", () => {
    expect(phpSdkInstall()).toContain(`composer require ${PHP_SDK_PACKAGE}:^0.1.0`);
    expect(phpSdkInstall()).toContain("../karbird/packages/sdk-php");
    expect(envMap(phpSdkEnv(input))).toEqual({
      KARBIRD_SERVICE_NAME: "orders-api",
      KARBIRD_ENVIRONMENT: "<environment-name>",
      KARBIRD_HOST_NAME: "<same-host-name-as-karbird-collector>",
      KARBIRD_OTLP_ENDPOINT: "https://ch.example/otlp",
      KARBIRD_INGEST_TOKEN: "chi_secret",
    });
    expect(phpSdkBootstrap()).toContain("Karbird\\Sdk\\Bootstrap::configure();");
    expect(phpSdkBootstrap().indexOf("Bootstrap::configure")).toBeLessThan(phpSdkBootstrap().indexOf("vendor/autoload.php"));
    expect(phpSdkEnv({ ...input, token: null })).toContain(TOKEN_PLACEHOLDER);
  });

  it("uses a placeholder until a token has been created", () => {
    const none = { ...input, token: null };
    for (const text of [sdkEnv(none), curlCheck(none)]) {
      expect(text).toContain(TOKEN_PLACEHOLDER);
      expect(text).not.toContain("chi_secret");
    }
  });

  it("puts the token and endpoint into the configuration and the connection check", () => {
    for (const text of [sdkEnv(input), curlCheck(input)]) {
      expect(text).toContain("chi_secret");
      expect(text).toContain("https://ch.example/otlp");
    }
    expect(curlCheck(input)).toContain('"https://ch.example/otlp/v1/traces"');
  });

  it("keeps the token out of the snippets that are not secret", () => {
    for (const text of [sdkInstall(), sdkStart(), sdkCode()]) expect(text).not.toContain("chi_secret");
  });

  it("sanitises service names so they cannot break the env file", () => {
    expect(safeServiceName("  Payments Service ")).toBe("payments-service");
    expect(safeServiceName("a\nKARBIRD_OTLP_ENDPOINT=https://evil")).toBe("a-karbird_otlp_endpoint-https-evil");
    expect(safeServiceName("$(rm -rf /)")).toBe("rm--rf");
    expect(safeServiceName("")).toBe("my-service");
    expect(safeServiceName("x".repeat(100))).toHaveLength(64);
    expect(sdkEnv({ ...input, serviceName: "a\nB=1" }).split("\n")).toHaveLength(3);
  });
});
