import { expect, it, vi } from 'vite-plus/test';
import { rasterDataUrl } from '../src/attachment-raster.js';
const PNG = Uint8Array.from(
  Buffer.from(
    'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==',
    'base64',
  ),
);
it('checks type, header and pixel bound before making a browser display copy', async () => {
  const read = vi.fn();
  vi.stubGlobal(
    'FileReader',
    class {
      readAsDataURL = read;
    },
  );
  try {
    for (const [type, bytes] of [
      ['image/svg+xml', PNG],
      ['image/jpeg', PNG],
      ['image/png', new TextEncoder().encode('<svg/>')],
    ] as const)
      await expect(rasterDataUrl(type, bytes)).rejects.toThrow('bounded raster');
    const oversized = PNG.slice();
    new DataView(oversized.buffer).setUint32(16, 16000001);
    await expect(rasterDataUrl('image/png', oversized)).rejects.toThrow('bounded raster');
    expect(read).not.toHaveBeenCalled();
  } finally {
    vi.unstubAllGlobals();
  }
});
it('makes a matching raster URL without changing the caller-owned plaintext', async () => {
  let copy: Blob | undefined;
  vi.stubGlobal(
    'FileReader',
    class {
      result = 'data:image/png;base64,verified';
      onload?: () => void;
      readAsDataURL(blob: Blob) {
        copy = blob;
        this.onload?.();
      }
    },
  );
  try {
    const before = PNG.slice();
    expect(await rasterDataUrl('image/png', PNG)).toBe('data:image/png;base64,verified');
    expect(copy!.type).toBe('image/png');
    expect(new Uint8Array(await copy!.arrayBuffer())).toEqual(before);
    expect(PNG).toEqual(before);
  } finally {
    vi.unstubAllGlobals();
  }
});
