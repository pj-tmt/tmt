import { describe, expect, it, vi } from 'vite-plus/test';
import { attachment, binary, encodeBinary, strictJson, text } from '@tmt/colab-client';
import corpus from '../../../contracts/vectors/attachment-v1.json';
import {
  AttachmentObjectChannel,
  FrozenAttachmentUpload,
  objectOutcome,
  readPolicy,
  uploadPolicy,
} from '../src/attachment-channel.js';

describe('frozen attachment object-channel bindings', () => {
  for (const answer of corpus.channelPolicies) {
    it(`${answer.name} uses the exact shared minimal policy and immutable ciphertext`, async () => {
      const value = corpus.cases.find((c) => c.name === `${answer.name}-asset`)!;
      const descriptor = attachment.attachmentDescriptor(strictJson(text(value.input), 2048));
      expect(await uploadPolicy(descriptor)).toEqual(text(answer.upload));
      expect(
        readPolicy(descriptor, answer.peerEpoch, attachment.attachmentSelector(answer.reference)),
      ).toEqual(text(answer.read));
      const bytes = binary(
        'payload' in value ? value.payload! : '',
        attachment.ATTACHMENT_PAYLOAD_BYTES,
      );
      const frozen = await FrozenAttachmentUpload.capture(
        descriptor,
        `v1:${'ab'.repeat(32)}`,
        '11111111-1111-4111-8111-111111111111',
        bytes,
      );
      const first = frozen.policy();
      first.fill(0);
      expect(frozen.policy()).toEqual(text(answer.upload));
      descriptor.filename = 'changed draft';
      const copy = frozen.descriptor;
      copy.objectId = 'ef'.repeat(32);
      expect(frozen.descriptor.filename).toBe('cat 🐈.bin');
      expect(frozen.descriptor.objectId).not.toBe(copy.objectId);
      const part = frozen.part(0);
      expect(part).toEqual(bytes);
      part.fill(0);
      expect(frozen.part(0)).toEqual(bytes);
      expect(() => frozen.part(1)).toThrow();
      expect(() => frozen.part(-1)).toThrow();
      expect(() => frozen.part(0.5)).toThrow();
      const bad = bytes.slice();
      bad[0] ^= 1;
      await expect(
        FrozenAttachmentUpload.capture(frozen.descriptor, frozen.base, frozen.transferId, bad),
      ).rejects.toThrow();
      bytes.fill(0);
      expect(frozen.part(0)).not.toEqual(bytes);
    });
  }
});

it('historical paging keeps one original deadline and cancels on consumer exit', async () => {
  const calls: { request: unknown; deadline: number }[] = [];
  const channel = new AttachmentObjectChannel(async (request, deadline) => {
    calls.push({ request: structuredClone(request), deadline });
    if (request.method === 'historycancel') return { ok: { result: 'history-end' } };
    return { ok: { result: 'history', frame: { page: 'ciphertext-only' }, more: true } };
  });
  const deadline = performance.now() + 15_000;
  const iterator = channel.historySource().frames('1', deadline)[Symbol.asyncIterator]();
  expect(await iterator.next()).toEqual({
    value: JSON.stringify({ page: 'ciphertext-only' }),
    done: false,
  });
  await iterator.next();
  await iterator.return?.();
  const first = calls[0].request as { historyId: string };
  expect(calls.map((call) => call.deadline)).toEqual([deadline, deadline, deadline]);
  expect(calls.map((call) => call.request)).toEqual([
    { method: 'history', epoch: '1', historyId: first.historyId },
    { method: 'historynext', historyId: first.historyId },
    { method: 'historycancel', historyId: first.historyId },
  ]);
});

it('a completed history releases naturally and a late history reply never yields data', async () => {
  const calls: string[] = [];
  const finished = new AttachmentObjectChannel(async (request) => {
    calls.push(request.method);
    return { ok: { result: 'history', frame: { more: false }, more: false } };
  });
  const values: string[] = [];
  for await (const frame of finished.historySource().frames('1', performance.now() + 15_000))
    values.push(frame);
  expect(values).toEqual([JSON.stringify({ more: false })]);
  expect(calls).toEqual(['history']);
  let now = 10;
  const clock = vi.spyOn(performance, 'now').mockImplementation(() => now);
  try {
    const late = new AttachmentObjectChannel(async () => {
      now = 21;
      return { ok: { result: 'history', frame: { secret: 'late' }, more: false } };
    });
    const iterator = late.historySource().frames('1', 20)[Symbol.asyncIterator]();
    await expect(iterator.next()).rejects.toThrow();
  } finally {
    clock.mockRestore();
  }
});

