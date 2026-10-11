import { afterEach, expect, it, vi } from 'vite-plus/test';
import type { attachment } from '@tmt/colab-client';
import type { AttachmentBinding } from '../src/attachment-service.js';
import { DocumentImages, IMAGE_BYTES, type DocumentImage } from '../src/document-images.js';

const id = '11111111-1111-4111-8111-111111111111';
const descriptor = (attachmentId = id, mediaType = 'image/png', plaintextBytes = '68') =>
  ({ attachmentId, mediaType, plaintextBytes }) as attachment.AttachmentDescriptor;
const PNG = Uint8Array.from(
  Buffer.from(
    'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==',
    'base64',
  ),
);
function binding(
  open: AttachmentBinding['open'] = vi.fn(async () => PNG.slice()),
  scope = () => 'admitted',
) {
  return { open, scope } as unknown as AttachmentBinding;
}
function display() {
  vi.stubGlobal(
    'FileReader',
    class {
      result = 'data:image/png;base64,verified';
      onload?: () => void;
      readAsDataURL() {
        this.onload?.();
      }
    },
  );
}
afterEach(() => vi.unstubAllGlobals());

it('admits only current document descriptors, resolving unique short and full aliases once', async () => {
  display();
  const open = vi.fn(async () => PNG.slice());
  const result: DocumentImage[] = [];
  const images = new DocumentImages(
    { binding: binding(open), descriptors: [descriptor()] },
    () => true,
  );
  await images.resolve(
    [`colab-attachment:11111111`, `colab-attachment:${id}`, 'colab-attachment:99999999'],
    (image) => result.push(image),
  );
  expect(open).toHaveBeenCalledExactlyOnceWith({ kind: 'document' }, descriptor());
  expect(result).toEqual([
    { reference: 'colab-attachment:11111111', url: 'data:image/png;base64,verified' },
    { reference: `colab-attachment:${id}`, url: 'data:image/png;base64,verified' },
    { reference: 'colab-attachment:99999999', reason: 'unavailable' },
  ]);
});
it('does not read ambiguous, non-raster or absent-backend references', async () => {
  const open = vi.fn();
  const descriptors = [
    descriptor(),
    descriptor('11111111-2222-4222-8222-222222222222'),
    descriptor('33333333-3333-4333-8333-333333333333', 'application/pdf'),
  ];
  const result: DocumentImage[] = [];
  await new DocumentImages({ binding: binding(open), descriptors }, () => true).resolve(
    ['colab-attachment:11111111', 'colab-attachment:33333333'],
    (image) => result.push(image),
  );
  expect(result.map((image) => 'reason' in image && image.reason)).toEqual([
    'unavailable',
    'unsupported',
  ]);
  await new DocumentImages({ descriptors: [] }, () => true).resolve(
    ['colab-attachment:11111111'],
    (image) => result.push(image),
  );
  expect(result.at(-1)).toEqual({ reference: 'colab-attachment:11111111', reason: 'not-here' });
  expect(open).not.toHaveBeenCalled();
});
it('charges unique descriptors in document order and refuses reads over the render budget', async () => {
  display();
  const descriptors = Array.from({ length: 9 }, (_, i) =>
    descriptor(
      `${i + 1}`.repeat(8) + '-1111-4111-8111-111111111111',
      'image/png',
      String(IMAGE_BYTES / 8),
    ),
  );
  const open = vi.fn(async () => PNG.slice());
  const result: DocumentImage[] = [];
  await new DocumentImages({ binding: binding(open), descriptors }, () => true).resolve(
    descriptors.map((d) => `colab-attachment:${d.attachmentId}`),
    (image) => result.push(image),
  );
  expect(open).toHaveBeenCalledTimes(8);
  for (let i = 0; i < 8; i++)
    expect(open).toHaveBeenNthCalledWith(i + 1, { kind: 'document' }, descriptors[i]);
  expect(result.at(-1)).toEqual({
    reference: `colab-attachment:${descriptors[8].attachmentId}`,
    reason: 'limit',
  });
});
it('clears late plaintext and suppresses disclosure after close or admission changes', async () => {
  display();
  for (const change of ['close', 'scope', 'standing']) {
    let finish!: (bytes: Uint8Array) => void;
    const open = vi.fn(
      () =>
        new Promise<Uint8Array>((resolve) => {
          finish = resolve;
        }),
    );
    let scope = 'admitted',
      standing = true;
    const images = new DocumentImages(
      { binding: binding(open, () => scope), descriptors: [descriptor()] },
      () => standing,
    );
    const publish = vi.fn();
    const pending = images.resolve([`colab-attachment:${id}`], publish);
    await Promise.resolve();
    if (change === 'close') images.close();
    else if (change === 'scope') scope = 'revoked';
    else standing = false;
    const bytes = PNG.slice();
    finish(bytes);
    await pending;
    expect(publish).not.toHaveBeenCalled();
    expect(bytes.every((byte) => byte === 0)).toBe(true);
  }
});
