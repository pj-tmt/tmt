import type { IndexedArt } from '../rendering/indexed-art.js';
import { indexedCells } from '../rendering/indexed-art.js';

/** Shared display geometry in room tiles; not part of stored avatar artwork. */
export const AVATAR_LAYOUT = {
  width: 5.6,
  height: 8.4,
  markCenterY: 6,
} as const;

/** Transparent portrait padding must not become a gap above the character's name. */
export function avatarTopInset(art: IndexedArt): number {
  const first = indexedCells(art).next();
  return first.done ? 0 : (first.value.y / art.pixels.length) * AVATAR_LAYOUT.height;
}

/** Choose chest-label contrast from the actual art, including custom palettes. */
export function avatarMarkColor(art: IndexedArt) {
  const digits = art.indexWidth ?? 1;
  const row =
    art.pixels[Math.floor((AVATAR_LAYOUT.markCenterY / AVATAR_LAYOUT.height) * art.pixels.length)]!;
  const center = Math.floor(row.length / digits / 2) * digits;
  const color = art.palette[Number.parseInt(row.slice(center, center + digits), 16)]!;
  const [r, g, b] = color
    .slice(1, 7)
    .match(/../g)!
    .map((value) => Number.parseInt(value, 16));
  return 0.2126 * r! + 0.7152 * g! + 0.0722 * b! > 140 ? '#203e37' : '#fff8e4';
}
