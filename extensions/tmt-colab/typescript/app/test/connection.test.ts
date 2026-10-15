import { readFileSync } from 'node:fs';
import { afterEach, expect, it, vi } from 'vite-plus/test';
import { binary, concat, digest, encodeBinary, text } from '@tmt/colab-client';
import type { TitleReadBudget } from '../src/transport.js';
import type { Admission } from '../src/admission.js';
import { Connection } from '../src/connection.js';
import type { OwnState } from '../src/fold-protocol.js';
import { SAVE_SOURCE_BYTES, SaveTooLarge } from '../src/save.js';

const calls = vi.hoisted(() => [] as string[]);
const projection = vi.hoisted(() => ({ own: undefined as OwnState | undefined }));
vi.mock('../src/fold.js', () => ({
  Fold: class {
    async run({ updates }: { updates?: Uint8Array[] }) {
      calls.push('fold');
      return {
        source: updates?.length ? 'updated' : 'initial',
        title: 'Title',
        ...(projection.own ? { own: projection.own } : {}),
      };
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
    ownData = false;
    head() {
      return { seq: 4n };
    }
    cursors() {
      return [];
    }
    async admit() {
      calls.push('admit');
      return { namespace: 'content', writer: author, update: new Uint8Array([1]) };
    }
    noteCut(stream: string, namespace: string, entry: { seq: string; envelopeHash: string }) {
      calls.push(`notice ${stream} ${namespace} ${entry.seq}`);
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
  close = vi.fn(() => {
    this.readyState = Socket.CLOSED;
    this.dispatchEvent(new Event('close'));
  });
  constructor(readonly url: string | URL) {
    super();
    Socket.current = this;
  }
  receive(frame: Record<string, unknown>) {
    this.onmessage?.({ data: JSON.stringify(frame) });
  }
}
const page = '10000000-0000-4000-8000-000000000001';
const author = '30000000-0000-4000-8000-000000000001';
const scope = { version: 1, space: 'a'.repeat(32), page, epoch: '1' };
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});
async function open(rejectChain = false, budget?: TitleReadBudget) {
  calls.length = 0;
  projection.own = undefined;
  vi.stubGlobal('WebSocket', Socket);
  const admission = {
    ...scope,
    root: new Uint8Array(32),
    closeKeys() {},
    validatePage: vi.fn(),
    registration: {
      deviceId: author,
      syncUrl: 'wss://example.test/colab/sync?tmt-session=fixture',
    },
    async chains(value: unknown) {
      calls.push('chain');
      expect(value).toEqual([{ deviceId: author, chain: 'fixture' }]);
      if (rejectChain) throw new Error('Forged chain');
    },
  } as unknown as Admission;
  const publish = vi.fn(),
    failed = vi.fn();
  const connection = new Connection(
    admission,
    new URL('https://example.test/colab/'),
    'private',
    publish,
    failed,
    undefined,
    false,
    budget,
  );
  expect(Socket.current.url).toBe(admission.registration.syncUrl);
  Socket.current.receive({ ...scope, type: 'catchup', streams: [], more: false });
  await connection.ready;
  calls.length = 0;
  return { connection, socket: Socket.current, publish, failed };
}
function broadcast(chains = true) {
  return {
    ...scope,
    type: 'broadcast',
    streamId: author,
    seq: '1',
    envelopeHash: encodeBinary(new Uint8Array(32)),
    envelope: 'AA',
    ...(chains ? { chains: [{ deviceId: author, chain: 'fixture' }] } : {}),
  };
}
function notice(change: Record<string, unknown> = {}) {
  return {
    ...scope,
    type: 'checkpoint',
    streamId: author,
    namespace: 'content',
    seq: '3',
    envelopeHash: encodeBinary(new Uint8Array(32).fill(5)),
    ...change,
  };
}
it('takes a checkpoint notice as an unseen cut: nothing is admitted, folded or published', async () => {
  const { connection, socket, publish, failed } = await open();
  publish.mockClear();
  socket.send.mockClear();
  try {
    socket.receive(notice());
    socket.receive(notice({ namespace: 'own', seq: '4' }));
    await connection.run(async () => {});
    expect(calls).toEqual([`notice ${author} content 3`, `notice ${author} own 4`]);
    expect(publish).not.toHaveBeenCalled();
    expect(failed).not.toHaveBeenCalled();
    // Each frame is acknowledged as a frame, with no position the notice would have advanced.
    expect(socket.send.mock.calls.filter(([raw]) => JSON.parse(raw).type === 'ack')).toHaveLength(
      2,
    );
  } finally {
    connection.close();
  }
});
it('closes on a checkpoint notice that is not exactly a notice', async () => {
  for (const bad of [
    notice({ namespace: 'other' }),
    notice({ seq: '03' }),
    notice({ streamId: 'not-an-id' }),
    notice({ envelopeHash: 'AA' }),
    notice({ envelope: 'AA' }),
  ]) {
    const { connection, socket, publish, failed } = await open();
    publish.mockClear();
    socket.receive(bad);
    await vi.waitFor(() => expect(failed).toHaveBeenCalledOnce());
    expect(calls.some((call) => call.startsWith('notice'))).toBe(false);
    expect(publish).not.toHaveBeenCalled();
    connection.close();
  }
});
it('verifies live author chains before envelope admission and Worker publication', async () => {
  const { connection, socket, publish, failed } = await open();
  try {
    socket.receive(broadcast());
    await connection.run(async () => {});
    expect(calls).toEqual(['chain', 'admit', 'fold']);
    expect(publish).toHaveBeenLastCalledWith({ source: 'updated', title: 'Title', ownData: false });
    expect(failed).not.toHaveBeenCalled();
    calls.length = 0;
    socket.receive(broadcast(false));
    await connection.run(async () => {});
    expect(calls).toEqual(['admit', 'fold']);
  } finally {
    connection.close();
  }
});
it('closes a forged live-chain broadcast before decoding or publishing its envelope', async () => {
  const { connection, socket, publish, failed } = await open(true);
  try {
    socket.receive(broadcast());
    await vi.waitFor(() => expect(failed).toHaveBeenCalledWith(new Error('Forged chain')));
    expect(calls).toEqual(['chain']);
    expect(publish).toHaveBeenCalledTimes(1);
    expect(socket.close).toHaveBeenCalledOnce();
  } finally {
    connection.close();
  }
});

it('waits for the exact records in the local writer projection before dependent reads', async () => {
  const { connection, socket, publish } = await open();
  const records = [
    { root: 'messages' as const, key: 'comment', value: { text: 'Hello', revision: 1 } },
  ];
  let completed = false;
  const pending = connection.waitForOwnRecords(records).then(() => {
    completed = true;
  });
  const own = (value: { text: string; revision: number }) => ({
    threads: {},
    intents: {},
    replies: {},
    messages: { comment: value },
  });
  try {
    projection.own = { foreign: own(records[0].value) };
    socket.receive(broadcast());
    await connection.run(async () => {});
    expect(completed).toBe(false);
    projection.own = { [author]: own({ text: 'Different', revision: 1 }) };
    socket.receive(broadcast());
    await connection.run(async () => {});
    expect(completed).toBe(false);
    // JSON object field order is cosmetic. Admission/fold and publication must precede resolution.
    projection.own = { [author]: own({ revision: 1, text: 'Hello' }) };
    socket.receive(broadcast());
    await pending;
    expect(publish).toHaveBeenLastCalledWith(expect.objectContaining({ own: projection.own }));
    await expect(connection.waitForOwnRecords(records)).resolves.toBeUndefined();
  } finally {
    connection.close();
  }
});

it('rejects an unobserved own publication on close without publishing it', async () => {
  const { connection, publish } = await open();
  const pending = connection.waitForOwnRecords([{ root: 'messages', key: 'missing', value: null }]);
  const rejected = expect(pending).rejects.toThrow('Disconnected');
  connection.close(new Error('Disconnected'));
  await rejected;
  expect(publish).toHaveBeenCalledTimes(1);
});

it('bounds own publication catchup and closes the unavailable connection', async () => {
  vi.useFakeTimers();
  const { connection, failed, socket, publish } = await open();
  const pending = connection.waitForOwnRecords([{ root: 'messages', key: 'missing', value: null }]);
  const rejected = expect(pending).rejects.toThrow('Own publication catchup timed out');
  await vi.advanceTimersByTimeAsync(10_000);
  await rejected;
  expect(connection.active).toBe(false);
  expect(failed).toHaveBeenCalledWith(
    expect.objectContaining({
      message: 'Own publication catchup timed out',
      diagnostic: {
        root: 'messages',
        kind: 'unknown',
        streamId: author,
        submittedSeq: null,
        admittedSeq: '4',
        leaderReceipt: false,
      },
    }),
  );
  expect(socket.close).toHaveBeenCalledOnce();
  expect(publish).toHaveBeenCalledTimes(1);
});

it('names an unadmitted own publication without retaining its key or plaintext', async () => {
  vi.useFakeTimers();
  const { connection, failed } = await open();
  const secret = 'PRIVATE_RECORD_SENTINEL';
  const pending = connection.waitForOwnRecords(
    [{ root: 'messages', key: secret, value: { kind: 'comment', body: secret } }],
    { seq: '7' },
  );
  const rejected = expect(pending).rejects.toThrow('Own publication catchup timed out');
  await vi.advanceTimersByTimeAsync(10_000);
  await rejected;
  const error = failed.mock.calls[0][0];
  expect(error).toMatchObject({
    message: 'Own publication catchup timed out',
    diagnostic: {
      root: 'messages',
      kind: 'comment',
      streamId: author,
      submittedSeq: '7',
      admittedSeq: '4',
      leaderReceipt: true,
    },
  });
  expect(JSON.stringify(error)).not.toContain(secret);
});

const operation = '40000000-0000-4000-8000-000000000001';
/** Frames the page asked for; acknowledgements of what it received are not requests. */
const sent = (socket: Socket) =>
  socket.send.mock.calls
    .map(([raw]) => JSON.parse(raw as string) as Record<string, unknown>)
    .filter((frame) => frame.type !== 'ack');
const savedFrame = (state: Record<string, unknown>) => ({
  ...scope,
  type: 'saveresult',
  operationId: operation,
  ...state,
});
const awaitSent = (socket: Socket, count: number) =>
  vi.waitFor(() => expect(sent(socket).length).toBeGreaterThanOrEqual(count));

it('saves a small source inline, bound to the base and source digests, and resolves on the reply', async () => {
  const { connection, socket, failed } = await open();
  try {
    const pending = connection.save(operation, '<p>old</p>', '<p>new</p>');
    await awaitSent(socket, 1);
    expect(sent(socket)).toEqual([
      {
        ...scope,
        type: 'save',
        operationId: operation,
        baseSha256: encodeBinary(await digest(text('<p>old</p>'))),
        sourceSha256: encodeBinary(await digest(text('<p>new</p>'))),
        source: encodeBinary(text('<p>new</p>')),
      },
    ]);
    socket.receive(savedFrame({ state: 'committed', revision: 'v1:00ff' }));
    await expect(pending).resolves.toEqual({
      operationId: operation,
      state: 'committed',
      revision: 'v1:00ff',
    });
    expect(failed).not.toHaveBeenCalled();
  } finally {
    connection.close();
  }
});

it('sends a typed attachment change with the save, omits an empty one and refuses free-form metadata', async () => {
  const oracle = JSON.parse(
    readFileSync(
      new URL('../../../contracts/vectors/attachment-change-v1.json', import.meta.url),
      'utf8',
    ),
  );
  const { connection, socket } = await open();
  try {
    const change = { set: [oracle.descriptors.a], remove: [oracle.descriptors.b.attachmentId] };
    const pending = connection.save(operation, '<p>old</p>', oracle.source, change);
    await awaitSent(socket, 1);
    expect(sent(socket)[0]).toMatchObject({ type: 'save', attachments: change });
    socket.receive(savedFrame({ state: 'committed', revision: 'v1:00ff' }));
    await pending;
    const plain = connection.save(operation, '<p>old</p>', '<p>new</p>', {});
    await awaitSent(socket, 2);
    expect(sent(socket)[1]).not.toHaveProperty('attachments');
    socket.receive(savedFrame({ state: 'committed', revision: 'v1:00ff' }));
    await plain;
    await expect(
      connection.save(operation, 'old', 'new', { meta: { title: 'x' } } as never),
    ).rejects.toThrow();
    expect(sent(socket)).toHaveLength(2);
  } finally {
    connection.close();
  }
});

it('uploads a large source as a reference and ordered 32 KiB chunks that rebuild it exactly', async () => {
  const { connection, socket } = await open();
  try {
    const source = 'é'.repeat(100_000) + '<p>end</p>';
    const bytes = text(source);
    const pending = connection.save(operation, 'old', source);
    const count = Math.ceil(bytes.length / (32 * 1024));
    await awaitSent(socket, 1 + count);
    const [save, ...chunks] = sent(socket);
    const hash = encodeBinary(await digest(bytes));
    expect(save).toMatchObject({ type: 'save', sourceSha256: hash });
    const objectId = (save.source as { objectId: string }).objectId;
    expect(objectId).toMatch(/^[0-9a-f]{64}$/);
    expect(chunks).toHaveLength(count);
    chunks.forEach((chunk, index) =>
      expect(chunk).toMatchObject({ type: 'chunk', objectId, envelopeHash: hash, index, count }),
    );
    expect(concat(...chunks.map((chunk) => binary(chunk.bytes as string, 32 * 1024)))).toEqual(
      bytes,
    );
    socket.receive(savedFrame({ state: 'unchanged', revision: 'v1:3a3a' }));
    await expect(pending).resolves.toMatchObject({ state: 'unchanged', revision: 'v1:3a3a' });
  } finally {
    connection.close();
  }
});

it('refuses a source over the page limit before sending anything, naming both numbers', async () => {
  const { connection, socket } = await open();
  try {
    const rejected = await connection
      .save(operation, 'old', 'x'.repeat(SAVE_SOURCE_BYTES + 1))
      .catch((error) => error);
    expect(rejected).toBeInstanceOf(SaveTooLarge);
    expect(rejected).toMatchObject({ size: SAVE_SOURCE_BYTES + 1, limit: SAVE_SOURCE_BYTES });
    expect(sent(socket)).toEqual([]);
  } finally {
    connection.close();
  }
});

it('carries every refusal reply with its stable code and message', async () => {
  const { connection, socket } = await open();
  try {
    const pending = connection.save(operation, 'old', 'new');
    await awaitSent(socket, 1);
    socket.receive(savedFrame({ state: 'rejected', code: 'COLAB_STALE_BASE', message: 'Moved.' }));
    await expect(pending).resolves.toEqual({
      operationId: operation,
      state: 'rejected',
      code: 'COLAB_STALE_BASE',
      message: 'Moved.',
    });
  } finally {
    connection.close();
  }
});

it('allows one save or status request at a time per connection', async () => {
  const { connection, socket } = await open();
  try {
    const first = connection.save(operation, 'old', 'new');
    await awaitSent(socket, 1);
    await expect(connection.saveStatus(operation)).rejects.toThrow();
    socket.receive(savedFrame({ state: 'committed', revision: '1' }));
    await first;
    const status = connection.saveStatus(operation);
    await awaitSent(socket, 2);
    expect(sent(socket).at(-1)).toEqual({ ...scope, type: 'savestatus', operationId: operation });
    socket.receive(savedFrame({ state: 'absent' }));
    await expect(status).resolves.toEqual({ operationId: operation, state: 'absent' });
  } finally {
    connection.close();
  }
});

it('closes on a reply for another operation or one nobody asked for', async () => {
  for (const reply of [
    savedFrame({
      state: 'committed',
      revision: '1',
      operationId: '40000000-0000-4000-8000-000000000002',
    }),
    savedFrame({ state: 'committed' }),
    savedFrame({ state: 'rejected', code: 'lowercase' }),
  ]) {
    const { connection, socket, failed } = await open();
    const pending = connection.save(operation, 'old', 'new');
    const rejected = expect(pending).rejects.toThrow();
    await awaitSent(socket, 1);
    socket.receive(reply);
    await rejected;
    expect(failed).toHaveBeenCalledOnce();
    expect(connection.active).toBe(false);
  }
  const { connection, socket, failed } = await open();
  socket.receive(savedFrame({ state: 'committed', revision: '1' }));
  await vi.waitFor(() => expect(failed).toHaveBeenCalledOnce());
  connection.close();
});

it('rejects an unanswered save when the connection closes and when the reply budget runs out', async () => {
  const lost = await open();
  const pending = lost.connection.save(operation, 'old', 'new');
  const rejected = expect(pending).rejects.toThrow('Sync disconnected');
  await awaitSent(lost.socket, 1);
  lost.connection.close(new Error('Sync disconnected'));
  await rejected;

  vi.useFakeTimers();
  const slow = await open();
  const waiting = slow.connection.save(operation, 'old', 'new');
  const timedOut = expect(waiting).rejects.toThrow('Save reply timed out');
  await awaitSent(slow.socket, 1);
  await vi.advanceTimersByTimeAsync(30_000);
  await timedOut;
  expect(slow.connection.active).toBe(false);
});

it('correlates object replies without blocking the inbound executor and keeps bare refusal usable', async () => {
  const { connection, socket, failed } = await open();
  try {
    const result = connection.attachmentObjects.config(performance.now() + 15_000);
    await awaitSent(socket, 1);
    const request = JSON.parse(socket.send.mock.calls.at(-1)![0]);
    expect(request).toMatchObject({ ...scope, type: 'object', request: { method: 'config' } });
    socket.receive({
      ...scope,
      type: 'object-result',
      requestId: request.requestId,
      result: { error: { code: 'unavailable' } },
    });
    expect(await result).toEqual({ error: { code: 'unavailable' } });
    expect(connection.active).toBe(true);
    expect(failed).not.toHaveBeenCalled();
    const next = connection.attachmentObjects.config(performance.now() + 15_000);
    const refused = expect(next).rejects.toThrow('Sync disconnected');
    await awaitSent(socket, 2);
    connection.close(new Error('Sync disconnected'));
    await refused;
    expect(sent(socket).filter((frame) => frame.type === 'object')).toHaveLength(2);
  } finally {
    connection.close();
  }
});

it('object method confusion ends the peer and outstanding requests cannot move to a successor', async () => {
  const first = await open();
  const result = first.connection.attachmentObjects.config(performance.now() + 15_000);
  const rejected = expect(result).rejects.toThrow();
  await awaitSent(first.socket, 1);
  const request = JSON.parse(first.socket.send.mock.calls.at(-1)![0]);
  first.socket.receive({
    ...scope,
    type: 'object-result',
    requestId: request.requestId,
    result: { ok: { result: 'read', offset: 0, totalBytes: 1, bytes: 'AA' } },
  });
  await rejected;
  expect(first.connection.active).toBe(false);
  const next = await open();
  next.socket.receive({
    ...scope,
    type: 'object-result',
    requestId: request.requestId,
    result: { error: { code: 'unavailable' } },
  });
  await vi.waitFor(() => expect(next.failed).toHaveBeenCalledOnce());
  expect(next.connection.active).toBe(false);
  next.connection.close();
});

it('an unanswered object request settles as storage unavailable within its original budget', async () => {
  vi.useFakeTimers();
  const { connection, socket, failed } = await open();
  const waiting = connection.attachmentObjects.config(performance.now() + 15_000);
  const refused = expect(waiting).rejects.toThrow('Storage unavailable');
  await awaitSent(socket, 1);
  await vi.advanceTimersByTimeAsync(15_000);
  await refused;
  expect(failed).toHaveBeenCalledOnce();
  expect(connection.active).toBe(false);
  expect(sent(socket).filter((frame) => frame.type === 'object')).toHaveLength(1);
});

const fullUsage = {
  updates: 124,
  updateLimit: 200,
  tailBytes: 1_258_291,
  tailBytesLimit: 4_194_304,
  checkpointBytes: 4000,
  otherDeviceUpdates: 4,
  stateBytes: 1_262_291,
  gzipLimit: 5_000_000,
  percent: 62,
  limiting: 'changes',
};
const reportOf = (extra: Record<string, unknown> = {}) => ({
  pageId: '10000000-0000-4000-8000-000000000001',
  compacted: true,
  by: '30000000-0000-4000-8000-000000000001',
  through: 120,
  mergedObjects: 120,
  sourceSha256: 'a'.repeat(64),
  before: fullUsage,
  after: { ...fullUsage, updates: 5, percent: 3 },
  ...extra,
});

it('asks for usage with an empty frame and resolves to exactly what the serve answered', async () => {
  const { connection, socket, failed } = await open();
  try {
    const pending = connection.usage();
    await awaitSent(socket, 1);
    expect(sent(socket)).toEqual([{ ...scope, type: 'usage' }]);
    socket.receive({ ...scope, type: 'usageresult', usage: fullUsage });
    await expect(pending).resolves.toEqual(fullUsage);
    expect(failed).not.toHaveBeenCalled();
    expect(connection.active).toBe(true);
  } finally {
    connection.close();
  }
});

it('asks to compact with an empty frame and resolves to the report', async () => {
  const { connection, socket } = await open();
  try {
    const pending = connection.compact();
    await awaitSent(socket, 1);
    expect(sent(socket)).toEqual([{ ...scope, type: 'compact' }]);
    socket.receive({ ...scope, type: 'compactresult', report: reportOf() });
    await expect(pending).resolves.toEqual(reportOf());
  } finally {
    connection.close();
  }
});

it('a refusal rejects the request with its code and keeps the connection', async () => {
  const { connection, socket, failed } = await open();
  try {
    const pending = connection.compact();
    await awaitSent(socket, 1);
    socket.receive({
      ...scope,
      type: 'compactresult',
      error: { code: 'COLAB_PAGE_INACTIVE', message: 'Archived.' },
    });
    await expect(pending).rejects.toMatchObject({ code: 'COLAB_PAGE_INACTIVE' });
    expect(failed).not.toHaveBeenCalled();
    expect(connection.active).toBe(true);
    // The slot is free again: a second request goes out.
    const next = connection.usage();
    await awaitSent(socket, 2);
    socket.receive({ ...scope, type: 'usageresult', usage: fullUsage });
    await expect(next).resolves.toEqual(fullUsage);
  } finally {
    connection.close();
  }
});

it('closes the connection for a reply nobody asked for, of the wrong kind or malformed', async () => {
  for (const [ask, reply] of [
    [null, { type: 'usageresult', usage: fullUsage }],
    ['usage', { type: 'compactresult', report: reportOf() }],
    ['usage', { type: 'usageresult', usage: { ...fullUsage, extra: 1 } }],
    ['compact', { type: 'compactresult', report: reportOf({ compacted: false }) }],
  ] as const) {
    const { connection, socket, failed } = await open();
    const pending = ask ? connection[ask]() : null;
    const rejected = pending ? expect(pending).rejects.toThrow() : null;
    if (ask) await awaitSent(socket, 1);
    socket.receive({ ...scope, ...reply });
    await vi.waitFor(() => expect(failed).toHaveBeenCalledOnce());
    expect(connection.active).toBe(false);
    await rejected;
  }
});

it('keeps one usage or compact request in flight, and closes when the reply never comes', async () => {
  vi.useFakeTimers();
  const { connection, socket, failed } = await open();
  const pending = connection.compact();
  const rejected = expect(pending).rejects.toThrow('Page reply timed out');
  await awaitSent(socket, 1);
  await expect(connection.usage()).rejects.toThrow();
  expect(sent(socket)).toHaveLength(1);
  await vi.advanceTimersByTimeAsync(20_000);
  await rejected;
  expect(failed).toHaveBeenCalledWith(new Error('Page reply timed out'));
  expect(connection.active).toBe(false);
});

it('a title fold never subscribes and charges received frames before admitting their bytes', async () => {
  const budget = { remainingBytes: 1000 };
  const { connection, socket, failed, publish } = await open(false, budget);
  try {
    const initial = { ...scope, type: 'catchup', streams: [], more: false };
    expect(budget.remainingBytes).toBe(1000 - text(JSON.stringify(initial)).length);
    expect(socket.send.mock.calls.map(([raw]) => JSON.parse(raw).type)).not.toContain('subscribe');
    publish.mockClear();
    budget.remainingBytes = 1;
    socket.receive(broadcast());
    expect(failed).toHaveBeenCalledWith(
      expect.objectContaining({ message: 'Title read byte budget exhausted' }),
    );
    expect(publish).not.toHaveBeenCalled();
    expect(socket.close).toHaveBeenCalledOnce();
  } finally {
    await connection.closeAndJoin();
  }
});

it('close-and-join waits for in-flight admission work before another title reader may start', async () => {
  const { connection } = await open();
  let release!: () => void,
    started = false;
  const gate = new Promise<void>((done) => {
    release = done;
  });
  const task = connection.run(async () => {
    started = true;
    await gate;
  });
  await vi.waitFor(() => expect(started).toBe(true));
  let joined = false;
  const closing = connection.closeAndJoin().then(() => {
    joined = true;
  });
  await Promise.resolve();
  expect(joined).toBe(false);
  release();
  await Promise.all([closing, task]);
  expect(joined).toBe(true);
});

it('a title read joins the socket close handshake and reports an unconfirmed cleanup deadline', async () => {
  const { connection, socket } = await open();
  socket.close.mockImplementation(() => {
    socket.readyState = 2;
  });
  let joined = false;
  const closing = connection.closeAndJoin().then(() => {
    joined = true;
  });
  await Promise.resolve();
  expect(joined).toBe(false);
  socket.readyState = Socket.CLOSED;
  socket.dispatchEvent(new Event('close'));
  await closing;
  expect(joined).toBe(true);

  vi.useFakeTimers();
  const pending = await open();
  pending.socket.close.mockImplementation(() => {
    pending.socket.readyState = 2;
  });
  const deadline = expect(pending.connection.closeAndJoin()).rejects.toThrow('cleanup unconfirmed');
  await vi.advanceTimersByTimeAsync(2000);
  await deadline;
});
