import { describe, expect, it } from "vitest";
import {
  asLogSource,
  combinedSource,
  flattenPages,
  hasTemplate,
  logQuery,
  sourceBadge,
  timelineArchiveNote,
  type LogFilters,
} from "./logs-query";

const base: LogFilters = { from: 10n, to: 20n, serviceId: null, minSeverity: 0, q: "", source: "auto", templateId: null };

function log(id: string) {
  return {
    id,
    time_ns: "1",
    observed_time_ns: "1",
    service_id: "s",
    service_name: "s",
    severity_number: 9,
    severity_text: "INFO",
    body: "b",
    template_id: "1",
    trace_id: null,
    span_id: null,
    attributes: {},
  };
}

describe("logQuery", () => {
  it("sends only the filters that are set and keeps nanoseconds exact", () => {
    expect(logQuery({ ...base, from: 1790000000000000001n, to: 1790000000000000002n }, null)).toEqual({
      from: "1790000000000000001",
      to: "1790000000000000002",
      limit: 1000,
    });
  });

  it("passes source, template, cursor and the other filters", () => {
    const q = logQuery(
      { ...base, serviceId: "svc", minSeverity: 17, q: "boom", source: "archive", templateId: "18446744073709551615" },
      "abc",
      50,
    );
    expect(q).toEqual({
      from: "10",
      to: "20",
      limit: 50,
      service_id: "svc",
      min_severity: 17,
      q: "boom",
      source: "archive",
      template_id: "18446744073709551615",
      cursor: "abc",
    });
  });

  it("omits the default source and the 'no template' ID", () => {
    const q = logQuery({ ...base, source: "auto", templateId: "0" }, null);
    expect(q).not.toHaveProperty("source");
    expect(q).not.toHaveProperty("template_id");
    expect(logQuery({ ...base, source: "hot" }, null)).toHaveProperty("source", "hot");
  });
});

describe("sources", () => {
  it("labels archive and mixed pages, and nothing for the hot store", () => {
    expect(sourceBadge("archive")?.text).toBe("From archive");
    expect(sourceBadge("mixed")?.text).toBe("Hot + archive");
    expect(sourceBadge("hot")).toBeNull();
    expect(sourceBadge(undefined)).toBeNull();
  });

  it("explains archived timeline logs and stays quiet for the hot store", () => {
    expect(timelineArchiveNote("archive")).toBe("Logs from archive · no traces");
    expect(timelineArchiveNote("mixed")).toBe("Older logs from archive · no traces there");
    expect(timelineArchiveNote("hot")).toBeNull();
    expect(timelineArchiveNote(undefined)).toBeNull();
  });

  it("narrows untyped values to a known source", () => {
    for (const v of ["hot", "archive", "mixed"]) expect(asLogSource(v)).toBe(v);
    for (const v of [undefined, null, "", "ARCHIVE", "cold", 1, {}]) expect(asLogSource(v)).toBeUndefined();
  });

  it("combines page sources: any mix is mixed", () => {
    expect(combinedSource([])).toBeUndefined();
    expect(combinedSource([{ source: "hot" }, { source: "hot" }])).toBe("hot");
    expect(combinedSource([{ source: "archive" }])).toBe("archive");
    expect(combinedSource([{ source: "hot" }, { source: "archive" }])).toBe("mixed");
    expect(combinedSource([{ source: "mixed" }])).toBe("mixed");
  });
});

describe("flattenPages", () => {
  it("keeps page order and drops rows repeated on later pages", () => {
    const out = flattenPages([{ logs: [log("a"), log("b")] }, { logs: [log("b"), log("c")] }]);
    expect(out.map((l) => l.id)).toEqual(["a", "b", "c"]);
    expect(flattenPages([])).toEqual([]);
  });
});

describe("hasTemplate", () => {
  it("is false for rows stored before templates and for malformed IDs", () => {
    expect(hasTemplate({ template_id: "123" })).toBe(true);
    expect(hasTemplate({ template_id: "18446744073709551615" })).toBe(true);
    for (const v of ["0", "", "-1", "01", "1e3", "abc"]) expect(hasTemplate({ template_id: v })).toBe(false);
  });
});
