import type { FloorSpan } from './map-contract.js';
import type { SceneRect } from '../rendering/office-geometry.js';

export interface FloorInterval {
  start: number;
  end: number;
}

export function subtractFloorInterval(
  parts: FloorInterval[],
  start: number,
  end: number,
  minWidth: number
): FloorInterval[] {
  return parts.flatMap((part) => {
    if (start >= part.end || end <= part.start) return [part];
    return [
      { start: part.start, end: Math.min(part.end, start) },
      { start: Math.max(part.start, end), end: part.end },
    ].filter((piece) => piece.end - piece.start >= minWidth);
  });
}

/** Sparse authoring/preview index. It suggests space, never authorizes a native Save. */
export function freeFloorRows(
  rows: readonly FloorSpan[],
  obstacles: readonly SceneRect[],
  minWidth: number
): Map<number, FloorInterval[]> {
  const result = new Map<number, FloorInterval[]>();
  for (const row of rows) {
    const parts = result.get(row.y) ?? [];
    parts.push({ start: row.start, end: row.end });
    result.set(row.y, parts);
  }
  for (const [y, parts] of result) {
    const merged: FloorInterval[] = [];
    for (const part of parts.sort((a, b) => a.start - b.start)) {
      const previous = merged.at(-1);
      if (previous && previous.end >= part.start) previous.end = Math.max(previous.end, part.end);
      else merged.push({ ...part });
    }
    let free = merged.filter((part) => part.end - part.start >= minWidth);
    for (const rect of obstacles) {
      if (y + 1 <= rect.y || y >= rect.y + rect.height) continue;
      free = subtractFloorInterval(
        free,
        Math.floor(rect.x),
        Math.ceil(rect.x + rect.width),
        minWidth
      );
    }
    result.set(y, free);
  }
  return result;
}

/** Horizontal starts that have continuous floor for every row of a rectangle. */
export function floorRectangleIntervals(
  rows: ReadonlyMap<number, FloorInterval[]>,
  y: number,
  height: number,
  width: number
): FloorInterval[] {
  let parts = rows.get(y) ?? [];
  for (let dy = 1; dy < height && parts.length; dy++) {
    const next = rows.get(y + dy) ?? [];
    const joined: FloorInterval[] = [];
    let a = 0,
      b = 0;
    while (a < parts.length && b < next.length) {
      const start = Math.max(parts[a]!.start, next[b]!.start);
      const end = Math.min(parts[a]!.end, next[b]!.end);
      if (end - start >= width) joined.push({ start, end });
      if (parts[a]!.end < next[b]!.end) a++;
      else b++;
    }
    parts = joined;
  }
  return parts.filter((part) => part.end - part.start >= width);
}
