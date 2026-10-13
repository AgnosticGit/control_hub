import { test } from "node:test";
import assert from "node:assert/strict";
import { stopAll } from "../stop.mjs";

test("stop terminates dev and demo processes without changing Docker state", async () => {
  const calls = [];
  const logs = [];
  const code = await stopAll({
    stopProcessGroup: (group) => {
      calls.push(group);
      return group === "dev" ? [{ name: "karbird-api", pid: 123 }] : [];
    },
    stopContainers: async () => calls.push("containers"),
    hasInfrastructureConfig: () => true,
    log: (message) => logs.push(message),
  });

  assert.equal(code, 0);
  assert.deepEqual(calls, ["dev", "demo"]);
  assert.ok(logs.includes("[stop] stopped 1 process(es)"));
  assert.equal(logs.some((message) => message.includes("Docker")), false);
});
