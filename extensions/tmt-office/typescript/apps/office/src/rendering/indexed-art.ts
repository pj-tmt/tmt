/** Already-admitted raster values shared by prop and avatar renderers. */
import type { InsetText } from './inset-text.js';

export interface IndexedArt {
  pixels: readonly string[];
  palette: readonly string[];
  indexWidth?: 1 | 2;
  text?: InsetText;
}

/** Tint admitted palette slots while retaining their relative highlight values. */
export function tintPalette(palette: readonly string[], indices: readonly number[], tint: string) {
  const result = [...palette];
  const channels = tint.match(/[0-9a-f]{2}/g)!.map((value) => Number.parseInt(value, 16));
  for (const index of indices) {
    const source = palette[index]!.slice(1, 7)
      .match(/../g)!
      .map((value) => Number.parseInt(value, 16));
    const brightness = Math.max(...source) / 255;
    result[index] = `#${channels
      .map((value) =>
        Math.round(value * brightness)
          .toString(16)
          .padStart(2, '0')
      )
      .join('')}ff`;
  }
  return result;
}

export function* indexedCells(art: IndexedArt) {
  const digits = art.indexWidth ?? 1;
  for (const [y, row] of art.pixels.entries()) {
    for (let offset = 0; offset < row.length; offset += digits) {
      const index = Number.parseInt(row.slice(offset, offset + digits), 16);
      if (index !== 0) yield { x: offset / digits, y, color: art.palette[index]! };
    }
  }
}

/** Merge adjacent same-color cells without bridging transparent gaps. */
export function* indexedRuns(art: IndexedArt) {
  let run: { x: number; y: number; width: number; color: string } | undefined;
  for (const cell of indexedCells(art)) {
    if (run && cell.y === run.y && cell.x === run.x + run.width && cell.color === run.color) {
      run.width += 1;
    } else {
      if (run) yield run;
      run = { ...cell, width: 1 };
    }
  }
  if (run) yield run;
}
