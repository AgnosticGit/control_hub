import { test } from 'node:test';
import assert from 'node:assert/strict';
import { containerIdFromCgroup, containerIdFromMountinfo, infraAttributes } from '../src/resource.js';

const ID = 'a'.repeat(12) + '0123456789abcdef'.repeat(3) + 'b'.repeat(4);

test('reads the container ID from cgroup v1 and systemd paths', () => {
  assert.equal(containerIdFromCgroup(`12:memory:/docker/${ID}\n11:cpu:/docker/${ID}`), ID);
  assert.equal(containerIdFromCgroup(`0::/system.slice/docker-${ID}.scope`), ID);
  assert.equal(containerIdFromCgroup(`0::/kubepods/burstable/pod1/cri-containerd-${ID}.scope`), ID);
});

test('finds no container on a plain host or in cgroup v2 namespaces', () => {
  assert.equal(containerIdFromCgroup('0::/'), null);
  assert.equal(containerIdFromCgroup('0::/user.slice/user-1000.slice/session-2.scope'), null);
  assert.equal(containerIdFromCgroup(''), null);
  assert.equal(containerIdFromCgroup(`0::/docker/${ID.slice(1)}`), null, 'too short');
});

test('reads the container ID from mountinfo under cgroup v2', () => {
  const line = `612 590 8:1 /var/lib/docker/containers/${ID}/hostname /etc/hostname rw,relatime - ext4 /dev/sda1 rw`;
  assert.equal(containerIdFromMountinfo(`1 0 8:1 / / rw - ext4 /dev/sda1 rw\n${line}`), ID);
  assert.equal(containerIdFromMountinfo('1 0 8:1 / / rw - ext4 /dev/sda1 rw'), null);
});

test('options win over env, env over detection', () => {
  const deps = {
    env: { KARBIRD_HOST_NAME: 'vps-app', KARBIRD_CONTAINER_ID: 'from-env' },
    hostname: () => 'os-host',
    pid: 42,
    read: () => `0::/docker/${ID}`,
  };
  assert.deepEqual(infraAttributes({}, deps), { 'host.name': 'vps-app', 'container.id': 'from-env', 'process.pid': 42 });
  assert.deepEqual(infraAttributes({ hostName: 'h', containerId: 'c' }, deps), { 'host.name': 'h', 'container.id': 'c', 'process.pid': 42 });
  assert.deepEqual(infraAttributes({}, { ...deps, env: {} }), { 'host.name': 'os-host', 'container.id': ID, 'process.pid': 42 });
});

test('a host without /proc gives no container.id and never throws', () => {
  const attrs = infraAttributes({}, {
    env: {},
    hostname: () => 'laptop',
    pid: 7,
    read: () => {
      throw new Error('ENOENT');
    },
  });
  assert.deepEqual(attrs, { 'host.name': 'laptop', 'process.pid': 7 });
});

test('only host, container and pid attributes are added (no command line or env)', () => {
  const attrs = infraAttributes({}, { env: { SECRET: 'x', KARBIRD_INGEST_TOKEN: 't' }, hostname: () => 'h', pid: 1, read: () => '' });
  assert.deepEqual(Object.keys(attrs).sort(), ['host.name', 'process.pid']);
});
