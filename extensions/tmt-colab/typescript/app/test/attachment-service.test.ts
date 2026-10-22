import { expect, it, vi } from 'vite-plus/test';
import { attachment } from '@tmt/colab-client';
import {
  AttachmentObjectChannel,
  type AttachmentObjectOutcome,
  type AttachmentObjectRequest,
} from '../src/attachment-channel.js';
import {
  AttachmentService,
  AttachmentStaleError,
  AttachmentUploadError,
} from '../src/attachment-service.js';
import type { Connection } from '../src/connection.js';

const space = 'a'.repeat(32);
const page = '11111111-1111-4111-8111-111111111111';
const device = '22222222-2222-4222-8222-222222222222';
const messageId = '33333333-3333-4333-8333-333333333333';

type Script = (request: AttachmentObjectRequest, calls: AttachmentObjectRequest[]) => unknown;
const hex = (n: number) => 'ab'.repeat(32).slice(0, n);

async function fixture(
  options: {
    script?: Script;
    revision?: () => string;
    payloadBytes?: number;
    /** What a connection reopened for the object channel answers; none offers no reopen. */
    replacement?: Script;
  } = {},
) {
  const signer = (await crypto.subtle.generateKey('Ed25519', false, [
    'sign',
    'verify',
  ])) as CryptoKeyPair;
  const first = connect(signer, options, options.script);
  const reopened: Connection[] = [];
  const replacement = options.replacement
    ? connect(signer, options, options.replacement)
    : undefined;
  // As in the live page, the connection asked for after a reopen is the replacement.
  let current = first.connection;
  const service = new AttachmentService({
    connection: async () => current,
    available: () => true,
    sharing: 'private',
    ...(replacement && {
      reopen: async (from: Connection) => {
        reopened.push(from);
        return (current = replacement.connection);
      },
    }),
  });
  const { calls, admission, deadlines } = first;
  return { service, calls, admission, deadlines, reopened, replacement };
}
function connect(
  signer: CryptoKeyPair,
  options: { revision?: () => string; payloadBytes?: number },
  script: Script | undefined,
) {
  const calls: AttachmentObjectRequest[] = [];
  /** The deadline each request carried, in order. */
  const deadlines: [AttachmentObjectRequest['method'], number][] = [];
  let received = 0;
  const channel = new AttachmentObjectChannel(
    async (request, deadline): Promise<AttachmentObjectOutcome> => {
      calls.push(structuredClone(request));
      deadlines.push([request.method, deadline]);
      const scripted = script?.(request, calls);
      if (scripted !== undefined) {
        if (scripted instanceof Error) throw scripted;
        return scripted as AttachmentObjectOutcome;
      }
      switch (request.method) {
        case 'config':
          return {
            ok: {
              result: 'config',
              projection: 'browser',
              backend: { id: 'local', source: 'x', editable: false },
              capabilities: { immutableCreate: true, chunkedRead: true, recoverByOriginalId: true },
              limits: { payloadBytes: options.payloadBytes ?? 12 * 1024 * 1024, chunkBytes: 32768 },
            },
          };
        case 'begin':
          received = 0;
          return { ok: { result: 'pending', nextIndex: 0, received: 0 } };
        case 'part':
          received = request.index + 1;
          return { ok: { result: 'progress', nextIndex: request.index + 1, received } };
        case 'commit': {
          const begin = calls.find((c) => c.method === 'begin' || c.method === 'status') as {
            descriptor: attachment.AttachmentDescriptor;
          };
          return {
            ok: {
              result: 'committed',
              opaqueKey: Array.from({ length: 32 }, () => 1).reduce(
                (s, v) => s + String.fromCharCode(v),
                '',
              ),
              payloadSha256: begin.descriptor.payloadSha256,
              payloadBytes: Number(begin.descriptor.payloadBytes),
            },
          };
        }
        default:
          throw new Error(`unscripted ${request.method}`);
      }
    },
  );
  const admission = {
    reader: undefined,
    head: { revision: 1n, hash: new Uint8Array(32) },
    root: {},
    space,
    page,
    epoch: '1',
    registration: { deviceId: device, keys: { sign: signer.privateKey } },
    validatePage() {},
    author() {},
    readRoot: () => new Uint8Array(32),
  };
  const connection = {
    active: true,
    admission,
    attachmentObjects: channel,
    attachmentSnapshot: async () => ({
      admission,
      revision: options.revision?.() ?? `v1:${hex(64)}`,
    }),
  } as unknown as Connection;
  return { connection, calls, admission, deadlines };
}
const input = (size = 100 * 1024) => ({
  filename: 'notes.bin',
  mediaType: 'application/octet-stream',
  bytes: Uint8Array.from({ length: size }, (_, i) => (i * 31 + 7) & 0xff),
});

