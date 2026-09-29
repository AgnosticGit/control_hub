import { describe, expect, it } from "vitest";
import { ApiRequestError, isUnauthenticated, scopedPath, scopedUrl } from "./api";

describe("scopedUrl", () => {
  it("builds the scoped API path with a query string", () => {
    expect(scopedUrl("p1", "e1", "timeline", { from: "1", to: "2" })).toBe(
      "/api/v1/projects/p1/environments/e1/timeline?from=1&to=2",
    );
  });

  it("omits an empty query", () => {
    expect(scopedUrl("p1", "e1", "live", {})).toBe("/api/v1/projects/p1/environments/e1/live");
  });

  it("encodes IDs so they cannot escape their path segment", () => {
    const url = scopedUrl("../../admin", "e/1?x=1#f", "topology", {});
    expect(url).toBe("/api/v1/projects/..%2F..%2Fadmin/environments/e%2F1%3Fx%3D1%23f/topology");
    const segments = new URL(url, "http://h").pathname.split("/").slice(1);
    expect(segments).toEqual(["api", "v1", "projects", "..%2F..%2Fadmin", "environments", "e%2F1%3Fx%3D1%23f", "topology"]);
  });

  it("encodes query values", () => {
    expect(scopedUrl("p", "e", "logs", { q: "a&b=c d" })).toBe("/api/v1/projects/p/environments/e/logs?q=a%26b%3Dc+d");
  });
});

describe("scopedPath", () => {
  it("maps to the OpenAPI path parameters", () => {
    expect(scopedPath("p1", "e1")).toEqual({ project_id: "p1", environment_id: "e1" });
  });
});

describe("isUnauthenticated", () => {
  it("is true only for API 401 errors", () => {
    expect(isUnauthenticated(new ApiRequestError(401, "unauthenticated", "no session"))).toBe(true);
    expect(isUnauthenticated(new ApiRequestError(403, "forbidden", "no"))).toBe(false);
    expect(isUnauthenticated(new Error("401"))).toBe(false);
    expect(isUnauthenticated({ status: 401 })).toBe(false);
    expect(isUnauthenticated(null)).toBe(false);
  });
});
