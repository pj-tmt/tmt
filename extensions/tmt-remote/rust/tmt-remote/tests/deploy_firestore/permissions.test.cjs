'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');
const { run } = require('../../src/deploy_firestore/helper.cjs');
const root = path.resolve(__dirname, '../fixtures/deploy_firestore/firebase-tools');
const project = 'demo-permissions';
const account = 'owner@example.test';
const permissions = ['firebase.projects.get', 'datastore.databases.create'];
const url = `https://cloudresourcemanager.googleapis.com/v3/projects/${project}:testIamPermissions`;
function request(names = permissions) {
  return {
    version: 1,
    operation: 'permission-probe',
    input: { project, permissions: names },
    account,
    budgetMs: 30000,
  };
}
async function probe(response, names = permissions, overrides = {}) {
  const calls = [];
  let identities = 0;
  const deps = {
    deadline: Date.now() + 30000,
    credential: async () => {
      identities++;
      return { account, token: 'TOKEN_CANARY' };
    },
    http: async (target, options, deadline) => {
      calls.push({
        target,
        method: options.method,
        body: options.body && JSON.parse(options.body),
        deadline,
      });
      if (target.endsWith('/oauth2/v3/userinfo'))
        return { status: 200, body: { email: account, email_verified: true } };
      assert.equal(target, url);
      assert.equal(options.method, 'POST');
      assert.deepEqual(JSON.parse(options.body), { permissions: names });
      return typeof response === 'function' ? response() : response;
    },
    ...overrides,
  };
  const result = await run(JSON.stringify(request(names)), root, deps);
  assert.ok(!JSON.stringify(result).includes('TOKEN_CANARY'));
  return { result, calls, identities, deadline: deps.deadline };
}
test('permission probe posts the exact set once without mutation identity refresh or deadline renewal', async () => {
  const answer = await probe({ status: 200, body: { permissions } });
  assert.deepEqual(answer.result, { version: 1, result: permissions });
  assert.equal(answer.identities, 1);
  assert.deepEqual(
    answer.calls.map((c) => c.method),
    ['GET', 'POST']
  );
  assert.ok(answer.calls.every((c) => c.deadline === answer.deadline));
});
test('omitted and empty grant sets remain readable subsets, never invented grants', async () => {
  for (const body of [{ permissions: permissions.slice(0, 1) }, { permissions: [] }, {}]) {
    assert.deepEqual((await probe({ status: 200, body })).result, {
      version: 1,
      result: body.permissions ?? [],
    });
  }
});
test('malformed, duplicate or unsolicited granted permissions are unreadable', async () => {
  for (const body of [
    null,
    [],
    { permissions: null },
    { permissions: 'TOKEN_CANARY' },
    { permissions: [permissions[0], permissions[0]] },
    { permissions: ['other.resource.read'] },
    { permissions, token: 'TOKEN_CANARY' },
  ]) {
    assert.equal((await probe({ status: 200, body })).result.error, 'unknown');
  }
});
test('API disabled, permission denied and network failures retain closed read diagnostics without retry', async () => {
  for (const [response, code] of [
    [
      {
        status: 403,
        body: { error: { details: [{ reason: 'SERVICE_DISABLED' }], message: 'TOKEN_CANARY' } },
      },
      'api-disabled',
    ],
    [{ status: 403, body: { error: { message: 'TOKEN_CANARY' } } }, 'permission-denied'],
    [
      () => {
        throw Error('TOKEN_CANARY');
      },
      'unknown',
    ],
  ]) {
    const answer = await probe(response);
    assert.equal(answer.result.error, code);
    assert.equal(answer.calls.length, 2);
    if (typeof response !== 'function')
      assert.deepEqual(answer.result.provider, { operation: 'permission-probe', status: 403 });
  }
});
test('invalid permission inputs refuse before identity or HTTP', async () => {
  for (const names of [
    [],
    ['*'],
    ['storage.*'],
    ['https://secret.invalid'],
    [permissions[0], permissions[0]],
    ['a'.repeat(129)],
    Array(129).fill('a.b.c'),
    null,
  ]) {
    const answer = await probe({ status: 200, body: {} }, names);
    assert.equal(answer.result.error, 'provider-rejected');
    assert.equal(answer.calls.length, 0);
    assert.equal(answer.identities, 0);
  }
});
test('an exhausted read deadline never becomes checked or starts the permission POST', async () => {
  const answer = await probe({ status: 200, body: { permissions } }, permissions, { deadline: 1 });
  assert.equal(answer.result.error, 'unknown');
  assert.equal(answer.calls.length, 0);
});
