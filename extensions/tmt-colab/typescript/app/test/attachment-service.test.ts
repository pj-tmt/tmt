import { expect, it } from 'vite-plus/test';
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
  options: { script?: Script; revision?: () => string; payloadBytes?: number } = {},
) {
  const calls: AttachmentObjectRequest[] = [];
  const signer = (await crypto.subtle.generateKey('Ed25519', false, [
    'sign',
    'verify',
  ])) as CryptoKeyPair;
  let received = 0;
  const channel = new AttachmentObjectChannel(async (request): Promise<AttachmentObjectOutcome> => {
    calls.push(structuredClone(request));
    const scripted = options.script?.(request, calls);
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
  });
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
  const service = new AttachmentService({
    connection: async () => connection,
    available: () => true,
    sharing: 'private',
  });
  return { service, calls, admission };
}
const input = (size = 100 * 1024) => ({
  filename: 'notes.bin',
  mediaType: 'application/octet-stream',
  bytes: Uint8Array.from({ length: size }, (_, i) => (i * 31 + 7) & 0xff),
});

it('uploads one frozen original in order and binds it to the preallocated message', async () => {
  const { service, calls } = await fixture();
  const progress: number[] = [];
  const stored = await service.upload(input(), messageId, (sent) => progress.push(sent));
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
  const failure = await service.upload(input(), messageId, () => {}).catch((e) => e);
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
  const failure = await service.upload(input(), messageId, () => {}).catch((e) => e);
  expect(failure.failure.kind).toBe('unknown');
});

it('a denied begin is a refusal that sent no part', async () => {
  const { service, calls } = await fixture({
    script: (request) => (request.method === 'begin' ? { error: { code: 'denied' } } : undefined),
  });
  const failure = await service.upload(input(), messageId, () => {}).catch((e) => e);
  expect(failure.failure).toEqual({ kind: 'refused', reason: 'denied' });
  expect(calls.some((c) => c.method === 'part')).toBe(false);
});

it('a status answer that the original is gone is reported as gone', async () => {
  const { service } = await fixture({
    script: (request) =>
      request.method === 'status' ? { ok: { result: 'state', state: 'expired' } } : undefined,
  });
  const stored = await service.upload(input(10), messageId, () => {});
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
  const a = await none.service.upload(input(10), messageId, () => {}).catch((e) => e);
  expect(a.failure).toEqual({ kind: 'refused', reason: 'unavailable' });
  expect(none.calls.map((c) => c.method)).toEqual(['config']);
  const small = await fixture({ payloadBytes: 1024 });
  const b = await small.service.upload(input(4096), messageId, () => {}).catch((e) => e);
  expect(b.failure).toEqual({ kind: 'refused', reason: 'too-large' });
  expect(small.calls.map((c) => c.method)).toEqual(['config']);
});

it('a message attachment is fenced by the membership head, not the page revision', async () => {
  let revision = `v1:${hex(64)}`;
  const fx = await fixture({ revision: () => revision });
  const { service, admission } = fx;
  const stored = await service.upload(input(10), messageId, () => {});
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
  const stored = await service.upload(input(10), messageId, () => {});
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
  const stored = await service.upload(input(10), messageId, () => {});
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
  const stored = await service.upload(input(10), messageId, () => {});
  const ask = () =>
    service
      .resume(stored.original, { filename: 'notes.bin', size: 10 }, () => {})
      .catch((e) => e.failure);
  expect(await ask()).toMatchObject({ kind: 'unknown', original: stored.original });
  for (state of ['expired', 'discarded', 'not-observed'])
    expect(await ask()).toEqual({ kind: 'gone' });
});
