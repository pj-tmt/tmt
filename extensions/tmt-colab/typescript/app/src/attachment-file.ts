import { attachment, text } from '@tmt/colab-client';

/** Raster previews are bounded by the header-declared pixel count, before any decode. */
export const PREVIEW_PIXELS = 16_000_000;
/** The only type a generic file is stored and downloaded as. */
export const GENERIC_MEDIA_TYPE = 'application/octet-stream';
export const RASTER_TYPES = ['image/png', 'image/jpeg', 'image/webp'] as const;
export type RasterType = (typeof RASTER_TYPES)[number];

export type FileRefusal = 'empty' | 'too-large' | 'too-many';

/** Size and count are checked before any byte is read from the picked file. */
export function refuseFile(size: number, existing: number): FileRefusal | undefined {
  if (existing >= attachment.MESSAGE_ATTACHMENTS) return 'too-many';
  if (size === 0) return 'empty';
  if (size > attachment.ATTACHMENT_PLAINTEXT_BYTES) return 'too-large';
  return undefined;
}

/** A display and download label only; it never selects a type or a path. Control
 * characters become `_` and the result fits the descriptor's 255 byte bound. */
export function attachmentFilename(raw: string): string {
  let out = '';
  for (const char of raw.replace(/[\p{Cc}/\\]/gu, '_')) {
    if (text(out + char).length > 255) break;
    out += char;
  }
  return out.trim() === '' ? 'attachment' : out;
}

const u16 = (b: Uint8Array, i: number) => (b[i] << 8) | b[i + 1];
const u32 = (b: Uint8Array, i: number) =>
  ((b[i] << 24) | (b[i + 1] << 16) | (b[i + 2] << 8) | b[i + 3]) >>> 0;
const le16 = (b: Uint8Array, i: number) => b[i] | (b[i + 1] << 8);
const tag = (b: Uint8Array, i: number, value: string) =>
  b.length >= i + value.length && [...value].every((char, k) => b[i + k] === char.charCodeAt(0));

export interface Raster {
  type: RasterType;
  width: number;
  height: number;
}

function png(b: Uint8Array): Raster | undefined {
  const signature = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
  if (b.length < 24 || !signature.every((v, i) => b[i] === v) || !tag(b, 12, 'IHDR'))
    return undefined;
  return { type: 'image/png', width: u32(b, 16), height: u32(b, 20) };
}

function jpeg(b: Uint8Array): Raster | undefined {
  if (b.length < 4 || b[0] !== 0xff || b[1] !== 0xd8) return undefined;
  let i = 2;
  while (i + 4 <= b.length) {
    if (b[i] !== 0xff) return undefined;
    const marker = b[i + 1];
    if (marker === 0xff) {
      i += 1;
      continue;
    }
    if (marker === 0x01 || (marker >= 0xd0 && marker <= 0xd8)) {
      i += 2;
      continue;
    }
    if (marker === 0xd9 || marker === 0xda) return undefined;
    const length = u16(b, i + 2);
    if (marker >= 0xc0 && marker <= 0xcf && ![0xc4, 0xc8, 0xcc].includes(marker)) {
      if (length < 8 || i + 9 > b.length) return undefined;
      return { type: 'image/jpeg', height: u16(b, i + 5), width: u16(b, i + 7) };
    }
    if (length < 2) return undefined;
    i += 2 + length;
  }
  return undefined;
}

function webp(b: Uint8Array): Raster | undefined {
  if (b.length < 25 || !tag(b, 0, 'RIFF') || !tag(b, 8, 'WEBP')) return undefined;
  if (tag(b, 12, 'VP8 ')) {
    if (b.length < 30 || b[23] !== 0x9d || b[24] !== 0x01 || b[25] !== 0x2a) return undefined;
    return { type: 'image/webp', width: le16(b, 26) & 0x3fff, height: le16(b, 28) & 0x3fff };
  }
  if (tag(b, 12, 'VP8L')) {
    if (b[20] !== 0x2f) return undefined;
    const bits = (b[21] | (b[22] << 8) | (b[23] << 16) | (b[24] << 24)) >>> 0;
    return { type: 'image/webp', width: (bits & 0x3fff) + 1, height: ((bits >>> 14) & 0x3fff) + 1 };
  }
  if (tag(b, 12, 'VP8X') && b.length >= 30)
    return {
      type: 'image/webp',
      width: (b[24] | (b[25] << 8) | (b[26] << 16)) + 1,
      height: (b[27] | (b[28] << 8) | (b[29] << 16)) + 1,
    };
  return undefined;
}

/** The parsed header of a bounded PNG/JPEG/WebP, or nothing for any other bytes. */
export function rasterHeader(bytes: Uint8Array): Raster | undefined {
  const found = png(bytes) ?? jpeg(bytes) ?? webp(bytes);
  return found &&
    found.width > 0 &&
    found.height > 0 &&
    found.width * found.height <= PREVIEW_PIXELS
    ? found
    : undefined;
}

/** What the sealed descriptor records: a verified raster type, else the generic type.
 * A filename or the browser's declared type never decides this. */
export function storedMediaType(bytes: Uint8Array): string {
  return rasterHeader(bytes)?.type ?? GENERIC_MEDIA_TYPE;
}

/** Preview is offered only when the descriptor and the verified bytes agree. */
export function previewType(mediaType: string, bytes: Uint8Array): RasterType | undefined {
  const raster = rasterHeader(bytes);
  return raster && raster.type === mediaType ? raster.type : undefined;
}
