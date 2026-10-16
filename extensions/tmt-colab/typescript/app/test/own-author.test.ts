import { readFileSync } from 'node:fs';
import { afterEach, expect, it, vi } from 'vite-plus/test';
import * as c from '@tmt/colab-client';
import { Admission } from '../src/admission.js';
import { Connection } from '../src/connection.js';
import { OwnAppendRefused } from '../src/own-publication.js';
import { Writer, memoryRecords } from '../src/writer.js';

// Which author a write is checked against follows the chain its device holds: a link's device is
// admitted by `linkAuthor`, a member's by `author`, and neither stands in for the other. The
// admission here is a fake whose `ownAuthor` is the real one, so a write that skipped it would call
// the wrong checker, and the checker that is not its own refuses.
vi.mock('../src/fold.js', () => ({
  Fold: class {
    async run() {
      return { source: '', title: '' };
    }
    close() {}
  },
}));
vi.mock('../src/catchup.js', () => ({
  Catchup: class {
    updates = [];
    checkpoints = [];
    baseline = null;
    async admitValue() {
      return true;
    }
    close() {}
  },
}));
vi.mock('../src/objects.js', async (original) => ({
  ...(await original<typeof import('../src/objects.js')>()),
  Objects: class {
    head() {
      return { seq: 4n, hash: new Uint8Array(32) };
    }
    cursors() {
      return [];
    }
  },
}));
class Socket extends EventTarget {
  static OPEN = 1;
  static CLOSED = 3;
  static current: Socket;
  readyState = 1;
  bufferedAmount = 0;
  onopen = null;
  onmessage: ((event: { data: string }) => void) | null = null;
  onerror = null;
  onclose = null;
  send = vi.fn();
  close = vi.fn();
  constructor(readonly url: string | URL) {
    super();
    Socket.current = this;
  }
}
class Channel {
  onmessage = null;
  postMessage() {}
  close() {}
}
afterEach(() => vi.unstubAllGlobals());

const v = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/link-author-v1.json', import.meta.url), 'utf8'),
);
type Kind = 'link' | 'member';
const REVISION = 6n;
async function world(kind: Kind, reader?: object) {
  const seed = new Uint8Array(32).fill(0x61);
  const device = await c.link.deriveDevice(
    seed,
    v.spaceId,
    '50000000-0000-4000-8000-000000000001',
    'door',
  );
  const root = await crypto.subtle.importKey(
    'raw',
    c.binary(v.epochSecret, 32, 32),
    'HKDF',
    false,
    ['deriveBits'],
  );
  // The checker of the device's own kind accepts it; the other one must never be asked.
  const own = vi.fn(async () => device.signingPublic);
  const foreign = vi.fn(() => {
    throw new Error(`the ${kind === 'link' ? 'member' : 'link'} path checked a ${kind} device`);
  });
  const admission = {
    space: v.spaceId,
    page: v.pageId,
    epoch: v.epoch,
    root,
    head: { revision: REVISION },
    closeKeys() {},
    validatePage: vi.fn(),
    registration: {
      deviceId: device.id,
      keys: { sign: device.sign },
      chain: { certificate: () => ({ issuerKind: kind }) },
      syncUrl: 'wss://example.test/colab/sync?tmt-session=fixture',
    },
    author: kind === 'member' ? own : foreign,
    linkAuthor: kind === 'link' ? own : foreign,
    ownAuthor: Admission.prototype.ownAuthor,
    ...(reader ? { reader } : {}),
  };
  return { device, admission, own, foreign };
}
/** An `own` update this device sealed at the admission's head, as the writer produces it. */
async function sealed(w: Awaited<ReturnType<typeof world>>) {
  const envelope = await c.Envelope.seal(
    {
      space: v.spaceId,
      page: v.pageId,
      epoch: v.epoch,
      kind: 'update',
      namespace: 'own',
      authorDevice: w.device.id,
      membershipRevision: REVISION.toString(),
      streamSeq: '5',
      prevHash: new Uint8Array(32),
    },
    w.admission.root,
    w.device.sign,
    new Uint8Array([1, 2, 3]),
  );
  return {
    seq: '5',
    envelopeHash: c.encodeBinary(await envelope.hash()),
    envelope: c.encodeBinary(envelope.toJson()),
  };
}

for (const kind of ['link', 'member'] as Kind[])
  it(`checks a ${kind} device's append by its own kind of author, never the other`, async () => {
    const w = await world(kind);
    vi.stubGlobal('WebSocket', Socket);
    const connection = new Connection(
      w.admission as unknown as Admission,
      new URL('https://example.test/colab/'),
      'private',
      vi.fn(),
      vi.fn(),
    );
    // The catchup frame completes the connection.
    Socket.current.onmessage?.({
      data: JSON.stringify({
        version: 1,
        space: v.spaceId,
        page: v.pageId,
        epoch: v.epoch,
        type: 'catchup',
        streams: [],
        more: false,
      }),
    });
    await connection.ready;
    const sent = connection.append(await sealed(w));
    sent.catch(() => {});
    await vi.waitFor(() =>
      expect(
        Socket.current.send.mock.calls.some(([raw]) => JSON.parse(raw).type === 'append'),
      ).toBe(true),
    );
    expect(w.own).toHaveBeenCalledTimes(1);
    expect(w.own).toHaveBeenCalledWith(w.device.id, REVISION.toString());
    expect(w.foreign).not.toHaveBeenCalled();
    connection.close();
  });

