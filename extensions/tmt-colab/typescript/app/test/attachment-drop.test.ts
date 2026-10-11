import { expect, it, vi } from 'vite-plus/test';
import { attachment } from '@tmt/colab-client';
import { AttachmentDropOwner } from '../src/attachment-drop.js';
import { AttachmentDraft } from '../src/attachment-draft.js';

it('routes once to the replacement composer and ignores stale unregisters', () => {
  const owner = new AttachmentDropOwner();
  const first = new AttachmentDraft(() => undefined);
  const second = new AttachmentDraft(() => undefined);
  const firstAdd = vi.spyOn(first, 'add').mockResolvedValue(undefined);
  const secondAdd = vi.spyOn(second, 'add').mockResolvedValue(undefined);
  const stale = owner.register({ draft: first, attachable: true });
  const current = owner.register({ draft: second, attachable: true });
  stale();
  const files = [new File(['local'], 'notes.txt')];
  expect(owner.add(files)).toBeUndefined();
  expect(firstAdd).not.toHaveBeenCalled();
  expect(secondAdd).toHaveBeenCalledExactlyOnceWith(files);
  current();
  expect(owner.add(files)).toBe('Attachments are unavailable here.');
  expect(secondAdd).toHaveBeenCalledOnce();
});

it('rejects an entire oversized or over-count batch without adding any file', () => {
  const owner = new AttachmentDropOwner();
  const draft = new AttachmentDraft(() => undefined);
  const add = vi.spyOn(draft, 'add').mockResolvedValue(undefined);
  owner.register({ draft, attachable: true });
  const large = new File(['local'], 'large.txt');
  Object.defineProperty(large, 'size', { value: attachment.ATTACHMENT_PLAINTEXT_BYTES + 1 });
  expect(owner.add([new File(['ok'], 'ok.txt'), large])).toBe('This file is too large.');
  expect(owner.refusal([], attachment.MESSAGE_ATTACHMENTS + 1)).toBe('Too many files.');
  expect(owner.add([new File([], 'empty.txt')])).toBe('Empty files cannot be attached.');
  expect(owner.add([])).toBe('Only files can be attached.');
  expect(add).not.toHaveBeenCalled();
});

it('keeps blocked and unmounted panels inert without preparing or uploading', () => {
  const owner = new AttachmentDropOwner();
  const draft = new AttachmentDraft(() => {
    throw new Error('Remote must not be read');
  });
  const add = vi.spyOn(draft, 'add');
  const remove = owner.register({ draft, attachable: false });
  expect(owner.attachable).toBe(false);
  expect(owner.add([new File(['local'], 'notes.txt')])).toBe('Attachments are unavailable here.');
  remove();
  expect(owner.attachable).toBe(false);
  expect(add).not.toHaveBeenCalled();
});