it('rejects method/result confusion, extra authority and noncanonical object replies', () => {
  for (const raw of [
    { ok: { result: 'read', offset: 0, totalBytes: 1, bytes: 'AA' } },
    { error: { code: 'unknown', retry: true } },
    { error: { code: 'capacity', limit: 'unbounded' } },
    { ok: { result: 'pending', nextIndex: 0, received: 0, committed: true } },
  ])
    expect(() => objectOutcome(raw, 'begin')).toThrow();
  expect(() => objectOutcome({ error: { code: 'unknown' } }, 'read')).toThrow();
  expect(() => objectOutcome({ error: { code: 'not-found' } }, 'begin')).toThrow();
  expect(objectOutcome({ error: { code: 'unknown' } }, 'commit')).toEqual({
    error: { code: 'unknown' },
  });
  expect(
    objectOutcome(
      { ok: { result: 'pending', nextIndex: 0, received: 0, expiresAtMs: 12 } },
      'status',
    ),
  ).toHaveProperty('ok');
});

it('status preserves the original and unknown never retries; committed reads check every range and digest', async () => {
  const value = corpus.cases.find((c) => c.name === `${corpus.channelPolicies[0].name}-asset`)!;
  const descriptor = attachment.attachmentDescriptor(strictJson(text(value.input), 2048));
  const raw = binary('payload' in value ? value.payload! : '', attachment.ATTACHMENT_PAYLOAD_BYTES);
  const original = await FrozenAttachmentUpload.capture(
    descriptor,
    `v1:${'ab'.repeat(32)}`,
    '11111111-1111-4111-8111-111111111111',
    raw,
  );
  const calls: unknown[] = [];
  const channel = new AttachmentObjectChannel(async (request) => {
    calls.push(structuredClone(request));
    if (request.method === 'verify')
      return {
        ok: {
          result: 'read',
          offset: request.offset,
          totalBytes: raw.length,
          bytes: encodeBinary(raw.slice(request.offset, request.offset + request.count)),
        },
      };
    return { error: { code: 'unknown' } };
  });
  const deadline = performance.now() + 15_000;
  expect(await channel.begin(original, deadline)).toEqual({ error: { code: 'unknown' } });
  expect(calls).toHaveLength(1);
  await channel.status(original, deadline);
  expect(calls[0]).toEqual({
    method: 'begin',
    transferId: original.transferId,
    descriptor: original.descriptor,
    base: original.base,
  });
  expect(calls[1]).toEqual({
    method: 'status',
    transferId: original.transferId,
    descriptor: original.descriptor,
    base: original.base,
  });
  const namespace = await (
    await import('../src/attachments.js')
  ).attachmentNamespace(descriptor.space, descriptor.page);
  const key = Uint8Array.from(descriptor.objectId.match(/../g)!, (byte) => parseInt(byte, 16));
  expect(await channel.verifier(original).readCommitted(namespace, key, deadline)).toEqual(raw);
  for (const corrupt of [
    { result: 'read', offset: 1, totalBytes: raw.length, bytes: encodeBinary(raw) },
    { result: 'read', offset: 0, totalBytes: raw.length + 1, bytes: encodeBinary(raw) },
    {
      result: 'read',
      offset: 0,
      totalBytes: raw.length,
      bytes: encodeBinary(new Uint8Array(raw.length)),
    },
  ]) {
    const bad = new AttachmentObjectChannel(async () => ({ ok: corrupt }));
    await expect(bad.verifier(original).readCommitted(namespace, key, deadline)).rejects.toThrow();
  }
  await expect(
    channel.verifier(original).readCommitted(new Uint8Array(32), key, deadline),
  ).rejects.toThrow();
});

it('a wire error on a read becomes the reason the person can act on, never a generic failure', async () => {
  const value = corpus.cases.find((c) => c.name === `${corpus.channelPolicies[0].name}-asset`)!;
  const descriptor = attachment.attachmentDescriptor(strictJson(text(value.input), 2048));
  const { attachmentNamespace } = await import('../src/attachments.js');
  const namespace = await attachmentNamespace(descriptor.space, descriptor.page);
  const key = Uint8Array.from(descriptor.objectId.match(/../g)!, (byte) => parseInt(byte, 16));
  const selector = attachment.attachmentSelector(corpus.channelPolicies[0].reference);
  const deadline = performance.now() + 15_000;
  for (const [code, reason] of [
    ['denied', 'denied'],
    ['not-found', 'not-found'],
    ['conflict', 'changed'],
    ['unavailable', 'unavailable'],
    ['capacity', 'unavailable'],
  ] as const) {
    const channel = new AttachmentObjectChannel(async () => ({ error: { code } }));
    await expect(
      channel.read(selector, descriptor).readCommitted(namespace, key, deadline),
    ).rejects.toMatchObject({ reason });
  }
});