for (const kind of ['link', 'member'] as Kind[])
  it(`seals a ${kind} device's own update under its own kind of author, never the other`, async () => {
    const w = await world(kind);
    vi.stubGlobal('BroadcastChannel', Channel);
    vi.stubGlobal('navigator', {
      locks: { request: (_key: string, _o: unknown, fn: () => unknown) => fn() },
    });
    const appended: unknown[] = [];
    const connection = {
      ready: Promise.resolve(),
      active: true,
      admission: w.admission,
      objects: { head: () => ({ seq: 4n, hash: new Uint8Array(32) }) },
      run: (fn: () => Promise<unknown>) => fn(),
      fold: {
        run: vi.fn(async (command: { type: string }) =>
          command.type === 'prepare-own'
            ? { update: new Uint8Array([1, 2, 3]) }
            : command.type === 'own-conflict'
              ? { update: new Uint8Array() }
              : {},
        ),
      },
      append: vi.fn(async (entry: unknown) => void appended.push(entry)),
      waitForOwnRecords: vi.fn(async () => {}),
    };
    const writer = new Writer(
      'own-author',
      async () => connection as unknown as Connection,
      memoryRecords(),
    );
    try {
      await writer.submitOwnRecords([{ root: 'messages', key: 'k', value: { text: 'hi' } }]);
      expect(w.own).toHaveBeenCalledTimes(1);
      expect(w.own).toHaveBeenCalledWith(w.device.id, REVISION.toString());
      expect(w.foreign).not.toHaveBeenCalled();
      expect(appended).toHaveLength(1);
      // What was sealed is this device's stream entry, which the checker's key opens.
      const entry = appended[0] as { seq: string; envelope: string };
      const header = c.decodeHeader(
        c.Envelope.fromJson(c.binary(entry.envelope, 1 << 20)).header(),
      );
      expect(header.context).toMatchObject({ namespace: 'own', authorDevice: w.device.id });
      expect(entry.seq).toBe('5');
    } finally {
      writer.close();
    }
  });

it('refuses a prepared update that the sender says is too large, before sealing or sending', async () => {
  const w = await world('link');
  vi.stubGlobal('BroadcastChannel', Channel);
  vi.stubGlobal('navigator', {
    locks: { request: (_key: string, _o: unknown, fn: () => unknown) => fn() },
  });
  const prepared = new Uint8Array([1, 2, 3]);
  const connection = {
    ready: Promise.resolve(),
    active: true,
    admission: w.admission,
    run: (fn: () => Promise<unknown>) => fn(),
    fold: { run: vi.fn(async () => ({ update: prepared })) },
    append: vi.fn(),
    waitForOwnRecords: vi.fn(),
  };
  const writer = new Writer(
    'own-author-fits',
    async () => connection as unknown as Connection,
    memoryRecords(),
  );
  try {
    await expect(
      writer.submitOwnRecords([{ root: 'messages', key: 'k', value: { text: 'hi' } }], (size) => {
        expect(size).toBe(3);
        throw new Error('too large');
      }),
    ).rejects.toThrow('too large');
    expect(w.own).not.toHaveBeenCalled();
    expect(connection.append).not.toHaveBeenCalled();
    expect(prepared).toEqual(new Uint8Array([0, 0, 0]));
  } finally {
    writer.close();
  }
});

const error = (code: string) =>
  JSON.stringify({
    version: 1,
    type: 'error',
    space: v.spaceId,
    page: v.pageId,
    epoch: v.epoch,
    code,
  });
/** A connection of `kind`, with an append of its device's own stream waiting for its receipt (or
 * none), to which the serve then answers `code`. Returns what the connection closed with, and what
 * the waiting append (if any) was refused with. */
async function answered(kind: Kind, reader: boolean, waiting: boolean, code: string) {
  const w = await world(kind, reader ? {} : undefined);
  vi.stubGlobal('WebSocket', Socket);
  const failed = vi.fn();
  const connection = new Connection(
    w.admission as unknown as Admission,
    new URL('https://example.test/colab/'),
    'private',
    vi.fn(),
    failed,
  );
  Socket.current.onmessage?.({
    data: JSON.stringify({
      version: 1,
      space: v.spaceId,
      page: v.pageId,
      epoch: v.epoch,
      type: 'catchup',
      streams: [],
      more: false,
    }),
  });
  await connection.ready;
  const refusal = waiting
    ? connection.append(await sealed(w)).then(
        () => undefined,
        (failure: Error) => failure,
      )
    : Promise.resolve(undefined);
  if (waiting)
    await vi.waitFor(() =>
      expect(
        Socket.current.send.mock.calls.some(([raw]) => JSON.parse(raw).type === 'append'),
      ).toBe(true),
    );
  Socket.current.onmessage?.({ data: error(code) });
  await vi.waitFor(() => expect(failed).toHaveBeenCalledOnce());
  return { closed: failed.mock.calls[0][0] as Error, refusal: await refusal };
}

