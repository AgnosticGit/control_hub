import { describe, expect, it } from "vitest";
import { afterSignupPath, formText, signupProblem } from "./signup";

const PW = "correct horse battery";

describe("signupProblem", () => {
  it("accepts a valid form", () => {
    expect(signupProblem(" Alice@Example.com ", PW, PW)).toBeNull();
  });

  it("reports the first problem", () => {
    expect(signupProblem("alice", PW, PW)).toMatch(/valid email/);
    expect(signupProblem("alice@localhost", PW, PW)).toMatch(/valid email/);
    expect(signupProblem("a@example.com", "short", "short")).toMatch(/at least 12/);
    expect(signupProblem("a@example.com", "x".repeat(11), "x".repeat(11))).toMatch(/at least 12/);
    expect(signupProblem("alice@example.com", "Alice@Example.com", "Alice@Example.com")).toMatch(/not be your email/);
    expect(signupProblem("a@example.com", PW, `${PW}!`)).toMatch(/do not match/);
  });

  it("counts characters, not UTF-16 units or bytes", () => {
    const pw = "пароль".repeat(2);
    expect(signupProblem("a@example.com", pw, pw)).toBeNull();
    expect(signupProblem("a@example.com", "😀".repeat(11), "😀".repeat(11))).toMatch(/at least 12/);
  });
});

describe("formText", () => {
  it("reads text fields and treats missing or file fields as empty", () => {
    const f = new FormData();
    f.set("email", "a@example.com");
    f.set("upload", new Blob(["x"]));
    expect(formText(f, "email")).toBe("a@example.com");
    expect(formText(f, "missing")).toBe("");
    expect(formText(f, "upload")).toBe("");
  });
});

describe("afterSignupPath", () => {
  it("opens the integration guide of the first environment", () => {
    expect(afterSignupPath([{ id: "p-1", environments: [{ id: "e 1" }] }])).toBe("/app/p-1/e%201/integration");
  });

  it("falls back to the environment picker", () => {
    expect(afterSignupPath([])).toBe("/app");
    expect(afterSignupPath([{ id: "p", environments: [] }])).toBe("/app");
  });
});