it('a read of the page has no deadline, while an upload request keeps its own bound', async () => {
  const { service, deadlines } = await fixture();
  await service.limits();
  expect(deadlines).toEqual([['config', Number.POSITIVE_INFINITY]]);
  deadlines.length = 0;
  await service.upload(input(), { kind: 'message', messageId }, () => {});
  expect(deadlines.map(([method]) => method)).toContain('commit');
  for (const [method, deadline] of deadlines.filter(([method]) => method !== 'config'))
    expect(Number.isFinite(deadline), method).toBe(true);
});

it('uploads one frozen original in order and binds it to the preallocated message', async () => {
  const { service, calls } = await fixture();
  const progress: number[] = [];
  const stored = await service.upload(input(), { kind: 'message', messageId }, (sent) =>
    progress.push(sent),
  );
  const methods = calls.map((c) => c.method);
  expect(methods[0]).toBe('config');
  expect(methods[1]).toBe('begin');
  expect(methods.at(-1)).toBe('commit');
  const parts = calls.filter((c) => c.method === 'part') as { index: number; transferId: string }[];
  expect(parts.map((p) => p.index)).toEqual([...parts.keys()]);
  expect(
    new Set(
      calls.filter((c) => 'transferId' in c).map((c) => (c as { transferId: string }).transferId),
    ).size,
  ).toBe(1);
  const d = stored.original.descriptor;
  expect(d.source).toEqual({ kind: 'message', writerId: device, messageId, messageRevision: '1' });
  expect(d.filename).toBe('notes.bin');
  expect(d.mediaType).toBe('application/octet-stream');
  expect(progress.at(-1)).toBe(parts.length);
});

it('a lost part leaves the same original unknown, and status continues it without replay', async () => {
  let failed = false;
  const { service, calls } = await fixture({
    script: (request) => {
      if (request.method === 'part' && request.index === 2 && !failed) {
        failed = true;
        return { error: { code: 'unknown' } };
      }
      if (request.method === 'status')
        return { ok: { result: 'pending', nextIndex: 2, received: 2 * 32768, expiresAtMs: 5 } };
      return undefined;
    },
  });
  const failure = await service
    .upload(input(), { kind: 'message', messageId }, () => {})
    .catch((e) => e);
  expect(failure).toBeInstanceOf(AttachmentUploadError);
  expect(failure.failure.kind).toBe('unknown');
  const original = failure.failure.original;
  const begins = calls.filter((c) => c.method === 'begin').length;
  const stored = await service.resume(
    original,
    { filename: 'notes.bin', size: 100 * 1024 },
    () => {},
  );
  expect(stored.original).toBe(original);
  // No begin again and no new transfer identity: status asks about the same original.
  expect(calls.filter((c) => c.method === 'begin')).toHaveLength(begins);
  const status = calls.find((c) => c.method === 'status') as { transferId: string };
  expect(status.transferId).toBe(original.transferId);
  const resumed = calls.slice(calls.findIndex((c) => c.method === 'status') + 1);
  expect((resumed[0] as { index: number }).index).toBe(2);
  expect(resumed.at(-1)!.method).toBe('commit');
});

it('a thrown request after the first one left is unknown, never refused', async () => {
  const { service } = await fixture({
    script: (request) => (request.method === 'part' ? new Error('socket closed') : undefined),
  });
  const failure = await service
    .upload(input(), { kind: 'message', messageId }, () => {})
    .catch((e) => e);
  expect(failure.failure.kind).toBe('unknown');
});

it('a denied begin is a refusal that sent no part', async () => {
  const { service, calls } = await fixture({
    script: (request) => (request.method === 'begin' ? { error: { code: 'denied' } } : undefined),
  });
  const failure = await service
    .upload(input(), { kind: 'message', messageId }, () => {})
    .catch((e) => e);
  expect(failure.failure).toEqual({ kind: 'refused', reason: 'denied' });
  expect(calls.some((c) => c.method === 'part')).toBe(false);
});

it('a status answer that the original is gone is reported as gone', async () => {
  const { service } = await fixture({
    script: (request) =>
      request.method === 'status' ? { ok: { result: 'state', state: 'expired' } } : undefined,
  });
  const stored = await service.upload(input(10), { kind: 'message', messageId }, () => {});
  const failure = await service
    .resume(stored.original, { filename: 'notes.bin', size: 10 }, () => {})
    .catch((e) => e);
  expect(failure.failure).toEqual({ kind: 'gone' });
});

