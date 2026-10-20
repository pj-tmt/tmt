import { expect, it, vi } from 'vite-plus/test';
import { AnswerFiles } from '../src/answer-files.js';
import type { FrozenAttachmentUpload } from '../src/attachment-channel.js';
import type { AttachmentBinding } from '../src/attachment-service.js';

let counter = 0;
const original = () =>
  ({
    transferId: `t${++counter}`,
    descriptor: { attachmentId: `a${counter}` },
  }) as unknown as FrozenAttachmentUpload;
const file = (name: string) => new File([new Uint8Array(10).fill(1)], name);

function binding(hold?: Promise<void>) {
  const value: AttachmentBinding = {
    limits: vi.fn(async () => ({ payloadBytes: 1 })),
    upload: vi.fn(async (input) => {
      await hold;
      return { original: original(), filename: input.filename, size: input.bytes.length };
    }),
    resume: vi.fn(),
    discard: vi.fn(async () => {}),
    open: vi.fn(async () => new Uint8Array()),
  } as unknown as AttachmentBinding;
  return value;
}

it('a question has one draft, made when first asked for, and its changes notify the group', async () => {
  const files = new AnswerFiles(() => binding());
  const notified = vi.fn();
  files.subscribe(notified);
  const draft = files.draft('q1');
  expect(files.draft('q1')).toBe(draft);
  expect(files.draft('q2')).not.toBe(draft);
  const before = files.getSnapshot();
  await draft.add([file('a.png')]);
  expect(files.getSnapshot()).toBeGreaterThan(before);
  expect(notified).toHaveBeenCalled();
});

it('a file in transfer holds back its own question alone', async () => {
  let release!: () => void;
  const files = new AnswerFiles(() => binding(new Promise<void>((resolve) => (release = resolve))));
  await files.draft('q1').add([file('a.png')]);
  await files.draft('q2').add([file('b.png')]);
  expect([files.uploading('q1'), files.uploading('q2'), files.uploading('q3')]).toEqual([
    false,
    false,
    false,
  ]);
  const prepared = files.draft('q1').prepare();
  expect([files.uploading('q1'), files.uploading('q2')]).toEqual([true, false]);
  release();
  expect((await prepared).ok).toBe(true);
  expect(files.uploading('q1')).toBe(false);
});

it('leaving the group releases what its drafts stored and stops notifying', async () => {
  const bound = binding();
  const files = new AnswerFiles(() => bound);
  const notified = vi.fn();
  files.subscribe(notified);
  const draft = files.draft('q1');
  await draft.add([file('kept.png')]);
  await draft.prepare();
  notified.mockClear();
  files.dispose();
  expect(bound.discard).toHaveBeenCalledTimes(1);
  await draft.add([file('late.png')]);
  expect(notified).not.toHaveBeenCalled();
  // A new draft after disposal is a new one.
  expect(files.draft('q1')).not.toBe(draft);
});
