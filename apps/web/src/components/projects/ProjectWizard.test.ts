import { describe, expect, it } from "vitest";
import { DEFAULT_ENVIRONMENTS } from "./ProjectWizard";

describe("first project wizard", () => {
  it("starts with development, staging and production as editable suggestions", () => {
    expect(DEFAULT_ENVIRONMENTS).toEqual(["Development", "Staging", "Production"]);
  });
});
