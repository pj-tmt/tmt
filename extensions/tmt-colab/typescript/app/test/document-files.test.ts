import { expect, it, vi } from 'vite-plus/test';
import type { AttachmentService, StoredAttachment } from '../src/attachment-service.js';
import { LiveDocumentFiles } from '../src/document-files.js';
import type { OwnRecord } from '../src/fold-protocol.js';

const stored = (id: string) =>
  ({ original: { descriptor: { attachmentId: id } } }) as unknown as StoredAttachment;
const proof = [{ root: 'intents', key: 'k', value: {} }] as unknown as OwnRecord[];

function fixture(failPublish = false) {
  const order: string[] = [];
  const service = {
    publication: vi.fn(async () => {
      order.push('publication');
      return proof;
    }),
  } as unknown as AttachmentService;
  const edit = vi.fn(async () => {
    order.push('edit');
  });
  const files = new LiveDocumentFiles({
    service,
    epoch: () => '1',
    source: () => 'source',
    publish: async () => {
      order.push('publish');
      if (failPublish) throw new Error('refused');
    },
    edit,
  });
  return { files, order, edit, service };
}

it('proves the files first, then writes their descriptors against the same source', async () => {
  const { files, order, edit, service } = fixture();
  await files.add([stored('a'), stored('b')]);
  expect(order).toEqual(['publication', 'publish', 'edit']);
  expect(service.publication).toHaveBeenCalledWith([stored('a'), stored('b')], 'source');
  expect(edit).toHaveBeenCalledWith('source', 'source', {
    set: [{ attachmentId: 'a' }, { attachmentId: 'b' }],
  });
});

it('never writes the page when the proof is not published', async () => {
  const { files, edit } = fixture(true);
  await expect(files.add([stored('a')])).rejects.toThrow('refused');
  expect(edit).not.toHaveBeenCalled();
});

it('removes by identifier with a typed change and does nothing for an empty list', async () => {
  const { files, edit } = fixture();
  await files.remove([]);
  await files.add([]);
  expect(edit).not.toHaveBeenCalled();
  await files.remove(['a']);
  expect(edit).toHaveBeenCalledWith('source', 'source', { remove: ['a'] });
  expect(files.target()).toEqual({ kind: 'document', source: 'source' });
});
