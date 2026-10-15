import { describe, expect, it } from "vitest";
import { afterSignupPath, formText, safeNext, signupProblem } from "./signup";

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

describe("safeNext", () => {
  it("keeps same-app paths", () => {
    expect(safeNext("/app")).toBe("/app");
    expect(safeNext("/app/invite")).toBe("/app/invite");
    expect(safeNext("/app/demo/local-demo/tokens")).toBe("/app/demo/local-demo/tokens");
  });

  it("refuses anything that could leave the app", () => {
    for (const bad of [null, undefined, "", "/", "https://evil.example/app", "//evil.example/app", "/apple", "/app\\..\\x", "javascript:alert(1)", "/\\evil.example"]) {
      expect(safeNext(bad)).toBeNull();
    }
  });
});

describe("afterSignupPath", () => {
  it("opens first-workspace setup by default", () => {
    expect(afterSignupPath(null)).toBe("/app");
    expect(afterSignupPath("https://evil.example")).toBe("/app");
  });

  it("returns to a safe next path, e.g. an invite", () => {
    expect(afterSignupPath("/app/invite")).toBe("/app/invite");
  });
});
