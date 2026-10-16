import { afterEach, expect, it, vi } from 'vite-plus/test';
import * as Y from 'yjs';
import type { Connection } from '../src/connection.js';
import type { FoldCommand, FoldResult } from '../src/fold-protocol.js';
import { Writer, memoryRecords } from '../src/writer.js';

// Two tabs of one device write one own stream through its leader. A tab whose view still lags the
// other's last write can prepare a record for a key the stream already holds: with two Yjs
// clients and no check, the key silently takes whichever value has the higher client ID, and the
// tab that lost times out and closes its connection (#3083). The leader orders every append of
// the stream, so it refuses such a write before sealing anything.
vi.mock('../src/own-seal.js', () => ({
  sealOwnUpdate: async (_a: unknown, head: { seq: bigint }, update: Uint8Array) => ({
    seq: String(head.seq + 1n),
    envelopeHash: 'hash',
    envelope: 'envelope',
    update: update.slice(),
  }),
}));
const device = '00000000-0000-4000-8000-000000000004';
const operation = '00000000-0000-4000-8000-0000000000a1';
const key = `${operation}:2`;
const ledger = (revision: string, state: string) => ({
  version: 1,
  kind: 'ask-state',
  operationId: operation,
  revision,
  state,
  requestId: null,
  reason: null,
});

class Channel {
  static open = new Set<Channel>();
  onmessage: ((event: { data: unknown }) => void) | null = null;
  constructor(readonly name: string) {
    Channel.open.add(this);
  }
  postMessage(data: unknown) {
    for (const other of Channel.open)
      if (other !== this && other.name === this.name)
        queueMicrotask(() => other.onmessage?.({ data: structuredClone(data) }));
  }
  close() {
    Channel.open.delete(this);
  }
}
afterEach(() => {
  vi.unstubAllGlobals();
  Channel.open.clear();
});

/** The real fold worker, as the leader's connection runs it. */
async function realFold() {
  vi.resetModules();
  let result: (FoldResult & { error?: string }) | undefined;
  const host = {
    onmessage: null as unknown as (event: unknown) => Promise<void>,
    postMessage(value: FoldResult & { error?: string }) {
      result = structuredClone(value);
    },
  };
  vi.stubGlobal('self', host);
  await import('../src/fold.worker.js');
  let id = 0;
  return async (command: FoldCommand) => {
    await host.onmessage({ data: { id: ++id, command } });
    if (result?.error) throw new Error(result.error);
    return result as FoldResult;
  };
}

async function tabs() {
  const run = await realFold();
  let head = 0n;
  const admitted: Uint8Array[] = [];
  const leaderConnection = {
    ready: Promise.resolve(),
    active: true,
    admission: { registration: { deviceId: device }, ownAuthor: async () => {} },
    objects: { head: () => ({ seq: head, hash: new Uint8Array(32) }) },
    run: (fn: () => Promise<unknown>) => fn(),
    fold: { run },
    // As Connection does at a receipt: the appended update is admitted into the fold.
    append: vi.fn(async (entry: { seq: string; update: Uint8Array }) => {
      admitted.push(entry.update);
      head = BigInt(entry.seq);
      await run({ type: 'apply', updates: [], own: [{ writer: device, update: entry.update }] });
    }),
    waitForOwnRecords: vi.fn(async () => {}),
    close: vi.fn(),
  };
  vi.stubGlobal('BroadcastChannel', Channel);
  // The first writer takes the lifetime lock and leads; the second never does.
  vi.stubGlobal('navigator', {
    locks: { request: (_key: string, _options: unknown, fn: () => unknown) => fn() },
  });
  const leader = new Writer(
    'own-stream',
    async () => leaderConnection as unknown as Connection,
    memoryRecords(),
  );
  await Promise.resolve();
  vi.stubGlobal('navigator', { locks: { request: () => new Promise(() => {}) } });
  // The peer's own fold has admitted nothing of the sender's last write: its view lags.
  const lagging = new Y.Doc();
  lagging.clientID = 7;
  for (const name of ['threads', 'messages', 'intents', 'replies']) lagging.getMap(name);
  const peerConnection = {
    ready: Promise.resolve(),
    run: (fn: () => Promise<unknown>) => fn(),
    admission: { registration: { deviceId: device } },
    fold: {
      run: vi.fn(async (command: FoldCommand) => {
        if (command.type !== 'prepare-own') throw new Error('unexpected fold command');
        // A draft on a copy: prepared bytes are never committed before the leader admits them.
        const draft = new Y.Doc();
        draft.clientID = lagging.clientID;
        Y.applyUpdate(draft, Y.encodeStateAsUpdate(lagging));
        const before = Y.encodeStateVector(draft);
        for (const record of command.records)
          draft.getMap(record.root).set(record.key, record.value);
        return { update: Y.encodeStateAsUpdate(draft, before) } as FoldResult;
      }),
    },
    waitForOwnRecords: vi.fn(async () => {}),
    close: vi.fn(),
  };
  const peer = new Writer(
    'own-stream',
    async () => peerConnection as unknown as Connection,
    memoryRecords(),
  );
  const accepted = async () => (await run({ type: 'apply', updates: [] })).own[device];
  return { leader, peer, leaderConnection, peerConnection, admitted, run, accepted };
}

it('refuses a lagging peer tab write to a key the stream already holds, and the sender keeps its value', async () => {
  const t = await tabs();
  try {
    // The sender wrote revision 2 `accepted`; the leader appended and admitted it.
    await t.leader.submitOwnRecords([{ root: 'messages', key, value: ledger('2', 'accepted') }]);
    expect(t.leaderConnection.append).toHaveBeenCalledTimes(1);
    // The peer's view still ends at revision 1, so it prepares revision 2 `uncertain`.
    const refused = t.peer
      .submitOwnRecords([{ root: 'messages', key, value: ledger('2', 'uncertain') }])
      .then(
        () => 'accepted',
        (error: Error) => error.constructor.name,
      );
    await expect(refused).resolves.toBe('OwnRecordConflict');
    // Nothing was appended, no publication wait ran, and neither connection was closed.
    expect(t.leaderConnection.append).toHaveBeenCalledTimes(1);
    expect(t.peerConnection.waitForOwnRecords).not.toHaveBeenCalled();
    expect(t.leaderConnection.close).not.toHaveBeenCalled();
    expect(t.peerConnection.close).not.toHaveBeenCalled();
    // The admitted value did not change, whatever the clients' IDs.
    expect((await t.accepted()).messages[key]).toEqual(ledger('2', 'accepted'));
    // The stream is not frozen: the next write, to a new key, is appended.
    await t.peer.submitOwnRecords([
      { root: 'messages', key: `${operation}:3`, value: ledger('3', 'accepted') },
    ]);
    expect(t.leaderConnection.append).toHaveBeenCalledTimes(2);
    expect((await t.accepted()).messages[`${operation}:3`]).toEqual(ledger('3', 'accepted'));
  } finally {
    t.leader.close();
    t.peer.close();
  }
});

it('accepts a byte-identical write to a key the stream already holds as already present', async () => {
  const t = await tabs();
  try {
    await t.leader.submitOwnRecords([{ root: 'messages', key, value: ledger('2', 'accepted') }]);
    await t.peer.submitOwnRecords([{ root: 'messages', key, value: ledger('2', 'accepted') }]);
    expect(t.peerConnection.waitForOwnRecords).toHaveBeenCalledOnce();
    expect((await t.accepted()).messages[key]).toEqual(ledger('2', 'accepted'));
  } finally {
    t.leader.close();
    t.peer.close();
  }
});
