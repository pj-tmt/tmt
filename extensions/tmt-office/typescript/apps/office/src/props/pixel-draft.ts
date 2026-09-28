import { snapshotHistory } from '../editor/snapshot-history.js';
import { exactRecord } from '../contracts/record.js';
import { indexedPalette, indexedRaster } from '../rendering/indexed-art-contract.js';
import { decodePropPack } from './prop-contract.js';
import type { PropPack } from './prop-contract.js';

export const PIXEL_SIZES = [16, 32] as const;
export type PixelSize = (typeof PIXEL_SIZES)[number];
export interface PixelDraft {
  size: PixelSize;
  pixels: string[];
  palette: string[];
  label: string;
  credit: string;
  license: string;
}
export interface PixelCell {
  x: number;
  y: number;
}

/** Blank/incomplete art is valid draft work; only pack admission permits persistence. */
function decodeDraft(value: PixelDraft): PixelDraft {
  const input = exactRecord(
    value,
    ['size', 'pixels', 'palette', 'label', 'credit', 'license'],
    'pixel draft'
  );
  if (!PIXEL_SIZES.includes(input.size as PixelSize) || !indexedPalette(input.palette, 16))
    throw new Error('Invalid pixel draft.');
  const size = input.size as PixelSize;
  const pixels = indexedRaster(input.pixels, input.palette.length, {
    width: size,
    height: size,
    maxSide: 32,
    maxCells: 1024,
    indexWidth: 2,
  });
  if (
    !pixels ||
    typeof input.label !== 'string' ||
    input.label.length > 80 ||
    typeof input.credit !== 'string' ||
    input.credit.length > 120 ||
    typeof input.license !== 'string' ||
    input.license.length > 64
  )
    throw new Error('Invalid pixel draft.');
  return {
    size,
    pixels: [...pixels],
    palette: [...input.palette],
    label: input.label,
    credit: input.credit,
    license: input.license,
  };
}

export const pixelHistory = snapshotHistory<PixelDraft>(decodeDraft);

export function newPixelDraft(size: PixelSize = 16): PixelDraft {
  return decodeDraft({
    size,
    pixels: Array.from({ length: size }, () => '00'.repeat(size)),
    palette: [
      '#00000000',
      '#f4e7c6ff',
      '#253e46ff',
      '#4fa8a0ff',
      '#d49355ff',
      '#dbe2e1ff',
      '#ab5d66ff',
      '#66825aff',
      '#b5a6c9ff',
      '#5a739bff',
      '#e7c764ff',
      '#7a5448ff',
      '#171e2dff',
      '#ffffffff',
      '#9cbfa7ff',
      '#d67447ff',
    ],
    label: 'Pixel artwork',
    credit: 'Office owner',
    license: 'LicenseRef-Private',
  });
}

export function paintPixels(
  draft: PixelDraft,
  cells: readonly PixelCell[],
  color: number
): PixelDraft {
  if (!Number.isInteger(color) || color < 0 || color >= draft.palette.length)
    throw new Error('Invalid palette index.');
  const pixels = [...draft.pixels];
  const encoded = color.toString(16).padStart(2, '0');
  for (const { x, y } of cells) {
    if (
      !Number.isInteger(x) ||
      !Number.isInteger(y) ||
      x < 0 ||
      y < 0 ||
      x >= draft.size ||
      y >= draft.size
    )
      throw new Error('Pixel is outside the canvas.');
    const row = pixels[y]!;
    pixels[y] = `${row.slice(0, x * 2)}${encoded}${row.slice(x * 2 + 2)}`;
  }
  return { ...draft, pixels };
}

/** Fill skipped pointer samples on the bounded cell grid, not a freehand vector store. */
export function pixelSegment(from: PixelCell, to: PixelCell): PixelCell[] {
  if (
    [from.x, from.y, to.x, to.y].some(
      (value) => !Number.isInteger(value) || value < 0 || value >= 32
    )
  )
    throw new Error('Pixel is outside the canvas.');
  const steps = Math.max(Math.abs(to.x - from.x), Math.abs(to.y - from.y));
  if (steps === 0) return [from];
  return Array.from({ length: steps + 1 }, (_, index) => ({
    x: Math.round(from.x + ((to.x - from.x) * index) / steps),
    y: Math.round(from.y + ((to.y - from.y) * index) / steps),
  }));
}

/** Flat artwork faces all four directions; the catalog still stores ordinary v2 packs. */
export function pixelDraftPack(draft: PixelDraft): PropPack {
  const checked = decodeDraft(draft);
  return decodePropPack({
    formatVersion: 2,
    label: checked.label,
    credit: checked.credit,
    license: checked.license,
    palette: checked.palette,
    props: [
      {
        key: 'artwork',
        label: checked.label,
        footprint: { width: checked.size / 8, height: checked.size / 8 },
        frames: Array.from({ length: 4 }, () => [...checked.pixels]),
      },
    ],
  });
}
