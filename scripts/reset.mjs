#!/usr/bin/env node
// DESTRUCTIVE: stop local infrastructure and delete its data volumes
// (PostgreSQL metadata, ClickHouse telemetry, Kafka log, demo database), then bootstrap
// a fresh stand (demo scope, its ingestion token and the owner account from .controlhub/owner.env).
// Requires typing RESET, or passing --yes for non-interactive use.
// `--no-bootstrap` only deletes.
import { createInterface } from "node:readline/promises";
import { join } from "node:path";
import { compose, root, run } from "./lib.mjs";

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
if (process.argv.includes("--no-bootstrap")) {
  console.error("[reset] containers and data volumes removed. Run `just bootstrap` to start fresh.");
} else {
  console.error("[reset] containers and data volumes removed; bootstrapping a fresh stand…");
  await run(process.execPath, [join(root, "scripts", "bootstrap.mjs")]);
}
