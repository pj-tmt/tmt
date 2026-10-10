import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
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
  let firestoreError: unknown;
  let firestoreGate: Promise<void> | undefined;
  const remote: RemoteManagement = {
    async settings(options?: { firestore: true }) {
      if (options && firestoreGate) await firestoreGate;
      if (options && firestoreError) throw firestoreError;
      return {
        firestoreLayers: [],
        firestoreBudget: JSON.parse(
          readFileSync(
            new URL(
              '../../../rust/tmt-remote/tests/fixtures/firestore_budget/limits-member.json',
              import.meta.url,
            ),
            'utf8',
          ),
        ),
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
    async talk(input) {
      return this.set({ operationId: input.operationId, setting: 'open', value: input.enabled });
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
    holdFirestore: (gate: Promise<void>) => {
      firestoreGate = gate;
    },
    failFirestore: (error: unknown) => {
      firestoreError = error;
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
            ? 'This change could not be confirmed. Check its original result before making another change.'
            : 'Session limit saved. Applies to new sessions. This session ended. Use the local CLI to continue.',
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
  assert.match(f.page.notice, /Browser change limit reached/);
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

test('unknown sending toggle keeps its frozen boolean and recovers only the original', async () => {
  const f = fixture();
  await f.page.refresh();
  const intent: PageIntent = {
    kind: 'talk',
    input: { operationId: crypto.randomUUID(), clientId: crypto.randomUUID(), enabled: true },
  };
  const id = intent.input.operationId;
  await f.page.submit(intent);
  intent.input.enabled = false;
  assert.equal(f.page.intent?.kind, 'talk');
  assert.ok(f.page.intent?.kind === 'talk');
  assert.equal(f.page.intent.input.enabled, true);
  f.setResult({ operationId: id, state: 'refused', reason: 'REMOTE_DEVICE_REVOKED' });
  await f.page.recover();
  await f.page.recover();
  assert.deepEqual(f.counts(), { effectCalls: 1, readCalls: 1, reopens: 1 });
});

test('optional Firestore failure clears evidence without losing access, original outcome or drafts', async () => {
  const f = fixture();
  await f.page.refresh();
  await f.page.observeFirestore();
  assert.equal(f.page.firestoreAccess, 'confirmed');
  assert.deepEqual(f.page.firestore!.firestoreLayers, []);
  await f.page.submit(input());
  const original = f.page.intent;
  const outcome = f.page.outcome;
  const notice = f.page.notice;
  f.failFirestore(new RefusalError('REMOTE_INPUT_INVALID'));
  await f.page.refresh();
  await f.page.observeFirestore();
  assert.equal(f.page.access, 'live');
  assert.equal(f.page.firestoreAccess, 'unconfirmed');
  assert.equal(f.page.firestore, undefined);
  assert.equal(f.page.intent, original);
  assert.equal(f.page.outcome, outcome);
  assert.equal(f.page.notice, notice);
  assert.equal(f.counts().reopens, 0);
});

test('deferred Firestore observation never keeps management busy and refresh invalidates stale evidence', async () => {
  const f = fixture();
  let release!: () => void;
  f.holdFirestore(
    new Promise<void>((resolve) => {
      release = resolve;
    }),
  );
  await f.page.refresh();
  const optional = f.page.observeFirestore();
  assert.equal(f.page.busy, false);
  assert.equal(f.page.writable, true);
  assert.equal(f.page.access, 'live');
  assert.ok(f.page.settings && f.page.devices);
  assert.equal(f.page.firestoreAccess, 'checking');
  assert.equal(f.page.firestore, undefined);
  await f.page.refresh();
  release();
  await optional;
  assert.equal(f.page.firestoreAccess, 'checking');
  assert.equal(f.page.firestore, undefined, 'old read cannot overwrite a new refresh');
  await f.page.observeFirestore();
  assert.equal(f.page.firestoreAccess, 'confirmed');
});

for (const [intent, expected] of [
  [
    { kind: 'setting', input: { operationId: 'original', setting: 'open', value: false } },
    'Browser opening saved.',
  ],
  [
    {
      kind: 'setting',
      input: { operationId: 'original', setting: 'sessions-per-device', value: '9' },
    },
    'Session limit saved. Applies to new sessions.',
  ],
  [
    { kind: 'rename', input: { operationId: 'original', clientId: 'device', name: 'Renamed' } },
    'Device renamed.',
  ],
  [
    { kind: 'talk', input: { operationId: 'original', clientId: 'device', enabled: true } },
    'Sending enabled for this device.',
  ],
  [
    { kind: 'talk', input: { operationId: 'original', clientId: 'device', enabled: false } },
    'Sending disabled for this device.',
  ],
  [{ kind: 'revoke', input: { operationId: 'original', clientId: 'device' } }, 'Device revoked.'],
] satisfies [PageIntent, string][]) {
  test(`acknowledged outcome copy: ${expected}`, async () => {
    for (const sessionEnded of [false, true]) {
      const f = fixture();
      await f.page.refresh();
      f.setResult({
        operationId: intent.input.operationId,
        state: 'committed',
        result: { settings: f.page.settings!.settings },
        sessionEnded,
      });
      await f.page.submit(intent);
      assert.equal(
        f.page.notice,
        expected + (sessionEnded ? ' This session ended. Use the local CLI to continue.' : ''),
      );
      assert.equal(f.page.access, sessionEnded ? 'unconfirmed' : 'live');
    }
  });
}

for (const [reason, expected] of [
  ['REMOTE_MANAGEMENT_READ_ONLY', 'This browser is read-only. Use the local CLI to make changes.'],
  [
    'REMOTE_MANAGEMENT_CAPACITY',
    'Browser change limit reached. Use the local CLI; do not retry or reset storage.',
  ],
  ['REMOTE_DEVICE_REVOKED', 'Change refused. Check the current values or use the local CLI.'],
] as const) {
  test(`known refusal copy stays distinct: ${reason}`, async () => {
    const f = fixture();
    await f.page.refresh();
    const intent = input();
    f.setResult({ operationId: intent.input.operationId, state: 'refused', reason });
    await f.page.submit(intent);
    assert.equal(f.page.notice, expected);
  });
}
for (const [kind, enabled, expected] of [
  ['rename', false, 'Saving…'],
  ['talk', true, 'Enabling sending…'],
  ['talk', false, 'Disabling sending…'],
  ['revoke', false, 'Revoking device…'],
] as const) {
  test(`pending ${kind}/${enabled} copy comes from the frozen original`, async () => {
    const f = fixture();
    await f.page.refresh();
    let release!: () => void;
    const held = new Promise<void>((resolve) => {
      release = resolve;
    });
    const set = f.remote.set.bind(f.remote);
    f.remote.set = async (change) => {
      await held;
      return set(change);
    };
    const intent: PageIntent =
      kind === 'rename'
        ? { kind, input: { operationId: 'original', clientId: 'device', name: 'New name' } }
        : kind === 'talk'
          ? { kind, input: { operationId: 'original', clientId: 'device', enabled } }
          : { kind, input: { operationId: 'original', clientId: 'device' } };
    const submission = f.page.submit(intent);
    assert.equal(f.page.busy, true);
    assert.equal(f.page.notice, expected);
    release();
    await submission;
    assert.equal(
      f.page.notice,
      'This change could not be confirmed. Check its original result before making another change.',
    );
  });
}
test('refused recovery keeps the unknown original and its write lock', async () => {
  for (const refusal of ['open', 'read'] as const) {
    const f = fixture();
    await f.page.refresh();
    await f.page.submit(input());
    const original = f.page.intent;
    const outcome = f.page.outcome;
    if (refusal === 'open') f.failOpen(new RefusalError('REMOTE_SESSION_ENDED'));
    else f.failRead(new RefusalError('REMOTE_MANAGEMENT_UNAVAILABLE'));
    await f.page.recover();
    await f.page.recover();
    assert.equal(f.page.intent, original);
    assert.equal(f.page.outcome, outcome);
    assert.equal(f.page.outcome!.state, 'unknown');
    assert.equal(f.page.writable, false);
    assert.equal(f.page.canRecover, false);
    assert.equal(
      f.page.notice,
      'The original result is still unconfirmed. Check with tmt remote settings or tmt remote devices.',
    );
    assert.deepEqual(f.counts(), {
      effectCalls: 1,
      readCalls: refusal === 'read' ? 1 : 0,
      reopens: 1,
    });
  }
});
