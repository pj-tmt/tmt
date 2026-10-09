import { expect, it, vi } from 'vite-plus/test';
import { AttachmentDraft } from '../src/attachment-draft.js';
import {
  AttachmentUploadError,
  type AttachmentBinding,
  type StoredAttachment,
} from '../src/attachment-service.js';
import type { FrozenAttachmentUpload } from '../src/attachment-channel.js';

let counter = 0;
const original = () =>
  ({
    transferId: `t${++counter}`,
    descriptor: { attachmentId: `a${counter}` },
  }) as unknown as FrozenAttachmentUpload;
const file = (name: string, size = 10) => new File([new Uint8Array(size).fill(1)], name);

function binding(overrides: Partial<AttachmentBinding> = {}) {
  const uploads: { messageId: string; filename: string }[] = [];
  const value: AttachmentBinding = {
    limits: vi.fn(async () => ({ payloadBytes: 1 })),
    upload: vi.fn(async (input, messageId, progress) => {
      uploads.push({ messageId, filename: input.filename });
      progress(1, 2);
      progress(2, 2);
      return { original: original(), filename: input.filename, size: input.bytes.length };
    }),
    resume: vi.fn(async (o, input) => ({
      original: o,
      filename: input.filename,
      size: input.size,
    })),
    discard: vi.fn(async () => {}),
    open: vi.fn(async () => new Uint8Array()),
    ...overrides,
  };
  return { value, uploads };
}
const states = (draft: AttachmentDraft) => draft.getSnapshot().chips.map((c) => c.state.kind);

it('picking files only makes local chips: nothing is stored until Send prepares them', async () => {
  const { value } = binding();
  const draft = new AttachmentDraft(() => value);
  await draft.add([file('a.txt'), file('b.png')]);
  expect(states(draft)).toEqual(['ready', 'ready']);
  expect(value.upload).not.toHaveBeenCalled();
  expect(value.limits).not.toHaveBeenCalled();
});

it('refuses empty, oversized and surplus files as notices without reading or adding them', async () => {
  const { value } = binding();
  const draft = new AttachmentDraft(() => value);
  const big = new File([new Uint8Array(1)], 'big.bin');
  Object.defineProperty(big, 'size', { value: 8 * 1024 * 1024 + 1 });
  await draft.add([file('empty.txt', 0), big]);
  expect(draft.getSnapshot().chips).toHaveLength(0);
  expect(draft.getSnapshot().notices.map((n) => n.reason)).toEqual(['empty', 'too-large']);
  await draft.add(Array.from({ length: 17 }, (_, i) => file(`f${i}`)));
  expect(draft.getSnapshot().chips).toHaveLength(16);
  expect(draft.getSnapshot().notices.at(-1)!.reason).toBe('too-many');
});

it('prepare uploads in order under one message id and returns the stored originals', async () => {
  const { value, uploads } = binding();
  const draft = new AttachmentDraft(() => value);
  await draft.add([file('one'), file('two')]);
  const messageId = draft.getSnapshot().messageId;
  const result = await draft.prepare();
  expect(uploads).toEqual([
    { messageId, filename: 'one' },
    { messageId, filename: 'two' },
  ]);
  expect(result.ok && result.attach?.messageId).toBe(messageId);
  expect(result.ok && result.attach?.stored).toHaveLength(2);
  expect(states(draft)).toEqual(['stored', 'stored']);
});

