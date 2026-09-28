#!/usr/bin/env node
// DESTRUCTIVE: stop local infrastructure and delete its data volumes
// (PostgreSQL metadata, ClickHouse telemetry, Kafka log, demo database).
// Requires typing RESET, or passing --yes for non-interactive use.
import { createInterface } from "node:readline/promises";
import { compose } from "./lib.mjs";

if (!process.argv.includes("--yes")) {
  const rl = createInterface({ input: process.stdin, output: process.stderr });
  const answer = await rl.question("This deletes ALL local ControlHub data volumes. Type RESET to continue: ");
  rl.close();
  if (answer.trim() !== "RESET") {
    console.error("[reset] aborted");
    process.exit(1);
  }
}
await compose(["down", "--volumes"], ["--profile", "demo"]);
console.error("[reset] containers and data volumes removed. Run `just bootstrap` to start fresh.");
