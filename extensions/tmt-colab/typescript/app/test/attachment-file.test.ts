import { expect, it } from 'vite-plus/test';
import {
  attachmentFilename,
  previewType,
  rasterHeader,
  refuseFile,
  storedMediaType,
} from '../src/attachment-file.js';

const ascii = (s: string) => [...s].map((c) => c.charCodeAt(0));
const be32 = (n: number) => [(n >>> 24) & 255, (n >>> 16) & 255, (n >>> 8) & 255, n & 255];
function png(width: number, height: number) {
  return Uint8Array.from([
    0x89,
    0x50,
    0x4e,
    0x47,
    0x0d,
    0x0a,
    0x1a,
    0x0a,
    ...be32(13),
    ...ascii('IHDR'),
    ...be32(width),
    ...be32(height),
    8,
    6,
    0,
    0,
    0,
  ]);
}
function jpeg(width: number, height: number) {
  return Uint8Array.from([
    0xff,
    0xd8,
    0xff,
    0xe0,
    0x00,
    0x04,
    0x4a,
    0x46, // an APPn segment is skipped by its length
    0xff,
    0xc0,
    0x00,
    0x11,
    8,
    height >> 8,
    height & 255,
    width >> 8,
    width & 255,
    3,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
  ]);
}
function webpLossy(width: number, height: number) {
  return Uint8Array.from([
    ...ascii('RIFF'),
    0,
    0,
    0,
    0,
    ...ascii('WEBP'),
    ...ascii('VP8 '),
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0x9d,
    0x01,
    0x2a,
    width & 255,
    width >> 8,
    height & 255,
    height >> 8,
  ]);
}
function webpLossless(width: number, height: number) {
  const bits = (width - 1) | ((height - 1) << 14);
  return Uint8Array.from([
    ...ascii('RIFF'),
    0,
    0,
    0,
    0,
    ...ascii('WEBP'),
    ...ascii('VP8L'),
    0,
    0,
    0,
    0,
    0x2f,
    bits & 255,
    (bits >> 8) & 255,
    (bits >> 16) & 255,
    (bits >>> 24) & 255,
    0,
    0,
    0,
    0,
  ]);
}
function webpExtended(width: number, height: number) {
  const w = width - 1,
    h = height - 1;
  return Uint8Array.from([
    ...ascii('RIFF'),
    0,
    0,
    0,
    0,
    ...ascii('WEBP'),
    ...ascii('VP8X'),
    10,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    w & 255,
    (w >> 8) & 255,
    (w >> 16) & 255,
    h & 255,
    (h >> 8) & 255,
    (h >> 16) & 255,
  ]);
}

it('reads the declared size of PNG, JPEG and each WebP form from the header only', () => {
  expect(rasterHeader(png(640, 480))).toEqual({ type: 'image/png', width: 640, height: 480 });
  expect(rasterHeader(jpeg(300, 200))).toEqual({ type: 'image/jpeg', width: 300, height: 200 });
  expect(rasterHeader(webpLossy(321, 123))).toEqual({
    type: 'image/webp',
    width: 321,
    height: 123,
  });
  expect(rasterHeader(webpLossless(50, 70))).toEqual({ type: 'image/webp', width: 50, height: 70 });
  expect(rasterHeader(webpExtended(1024, 768))).toEqual({
    type: 'image/webp',
    width: 1024,
    height: 768,
  });
});

it('classifies by bytes, never by name, and treats every other file as an octet-stream', () => {
  expect(storedMediaType(png(10, 10))).toBe('image/png');
  expect(
    storedMediaType(new TextEncoder().encode('<svg xmlns="http://www.w3.org/2000/svg"/>')),
  ).toBe('application/octet-stream');
  // Video bytes, PDF and plain text are download-only; so is anything truncated or zero-sized.
  expect(storedMediaType(Uint8Array.from([0, 0, 0, 0x18, ...ascii('ftypmp42')]))).toBe(
    'application/octet-stream',
  );
  expect(storedMediaType(png(10, 10).slice(0, 20))).toBe('application/octet-stream');
  expect(storedMediaType(png(0, 10))).toBe('application/octet-stream');
});

it('bounds a preview at 16 million pixels without decoding', () => {
  expect(rasterHeader(png(4000, 4000))).toBeDefined();
  expect(rasterHeader(png(4001, 4000))).toBeUndefined();
  expect(storedMediaType(png(60000, 60000))).toBe('application/octet-stream');
});

it('previews only when the stored type and the verified bytes agree', () => {
  expect(previewType('image/png', png(8, 8))).toBe('image/png');
  expect(previewType('image/jpeg', png(8, 8))).toBeUndefined();
  expect(previewType('application/octet-stream', png(8, 8))).toBeUndefined();
});

it('refuses by size and count before reading and sanitizes labels', () => {
  expect(refuseFile(0, 0)).toBe('empty');
  expect(refuseFile(8 * 1024 * 1024 + 1, 0)).toBe('too-large');
  expect(refuseFile(8 * 1024 * 1024, 0)).toBeUndefined();
  expect(refuseFile(1, 16)).toBe('too-many');
  expect(attachmentFilename('a/b\\c\u0000d.txt')).toBe('a_b_c_d.txt');
  expect(attachmentFilename('   ')).toBe('attachment');
  expect(new TextEncoder().encode(attachmentFilename('é'.repeat(300))).length).toBeLessThanOrEqual(
    255,
  );
});
