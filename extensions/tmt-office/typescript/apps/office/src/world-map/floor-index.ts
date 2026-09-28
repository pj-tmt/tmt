import type { FloorSpan } from './map-contract.js';

/** Disposable row index shared by spatial labels and geometry; no per-tile store. */
export function indexFloor(spans: readonly FloorSpan[]) {
  const rows = new Map<number, FloorSpan[]>();
  let left = Infinity,
    top = Infinity,
    right = -Infinity,
    bottom = -Infinity;
  let tileCount = 0;
  for (const span of spans) {
    const row = rows.get(span.y) ?? [];
    row.push(span);
    rows.set(span.y, row);
    left = Math.min(left, span.start);
    right = Math.max(right, span.end);
    top = Math.min(top, span.y);
    bottom = Math.max(bottom, span.y + 1);
    tileCount += span.end - span.start;
  }
  for (const row of rows.values()) {
    row.sort((a, b) => a.start - b.start);
    for (let i = 1; i < row.length; i++)
      if (row[i - 1]!.end > row[i]!.start) throw new Error('Overlapping Office floor.');
  }
  return {
    bounds: tileCount ? { x: left, y: top, width: right - left, height: bottom - top } : null,
    tileCount,
    /** undefined = outside, null = common floor; callers supply tile coordinates. */
    areaAt(x: number, y: number): string | null | undefined {
      if (!Number.isInteger(x) || !Number.isInteger(y)) return undefined;
      const row = rows.get(y);
      if (!row) return undefined;
      let low = 0,
        high = row.length;
      while (low < high) {
        const mid = (low + high) >>> 1;
        if (row[mid]!.start <= x) low = mid + 1;
        else high = mid;
      }
      const span = row[low - 1];
      return span && x < span.end ? span.areaId : undefined;
    },
  };
}
