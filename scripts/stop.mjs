#!/usr/bin/env node
// Stop processes started by `just dev` / `just demo` that are still running, e.g. after the
// terminal running them was closed. Only PIDs recorded by those scripts are touched.
// Infrastructure containers are left alone (`just down` stops them).
import { stopGroup } from "./lib.mjs";

let total = 0;
for (const group of ["dev", "demo"]) {
  const stopped = stopGroup(group);
  total += stopped.length;
  for (const e of stopped) console.error(`[stop] ${group}: ${e.name} (pid ${e.pid})`);
}
console.error(total ? `[stop] stopped ${total} process(es)` : "[stop] nothing running");
