import assert from 'node:assert/strict';
import { test } from 'vite-plus/test';
import { ManagementPage, type PageIntent } from '../src/management-page.js';
import type { RemoteManagement, ManagementOutcome } from '../src/management.js';
import { ClientError, RefusalError } from '../src/operations.js';
function fixture() {
  let effectCalls = 0;
  let readCalls = 0;
  let reopens = 0;
  let failOpen: unknown;
  let result: ManagementOutcome | undefined;
  let mutationError: unknown;
  let readError: unknown;
  let writable = true;
  const remote: RemoteManagement = {
    async settings() {
      return {
        settings: {
          open: true,
          source: 'default',
          sessionsPerDevice: '18446744073709551615',
          sessionsPerDeviceSource: 'settings.json',
          warning: null,
        },
        capabilities: { settingsWrite: writable, devicesWrite: writable },
        readOnlyReason: writable ? null : 'local_cli_required',
      };
    },
    async devices() {
      return { devices: [], nextCursor: null };
    },
    async set(input) {
      effectCalls++;
      if (mutationError) throw mutationError;
      return (
        result ?? {
          operationId: input.operationId,
          state: 'unknown',
          reason: 'effect_outcome_unconfirmed',
        }
      );
    },
    async rename(input) {
      return this.set({ operationId: input.operationId, setting: 'open', value: true });
    },
    async revoke(input) {
      return this.set({ operationId: input.operationId, setting: 'open', value: true });
    },
    async operation() {
      readCalls++;
      if (readError) throw readError;
      return result!;
    },
  };
  const page = new ManagementPage(remote, async () => {
    reopens++;
    if (failOpen) throw failOpen;
    return remote;
  });
  return {
    page,
    remote,
    counts: () => ({ effectCalls, readCalls, reopens }),
    setResult: (value: ManagementOutcome) => {
      result = value;
    },
    failOpen: (error: unknown) => {
      failOpen = error;
    },
    failMutation: (error: unknown) => {
      mutationError = error;
    },
    failRead: (error: unknown) => {
      readError = error;
    },
    writable: (value: boolean) => {
      writable = value;
    },
  };
}
const input = (): PageIntent => ({
  kind: 'setting',
  input: {
    operationId: crypto.randomUUID(),
    setting: 'sessions-per-device',
    value: '18446744073709551615',
  },
});
test('page preserves admitted exact caps and read-only capability', async () => {
  const f = fixture();
  f.writable(false);
  await f.page.refresh();
  assert.equal(f.page.settings!.settings.sessionsPerDevice, '18446744073709551615');
  assert.equal(f.page.writable, false);
  await assert.rejects(f.page.submit(input()), /read-only/);
  assert.equal(f.counts().effectCalls, 0);
});
test('unknown freezes original intent, blocks fresh mutations and recovers by original read once', async () => {
  const f = fixture();
  await f.page.refresh();
  const intent = input();
  f.failMutation(new ClientError('outcome_unconfirmed', 'Unconfirmed.', intent.input.operationId));
  await f.page.submit(intent);
  const id = intent.input.operationId;
  intent.input.operationId = crypto.randomUUID();
  assert.equal(f.page.intent!.input.operationId, id);
  assert.equal(f.page.outcome!.state, 'unknown');
  await f.page.refresh();
  await assert.rejects(f.page.submit(input()), /read-only/);
  f.setResult({
    operationId: id,
    state: 'committed',
    result: { settings: f.page.settings!.settings },
    sessionEnded: false,
  });
  await f.page.recover();
  await f.page.recover();
  assert.equal(f.page.outcome!.state, 'committed');
  assert.deepEqual(f.counts(), { effectCalls: 1, readCalls: 1, reopens: 1 });
});
test('fresh admission refusal reports access loss without changing unknown to committed', async () => {
  const f = fixture();
  await f.page.refresh();
  await f.page.submit(input());
  f.failOpen(new RefusalError('REMOTE_SESSION_ENDED'));
  await f.page.recover();
  await f.page.recover();
  assert.equal(f.page.access, 'lost');
  assert.equal(f.page.outcome!.state, 'unknown');
  assert.equal(f.page.writable, false);
  assert.equal(f.page.canRecover, false);
  assert.deepEqual(f.counts(), { effectCalls: 1, readCalls: 0, reopens: 1 });
});
test('transport or stale/malformed descriptor remains unconfirmed access and unknown outcome', async () => {
  for (const error of [
    new Error('Transport failed.'),
    new Error('Descriptor changed.'),
    new Error('Invalid descriptor.'),
  ]) {
    const f = fixture();
    await f.page.refresh();
    await f.page.submit(input());
    f.failOpen(error);
    await f.page.recover();
    assert.equal(f.page.access, 'unconfirmed');
    assert.equal(f.page.outcome!.state, 'unknown');
    assert.equal(f.counts().effectCalls, 1);
  }
});
test('unavailable original receipt retains unknown, never authorizes replay', async () => {
  const f = fixture();
  await f.page.refresh();
  await f.page.submit(input());
  f.failRead(new RefusalError('REMOTE_MANAGEMENT_UNAVAILABLE'));
  await f.page.recover();
  assert.equal(f.page.outcome!.state, 'unknown');
  assert.equal(f.page.writable, false);
  assert.deepEqual(f.counts(), { effectCalls: 1, readCalls: 1, reopens: 1 });
});
for (const state of ['unknown', 'committed'] as const) {
  for (const method of ['settings', 'devices'] as const) {
    for (const access of ['lost', 'unconfirmed'] as const) {
      test(`failed ${method} refresh preserves ${state} original with ${access} access`, async () => {
        const f = fixture();
        await f.page.refresh();
        const intent = input();
        if (state === 'committed')
          f.setResult({
            operationId: intent.input.operationId,
            state,
            result: { settings: f.page.settings!.settings },
            sessionEnded: true,
          });
        await f.page.submit(intent);
        const original = f.page.intent;
        const outcome = f.page.outcome;
        let arrived!: () => void;
        let reject!: (error: unknown) => void;
        const reached = new Promise<void>((resolve) => {
          arrived = resolve;
        });
        const held = new Promise<never>((_resolve, refused) => {
          reject = refused;
        });
        f.remote[method] = () => {
          arrived();
          return held;
        };
        const refresh = f.page.refresh();
        await reached;
        assert.equal(f.page.busy, true);
        assert.equal(f.page.writable, false);
        assert.equal(f.page.intent, original);
        assert.equal(f.page.outcome, outcome);
        reject(
          access === 'lost'
            ? new RefusalError('REMOTE_SESSION_ENDED')
            : new ClientError('transport_failure', 'Unconfirmed refresh.'),
        );
        await refresh;
        assert.equal(f.page.busy, false);
        assert.equal(f.page.access, access);
        assert.equal(f.page.intent, original);
        assert.equal(f.page.outcome, outcome);
        assert.equal(f.page.writable, false);
        assert.equal(
          f.page.notice,
          state === 'unknown'
            ? 'Outcome unknown. Read the original operation; do not submit it again.'
            : 'Saved. Session limits apply at the next session open.',
        );
        assert.deepEqual(f.counts(), { effectCalls: 1, readCalls: 0, reopens: 0 });
      });
    }
  }
}

