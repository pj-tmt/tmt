import assert from 'node:assert/strict';
import { test } from 'vite-plus/test';
import { invitationExpiry } from '../src/invite-page.js';
import vector from '../../../rust/tmt-remote/tests/fixtures/invitations/invite-v2.json' with { type: 'json' };

test('instructions use the exact native v2 code, pin and original deadline', () => {
  assert.equal(invitationExpiry(vector.token, vector.now), vector.invite.context.expires_at_ms);
  const link = new URL(`https://dedicated-site.web.app/join#${vector.token}`);
  assert.equal(link.hash.slice(1), vector.token);
  assert.deepEqual(
    JSON.parse(Buffer.from(link.hash.slice(1), 'base64url').toString()),
    vector.invite,
  );
  assert.throws(() => invitationExpiry(vector.token, vector.invite.context.expires_at_ms));
});
test('instructions refuse malformed, grafted, ambiguous or expired carriers without echoing them', () => {
  const encode = (value: unknown) => Buffer.from(JSON.stringify(value)).toString('base64url');
  const controls: unknown[] = [
    { ...vector.invite, version: 1 },
    { ...vector.invite, unexpected: 'SECRET_CANARY' },
    { ...vector.invite, code: [4] },
    { ...vector.invite, context: { ...vector.invite.context, owner: 'wrong' } },
    {
      ...vector.invite,
      context: { ...vector.invite.context, expires_at_ms: vector.now + 600_001 },
    },
    {
      ...vector.invite,
      context: {
        ...vector.invite.context,
        binding: { ...vector.invite.context.binding, backend: 'other' },
      },
    },
    {
      ...vector.invite,
      publicConfig: { ...vector.invite.publicConfig, projectId: 'wrong-project' },
    },
  ];
  for (const value of controls) {
    assert.throws(
      () => invitationExpiry(encode(value), vector.now),
      (error: Error) => !error.message.includes('SECRET_CANARY'),
    );
  }
  for (const token of [
    '',
    vector.token + '=',
    'x'.repeat(16_385),
    Buffer.from('{"version":1,"version":2}').toString('base64url'),
  ]) {
    assert.throws(() => invitationExpiry(token, vector.now));
  }
});
