#!/usr/bin/env node
// Stop processes started by `just dev` / `just demo`. Docker state is unchanged.
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { stopGroup } from "./lib.mjs";

const isMain = process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url;

export async function stopAll({ stopProcessGroup = stopGroup, log = console.error } = {}) {
  let total = 0;
  for (const group of ["dev", "demo"]) {
    const stopped = stopProcessGroup(group);
    total += stopped.length;
    for (const entry of stopped) log(`[stop] ${group}: ${entry.name} (pid ${entry.pid})`);
  }
  log(total ? `[stop] stopped ${total} process(es)` : "[stop] no application processes were running");

  return 0;
}

if (isMain) process.exitCode = await stopAll();
