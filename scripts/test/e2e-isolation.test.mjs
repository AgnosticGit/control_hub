import assert from "node:assert/strict";
import { test } from "node:test";
import { createSpecIsolation } from "../../tests/e2e/spec-isolation.mjs";

test("a new spec drains and clears before its first test, without clearing between cases", () => {
  const steps = [];
  const prepare = createSpecIsolation({
    drain: () => steps.push("drain"),
    clear: () => steps.push("clear"),
  });

  prepare("first.spec.ts");
  steps.push("first body");
  prepare("first.spec.ts");
  steps.push("second body");
  prepare("next.spec.ts");
  steps.push("next body");

  assert.deepEqual(steps, ["drain", "clear", "first body", "second body", "drain", "clear", "next body"]);
});

test("a failed clear cannot let a spec start", () => {
  const prepare = createSpecIsolation({ drain: () => {}, clear: () => { throw new Error("cleanup failed"); } });
  assert.throws(() => prepare("first.spec.ts"), /cleanup failed/);
});