it('refuses before sealing when the backend cannot recover by original id or the payload is too large', async () => {
  const none = await fixture({
    script: (request) =>
      request.method === 'config'
        ? {
            ok: {
              result: 'config',
              projection: 'browser',
              backend: { id: 'x', source: 'x', editable: false },
              capabilities: {
                immutableCreate: true,
                chunkedRead: true,
                recoverByOriginalId: false,
              },
              limits: { payloadBytes: 1024, chunkBytes: 32768 },
            },
          }
        : undefined,
  });
  const a = await none.service
    .upload(input(10), { kind: 'message', messageId }, () => {})
    .catch((e) => e);
  expect(a.failure).toEqual({ kind: 'refused', reason: 'unavailable' });
  expect(none.calls.map((c) => c.method)).toEqual(['config']);
  const small = await fixture({ payloadBytes: 1024 });
  const b = await small.service
    .upload(input(4096), { kind: 'message', messageId }, () => {})
    .catch((e) => e);
  expect(b.failure).toEqual({ kind: 'refused', reason: 'too-large' });
  expect(small.calls.map((c) => c.method)).toEqual(['config']);
});

it('a message attachment is fenced by the membership head, not the page revision', async () => {
  let revision = `v1:${hex(64)}`;
  const fx = await fixture({ revision: () => revision });
  const { service, admission } = fx;
  const stored = await service.upload(input(10), { kind: 'message', messageId }, () => {});
  // A foreign write moves the page revision: the fence does not, so it is not stale.
  revision = `v1:${'cd'.repeat(32)}`;
  const moved = await service.publication([stored]).catch((e) => e);
  expect(moved).not.toBeInstanceOf(AttachmentStaleError);
  // A membership change does.
  admission.head = { revision: 2n, hash: new Uint8Array(32) };
  const failure = await service.publication([stored]).catch((e) => e);
  expect(failure).toBeInstanceOf(AttachmentStaleError);
  expect(failure.attachmentIds).toEqual([stored.original.descriptor.attachmentId]);
});

it('the message fence is not a page revision and changes with epoch or membership hash', async () => {
  const { service, admission } = await fixture();
  const stored = await service.upload(input(10), { kind: 'message', messageId }, () => {});
  expect(stored.original.base).toMatch(/^v1:[0-9a-f]{64}$/);
  expect(stored.original.base).not.toBe(`v1:${hex(64)}`);
  admission.head = { revision: 1n, hash: Uint8Array.from({ length: 32 }, () => 7) };
  const hashMoved = await service.publication([stored]).catch((e) => e);
  expect(hashMoved).toBeInstanceOf(AttachmentStaleError);
});

it('a status question that cannot be answered leaves the original unknown, not refused', async () => {
  const { service } = await fixture({
    script: (request) => (request.method === 'status' ? new Error('socket closed') : undefined),
  });
  const stored = await service.upload(input(10), { kind: 'message', messageId }, () => {});
  const failure = await service
    .resume(stored.original, { filename: 'notes.bin', size: 10 }, () => {})
    .catch((e) => e);
  expect(failure.failure.kind).toBe('unknown');
  expect(failure.failure.original).toBe(stored.original);
});

it('a status state of unavailable keeps the original unknown; expired, discarded and not-observed are gone', async () => {
  let state = 'unavailable';
  const { service } = await fixture({
    script: (request) =>
      request.method === 'status' ? { ok: { result: 'state', state } } : undefined,
  });
  const stored = await service.upload(input(10), { kind: 'message', messageId }, () => {});
  const ask = () =>
    service
      .resume(stored.original, { filename: 'notes.bin', size: 10 }, () => {})
      .catch((e) => e.failure);
  expect(await ask()).toMatchObject({ kind: 'unknown', original: stored.original });
  for (state of ['expired', 'discarded', 'not-observed'])
    expect(await ask()).toEqual({ kind: 'gone' });
});

const sha256Hex = async (value: string) =>
  [...new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(value)))]
    .map((b) => b.toString(16).padStart(2, '0'))
    .join('');

it('a document upload is a content asset bound to the digest of the source being written', async () => {
  const { service } = await fixture();
  const stored = await service.upload(
    input(10),
    { kind: 'document', source: '<h1>a</h1>' },
    () => {},
  );
  const d = stored.original.descriptor;
  expect(d.namespace).toBe('content');
  expect(d.source).toEqual({ kind: 'document', sourceDigest: await sha256Hex('<h1>a</h1>') });
});