it('tells a link session its append was refused only when the serve answered CONFLICT to it', async () => {
  const refused = await answered('link', true, true, 'CONFLICT');
  expect(refused.refusal).toBeInstanceOf(OwnAppendRefused);
  expect(refused.refusal).toMatchObject({ code: 'CONFLICT', message: 'CONFLICT' });
  expect(refused.closed).toBeInstanceOf(OwnAppendRefused);
  // Not for another answer, with no append waiting, or for a device that is not a link's.
  for (const [kind, reader, waiting, code] of [
    ['link', true, true, 'DENIED'],
    ['link', true, true, 'GAP'],
    ['link', true, false, 'CONFLICT'],
    ['member', false, true, 'CONFLICT'],
  ] as [Kind, boolean, boolean, string][]) {
    const other = await answered(kind, reader, waiting, code);
    const label = `${kind} ${code} waiting=${waiting}`;
    expect(other.closed, label).not.toBeInstanceOf(OwnAppendRefused);
    expect(other.closed, label).toMatchObject({ message: code });
    if (waiting) expect(other.refusal, label).not.toBeInstanceOf(OwnAppendRefused);
  }
});

/** A writer over a fake connection whose appends answer as `appends` says, in order. */
async function writerOver(appends: (Error | undefined)[]) {
  const w = await world('link');
  vi.stubGlobal('BroadcastChannel', Channel);
  vi.stubGlobal('navigator', {
    locks: { request: (_key: string, _o: unknown, fn: () => unknown) => fn() },
  });
  let head = 4n;
  const appended: { seq: string; envelope: string }[] = [];
  const connection = {
    ready: Promise.resolve(),
    active: true,
    admission: w.admission,
    objects: { head: () => ({ seq: head, hash: new Uint8Array(32) }) },
    run: (fn: () => Promise<unknown>) => fn(),
    fold: {
      run: vi.fn(async (command: { type: string }) =>
        command.type === 'prepare-own'
          ? { update: new Uint8Array([1, 2, 3]) }
          : command.type === 'own-conflict'
            ? { update: new Uint8Array() }
            : {},
      ),
    },
    append: vi.fn(async (entry: { seq: string; envelope: string }) => {
      appended.push(entry);
      const failure = appends.shift();
      if (failure) throw failure;
      head = BigInt(entry.seq);
    }),
    waitForOwnRecords: vi.fn(async () => {}),
  };
  const records = memoryRecords();
  const writer = new Writer(
    'own-author-refused',
    async () => connection as unknown as Connection,
    records,
  );
  const save = () =>
    writer.submitOwnRecords([{ root: 'messages', key: 'k', value: { text: 'hi' } }]);
  return {
    writer,
    save,
    appended,
    saved: () =>
      records<{
        pending: { entry: { seq: string; envelope: string } } | null;
        completed: string[];
      }>('own-author-refused'),
    /** Another holder's original now holds the sequence. */
    advance: () => {
      head += 1n;
    },
  };
}

it('drops an append the serve refused as taken, and seals the next write at the new head', async () => {
  const w = await writerOver([new OwnAppendRefused('CONFLICT'), undefined]);
  try {
    await expect(w.save()).rejects.toBeInstanceOf(OwnAppendRefused);
    // Never stored, so never kept: nothing is left to retry.
    expect((await w.saved())?.pending).toBeNull();
    w.advance();
    await w.save();
    expect(w.appended.map((entry) => entry.seq)).toEqual(['5', '6']);
    expect((await w.saved())?.pending).toBeNull();
  } finally {
    w.writer.close();
  }
});

it('keeps an append whose outcome is unknown and retries exactly those bytes first', async () => {
  const w = await writerOver([new Error('Sync disconnected'), undefined, undefined]);
  try {
    await expect(w.save()).rejects.not.toBeInstanceOf(OwnAppendRefused);
    const kept = (await w.saved())?.pending;
    expect(kept?.entry.seq).toBe('5');
    await w.save();
    // The stored entry is appended again unchanged, then the new write follows it.
    expect(w.appended[1]).toEqual(kept!.entry);
    expect(w.appended[1].envelope).toBe(w.appended[0].envelope);
    expect(w.appended.map((entry) => entry.seq)).toEqual(['5', '5', '6']);
    expect((await w.saved())?.pending).toBeNull();
  } finally {
    w.writer.close();
  }
});

it('drops an older kept append that the serve refuses when it is retried', async () => {
  const w = await writerOver([
    new Error('Sync disconnected'),
    new OwnAppendRefused('CONFLICT'),
    undefined,
  ]);
  try {
    await expect(w.save()).rejects.not.toBeInstanceOf(OwnAppendRefused);
    await expect(w.save()).rejects.toBeInstanceOf(OwnAppendRefused);
    expect((await w.saved())?.pending).toBeNull();
    w.advance();
    await w.save();
    expect(w.appended.map((entry) => entry.seq)).toEqual(['5', '5', '6']);
  } finally {
    w.writer.close();
  }
});
