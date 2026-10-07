import { test } from 'node:test';
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { ORIGIN, Session } from './lib.mjs';

const PASSWORD = `organization test ${randomUUID()}`;

async function account(label) {
  const session = new Session();
  const email = `${label}-${randomUUID()}@example.test`;
  const registered = await session.json('/api/v1/session/register', {
    method: 'POST',
    headers: { 'content-type': 'application/json', origin: ORIGIN },
    body: JSON.stringify({ email, password: PASSWORD }),
  });
  assert.equal(registered.status, 201, JSON.stringify(registered.body));
  return { session, email, body: registered.body };
}

test('organization membership does not expose a project until it is explicitly assigned', async () => {
  const owner = await account('organization-owner');
  const member = await account('organization-member');
  const first = await owner.session.mutate('/api/v1/onboarding/first-project', {
    organization_name: `Organization ${randomUUID().slice(0, 6)}`,
    project_name: `Project ${randomUUID().slice(0, 6)}`,
    environments: ['Production'],
  });
  assert.equal(first.status, 201, JSON.stringify(first.body));
  const ownerSession = (await owner.session.json('/api/v1/session')).body;
  const organization = ownerSession.organizations[0];
  const project = ownerSession.projects[0];
  const role = (await owner.session.json(`/api/v1/projects/${project.id}/roles`)).body.find((candidate) => candidate.builtin === 'viewer');

  const added = await owner.session.mutate(`/api/v1/organizations/${organization.id}/members`, {
    email: member.email,
    organization_role: 'member',
    assignments: [],
  });
  assert.equal(added.status, 201, JSON.stringify(added.body));
  assert.deepEqual(added.body.assignments, []);
  assert.deepEqual((await member.session.json('/api/v1/session')).body.projects, []);
  assert.equal((await member.session.json(`/api/v1/projects/${project.id}/environments/${project.environments[0].id}/services`)).status, 404);

  const assigned = await owner.session.mutate(
    `/api/v1/organizations/${organization.id}/projects/${project.id}/members/${added.body.user_id}`,
    { role_id: role.id },
    'PUT',
  );
  assert.equal(assigned.status, 204, JSON.stringify(assigned.body));
  assert.ok((await member.session.json('/api/v1/session')).body.projects.some((candidate) => candidate.id === project.id));

  const removed = await owner.session.mutate(
    `/api/v1/organizations/${organization.id}/projects/${project.id}/members/${added.body.user_id}`,
    null,
    'DELETE',
  );
  assert.equal(removed.status, 204, JSON.stringify(removed.body));
  assert.deepEqual((await member.session.json('/api/v1/session')).body.projects, []);
});

