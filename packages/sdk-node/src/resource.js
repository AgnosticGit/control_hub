// Where this process runs: host.name, container.id and process.pid on the resource, so
// karbird can tie the service's spans to the machine and container metrics the Karbird
// agent reports. Nothing else is read from the host (no command line, no environment dump).

import { readFileSync } from 'node:fs';
import { hostname } from 'node:os';

const ID = /[0-9a-f]{64}/;
// cgroup v1 / systemd paths: .../docker/<id>, docker-<id>.scope, cri-containerd-<id>.scope, crio-<id>, libpod-<id>.
const CGROUP_ID = /[/-]([0-9a-f]{64})(?:\.scope)?$/;
// cgroup v2 hides the path; Docker's bind mounts still name the container directory.
const MOUNT_ID = /\/(?:docker\/containers|containers|sandboxes)\/([0-9a-f]{64})\//;

/** Container ID from `/proc/self/cgroup` (cgroup v1 or systemd driver), or null. */
export function containerIdFromCgroup(text) {
  for (const line of String(text).split('\n')) {
    const path = line.slice(line.lastIndexOf(':') + 1).trim();
    const m = CGROUP_ID.exec(path);
    if (m) return m[1];
  }
  return null;
}

/** Container ID from `/proc/self/mountinfo` (cgroup v2), or null. */
export function containerIdFromMountinfo(text) {
  for (const line of String(text).split('\n')) {
    const m = MOUNT_ID.exec(line);
    if (m && ID.test(m[1])) return m[1];
  }
  return null;
}

function readOrEmpty(read, path) {
  try {
    return read(path);
  } catch {
    return '';
  }
}

/**
 * Resource attributes describing the host and container.
 * @param {{ hostName?: string, containerId?: string }} options
 * @param {{ env?: NodeJS.ProcessEnv, hostname?: () => string, pid?: number, read?: (path: string) => string }} deps
 */
export function infraAttributes(options = {}, deps = {}) {
  const env = deps.env ?? process.env;
  const read = deps.read ?? ((p) => readFileSync(p, 'utf8'));
  const attrs = {};
  const host = options.hostName || env.KARBIRD_HOST_NAME || (deps.hostname ?? hostname)();
  if (host) attrs['host.name'] = host;
  const container =
    options.containerId ||
    env.KARBIRD_CONTAINER_ID ||
    containerIdFromCgroup(readOrEmpty(read, '/proc/self/cgroup')) ||
    containerIdFromMountinfo(readOrEmpty(read, '/proc/self/mountinfo'));
  if (container) attrs['container.id'] = container;
  attrs['process.pid'] = deps.pid ?? process.pid;
  return attrs;
}