it('document publication refuses a file bound to a source other than the one being written', async () => {
  const { service } = await fixture();
  const stored = await service.upload(input(10), { kind: 'document', source: 'one' }, () => {});
  const failure = await service.publication([stored], 'two').catch((e) => e);
  expect(failure).toBeInstanceOf(AttachmentStaleError);
  expect(failure.attachmentIds).toEqual([stored.original.descriptor.attachmentId]);
});

// A connection upgraded without an origin has no object channel: Remote's setup missed its budget
// (#4297). The serve says so, and an attachment reconnects once for it.
const noChannel = { error: { code: 'no-object-channel' } };
const COOLDOWN = 1_000;

it('reopens once, no earlier than the cool-down, when the connection has no object channel', async () => {
  vi.useFakeTimers();
  try {
    const { service, reopened, replacement } = await fixture({
      script: () => noChannel,
      replacement: () => undefined,
    });
    let settled = false;
    const stored = service
      .upload(input(10), { kind: 'message', messageId }, () => {})
      .finally(() => (settled = true));
    await vi.advanceTimersByTimeAsync(COOLDOWN - 1);
    expect(reopened).toHaveLength(0);
    await vi.advanceTimersByTimeAsync(1);
    expect(reopened).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(20_000);
    await stored;
    // The replacement connection carried the whole upload, and nothing reopened twice.
    expect(replacement!.calls.map((call) => call.method)).toEqual([
      'config',
      'begin',
      'part',
      'commit',
    ]);
    expect(settled).toBe(true);
    expect(reopened).toHaveLength(1);
  } finally {
    vi.useRealTimers();
  }
});

it('a repeat of no object channel is a typed refusal after exactly one reopen', async () => {
  vi.useFakeTimers();
  try {
    const { service, reopened, replacement } = await fixture({
      script: () => noChannel,
      replacement: () => noChannel,
    });
    const failure = service
      .upload(input(10), { kind: 'message', messageId }, () => {})
      .catch((e) => e.failure);
    await vi.advanceTimersByTimeAsync(COOLDOWN * 5);
    expect(await failure).toEqual({ kind: 'refused', reason: 'no-object-channel' });
    expect(reopened).toHaveLength(1);
    expect(replacement!.calls).toHaveLength(1);
    // A later attempt is its own attempt: it may reopen once more.
    const again = service
      .upload(input(10), { kind: 'message', messageId }, () => {})
      .catch((e) => e.failure);
    await vi.advanceTimersByTimeAsync(COOLDOWN * 5);
    expect(await again).toEqual({ kind: 'refused', reason: 'no-object-channel' });
    expect(reopened).toHaveLength(2);
  } finally {
    vi.useRealTimers();
  }
});

it('without a way to reopen, no object channel is the same typed refusal and nothing waits', async () => {
  const { service } = await fixture({ script: () => noChannel });
  const failure = await service
    .upload(input(10), { kind: 'message', messageId }, () => {})
    .catch((e) => e.failure);
  expect(failure).toEqual({ kind: 'refused', reason: 'no-object-channel' });
});

it('only no object channel reopens: every other refusal stays what it was', async () => {
  for (const code of ['unavailable', 'denied', 'capacity', 'invalid']) {
    const { service, reopened } = await fixture({
      script: () => ({ error: { code, ...(code === 'capacity' ? { limit: 'x' } : {}) } }),
      replacement: () => undefined,
    });
    const failure = await service
      .upload(input(10), { kind: 'message', messageId }, () => {})
      .catch((e) => e.failure);
    expect(failure).toMatchObject({ kind: 'refused', reason: code });
    expect(reopened).toHaveLength(0);
  }
});

it('a status question on a connection with no object channel reopens once too', async () => {
  vi.useFakeTimers();
  try {
    const uploaded = await fixture();
    const stored = await uploaded.service.upload(
      input(10),
      { kind: 'message', messageId },
      () => {},
    );
    const { service, reopened, replacement } = await fixture({
      script: () => noChannel,
      replacement: (request) =>
        request.method === 'status'
          ? { ok: { result: 'pending', nextIndex: 0, received: 0 } }
          : undefined,
    });
    const resumed = service.resume(stored.original, { filename: 'notes.bin', size: 10 }, () => {});
    await vi.advanceTimersByTimeAsync(COOLDOWN);
    await resumed;
    expect(reopened).toHaveLength(1);
    expect(replacement!.calls[0]!.method).toBe('status');
  } finally {
    vi.useRealTimers();
  }
});