it('a refused upload stops the send, keeps the other chips and never retries on its own', async () => {
  let n = 0;
  const { value } = binding({
    upload: vi.fn(async (input, _id, _p) => {
      if (++n === 2) throw new AttachmentUploadError({ kind: 'refused', reason: 'capacity' });
      return { original: original(), filename: input.filename, size: 1 };
    }),
  });
  const draft = new AttachmentDraft(() => value);
  await draft.add([file('one'), file('two'), file('three')]);
  expect(await draft.prepare()).toEqual({ ok: false, why: 'failed' });
  expect(states(draft)).toEqual(['stored', 'refused', 'ready']);
  expect(value.upload).toHaveBeenCalledTimes(2);
  // The refused chip blocks Send until the user chooses to try it again or removes it.
  expect(await draft.prepare()).toEqual({ ok: false, why: 'refused' });
  expect(value.upload).toHaveBeenCalledTimes(2);
  draft.retry(draft.getSnapshot().chips[1].id);
  expect((await draft.prepare()).ok).toBe(true);
});

it('an unknown upload blocks Send; check asks status of the same original and removal releases it', async () => {
  const lost = original();
  const { value } = binding({
    upload: vi.fn(async () => {
      throw new AttachmentUploadError({ kind: 'unknown', original: lost });
    }),
  });
  const draft = new AttachmentDraft(() => value);
  await draft.add([file('one')]);
  expect(await draft.prepare()).toEqual({ ok: false, why: 'failed' });
  expect(states(draft)).toEqual(['unknown']);
  expect(await draft.prepare()).toEqual({ ok: false, why: 'unknown' });
  expect(value.upload).toHaveBeenCalledTimes(1);
  await draft.check(draft.getSnapshot().chips[0].id);
  expect(value.resume).toHaveBeenCalledWith(
    lost,
    { filename: 'one', size: 10 },
    expect.any(Function),
  );
  expect(states(draft)).toEqual(['stored']);
});

it('a status answer that the upload is gone attaches again from the local bytes', async () => {
  const lost = original();
  let first = true;
  const { value } = binding({
    upload: vi.fn(async (input) => {
      if (first) {
        first = false;
        throw new AttachmentUploadError({ kind: 'unknown', original: lost });
      }
      return { original: original(), filename: input.filename, size: 10 };
    }),
    resume: vi.fn(async () => {
      throw new AttachmentUploadError({ kind: 'gone' });
    }),
  });
  const draft = new AttachmentDraft(() => value);
  await draft.add([file('one')]);
  await draft.prepare();
  await draft.check(draft.getSnapshot().chips[0].id);
  expect(states(draft)).toEqual(['again']);
  const messageId = draft.getSnapshot().messageId;
  expect((await draft.prepare()).ok).toBe(true);
  expect(value.upload).toHaveBeenCalledTimes(2);
  expect(draft.getSnapshot().messageId).toBe(messageId);
});

it('a stale page reuses the local bytes and the message id, and releases the old original', async () => {
  const { value } = binding();
  const draft = new AttachmentDraft(() => value);
  await draft.add([file('one')]);
  const first = await draft.prepare();
  const ids = first.ok
    ? first.attach!.stored.map((s: StoredAttachment) => s.original.descriptor.attachmentId)
    : [];
  draft.stale(ids);
  expect(states(draft)).toEqual(['again']);
  expect((await draft.prepare()).ok).toBe(true);
  expect(value.upload).toHaveBeenCalledTimes(2);
  expect(value.discard).toHaveBeenCalledTimes(1);
});

it('removing or leaving the composer releases what was stored; sending keeps it', async () => {
  const { value } = binding();
  const draft = new AttachmentDraft(() => value);
  await draft.add([file('one'), file('two')]);
  await draft.prepare();
  draft.remove(draft.getSnapshot().chips[0].id);
  expect(value.discard).toHaveBeenCalledTimes(1);
  draft.dispose();
  expect(value.discard).toHaveBeenCalledTimes(2);
  const sent = new AttachmentDraft(() => value);
  await sent.add([file('three')]);
  await sent.prepare();
  const before = sent.getSnapshot().messageId;
  sent.committed();
  sent.dispose();
  expect(value.discard).toHaveBeenCalledTimes(2);
  expect(sent.getSnapshot().messageId).not.toBe(before);
  expect(sent.getSnapshot().chips).toHaveLength(0);
});
