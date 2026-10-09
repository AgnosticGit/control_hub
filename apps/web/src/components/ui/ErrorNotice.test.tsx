import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { ErrorNotice } from "./ErrorNotice";

describe("ErrorNotice", () => {
  it("renders its message as an accessible alert", () => {
    const html = renderToStaticMarkup(<ErrorNotice>Could not save.</ErrorNotice>);

    expect(html).toContain('role="alert"');
    expect(html).toContain("Could not save.");
    expect(html).toContain("rounded-lg border border-error/40 bg-error/10");
  });

  it("preserves layout classes provided by the caller", () => {
    const html = renderToStaticMarkup(<ErrorNotice className="text-sm md:col-span-2">Invite failed.</ErrorNotice>);

    expect(html).toContain("text-sm md:col-span-2");
    expect(html).toContain("Invite failed.");
  });
});
