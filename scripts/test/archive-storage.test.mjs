import { test } from 'node:test';
import assert from 'node:assert/strict';
import { ARCHIVE_BUCKET, s3Identities } from '../archive-storage.mjs';

const infra = {
  CH_S3_ADMIN_ACCESS_KEY: 'adm', CH_S3_ADMIN_SECRET_KEY: 'adm-s',
  CH_S3_ARCHIVER_ACCESS_KEY: 'arc', CH_S3_ARCHIVER_SECRET_KEY: 'arc-s',
  CH_S3_API_ACCESS_KEY: 'api', CH_S3_API_SECRET_KEY: 'api-s',
};

const byName = (cfg) => Object.fromEntries(cfg.identities.map((i) => [i.name, i]));

test('archiver and API are limited to the archive bucket and never admins', () => {
  const ids = byName(s3Identities(infra));
  for (const name of ['archiver', 'api']) {
    assert.ok(ids[name].actions.length > 0);
    assert.ok(ids[name].actions.every((a) => a.endsWith(`:${ARCHIVE_BUCKET}`)), `${name}: every grant is scoped to the bucket`);
    assert.ok(!ids[name].actions.some((a) => a.startsWith('Admin')));
  }
  assert.ok(ids.api.actions.includes(`Read:${ARCHIVE_BUCKET}`));
  assert.ok(ids.archiver.actions.includes(`Write:${ARCHIVE_BUCKET}`));
});

test('every identity has its own credentials', () => {
  const cfg = s3Identities(infra);
  const keys = cfg.identities.map((i) => i.credentials[0].accessKey);
  assert.equal(new Set(keys).size, 3);
  assert.deepEqual(byName(cfg).api.credentials, [{ accessKey: 'api', secretKey: 'api-s' }]);
});

test('identities reference a declared account (SeaweedFS ignores undeclared ones)', () => {
  const cfg = s3Identities(infra);
  for (const i of cfg.identities) assert.ok(cfg.accounts.some((a) => a.id === i.account.id), i.name);
});

test('missing credentials fail instead of writing an open config', () => {
  assert.throws(() => s3Identities({ ...infra, CH_S3_API_SECRET_KEY: '' }), /CH_S3_API/);
  assert.throws(() => s3Identities({}), /CH_S3_ADMIN/);
});
