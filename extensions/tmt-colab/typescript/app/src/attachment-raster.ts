import { previewType } from './attachment-file.js';

/** The caller owns admitted plaintext and clears it. Only a bounded, matching raster
 * reaches a browser decoder; the display copy never changes the descriptor's authority. */
export function rasterDataUrl(mediaType: string, bytes: Uint8Array): Promise<string> {
  const type = previewType(mediaType, bytes);
  if (!type) return Promise.reject(new Error('Not a bounded raster'));
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result as string);
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(new Blob([new Uint8Array(bytes)], { type }));
  });
}