test('failed refresh without an original keeps current-access guidance', async () => {
  const f = fixture();
  f.remote.settings = async () => {
    throw new RefusalError('REMOTE_SESSION_ENDED');
  };
  await f.page.refresh();
  assert.equal(f.page.access, 'lost');
  assert.equal(f.page.outcome, undefined);
  assert.equal(f.page.notice, 'Current access could not be confirmed. Use the local CLI.');
  assert.deepEqual(f.counts(), { effectCalls: 0, readCalls: 0, reopens: 0 });
});

test('capacity refusal names local CLI and acknowledged self-change remains committed after access loss', async () => {
  const f = fixture();
  await f.page.refresh();
  const intent = input();
  f.setResult({
    operationId: intent.input.operationId,
    state: 'refused',
    reason: 'REMOTE_MANAGEMENT_CAPACITY',
  });
  await f.page.submit(intent);
  assert.match(f.page.notice, /operation limit.*tmt remote settings/);
  assert.equal(f.page.writable, false);
  const acknowledged = fixture();
  await acknowledged.page.refresh();
  const next = input();
  acknowledged.setResult({
    operationId: next.input.operationId,
    state: 'committed',
    result: { settings: f.page.settings!.settings },
    sessionEnded: true,
  });
  await acknowledged.page.submit(next);
  acknowledged.failOpen(new RefusalError('REMOTE_SESSION_ENDED'));
  await acknowledged.page.recover();
  assert.equal(acknowledged.page.access, 'lost');
  assert.equal(acknowledged.page.outcome!.state, 'committed');
});

test('ordinary, effect and original-receipt refresh retain the last successfully loaded cursor', async () => {
  const f = fixture();
  const cursors: (string | null)[] = [];
  f.remote.devices = async ({ cursor }) => {
    cursors.push(cursor);
    return { devices: [], nextCursor: 'next' };
  };
  await f.page.refresh();
  await f.page.refresh('later-page');
  await f.page.refresh();
  const intent = input();
  f.setResult({
    operationId: intent.input.operationId,
    state: 'committed',
    result: { settings: f.page.settings!.settings },
    sessionEnded: false,
  });
  await f.page.submit(intent);
  await f.page.refresh();
  await f.page.recover();
  await f.page.refresh();
  assert.deepEqual(cursors, [null, 'later-page', 'later-page', 'later-page', 'later-page']);
  assert.equal(f.page.onFirstPage, false);
  await f.page.refresh(null);
  assert.equal(f.page.onFirstPage, true);
});
