#!/usr/bin/env node
// Stop local infrastructure. Keeps all data volumes (use `just reset` to wipe).
import { compose } from "./lib.mjs";

await compose(["down"], ["--profile", "demo"]);
console.error("[down] containers stopped; data volumes kept.");
