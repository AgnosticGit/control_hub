import { describe, expect, it } from "vitest";
import { passwordChangeProblem } from "./password";

const OLD = "old horse battery";
const NEW = "new horse battery staple";

describe("passwordChangeProblem", () => {
  it("accepts a valid change", () => {
    expect(passwordChangeProblem(OLD, NEW, NEW)).toBeNull();
  });

  it("reports the first problem", () => {
    expect(passwordChangeProblem("", NEW, NEW)).toMatch(/current password/);
    expect(passwordChangeProblem(OLD, "short", "short")).toMatch(/at least 12/);
    expect(passwordChangeProblem(OLD, OLD, OLD)).toMatch(/differ/);
    expect(passwordChangeProblem(OLD, NEW, `${NEW}!`)).toMatch(/do not match/);
  });

  it("counts characters, not UTF-16 units", () => {
    expect(passwordChangeProblem(OLD, "😀".repeat(11), "😀".repeat(11))).toMatch(/at least 12/);
    expect(passwordChangeProblem(OLD, "пароль".repeat(2), "пароль".repeat(2))).toBeNull();
  });
});
