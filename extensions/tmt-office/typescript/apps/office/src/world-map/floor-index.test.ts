import { expect, it } from 'vitest';
import { indexFloor } from './floor-index.js';
import type { FloorSpan } from './map-contract.js';

it('indexes shuffled negative rows, gaps and common floor without modifying source spans', () => {
  const spans: FloorSpan[] = [
    { y: -2, start: 3, end: 6, areaId: 'east' },
    { y: 1, start: -4, end: 0, areaId: null },
    { y: -2, start: -3, end: 0, areaId: 'west' },
    { y: -2, start: 0, end: 2, areaId: null },
  ];
  const before = structuredClone(spans);
  const index = indexFloor(spans);
  expect(index.bounds).toEqual({ x: -4, y: -2, width: 10, height: 4 });
  expect(index.tileCount).toBe(12);
  for (let y = -3; y <= 2; y++)
    for (let x = -5; x <= 6; x++) {
      const expected = spans.find(
        (span) => span.y === y && span.start <= x && x < span.end
      )?.areaId;
      expect(index.areaAt(x, y), `${x},${y}`).toBe(expected);
    }
  expect(index.areaAt(0.5, -2)).toBeUndefined();
  expect(index.areaAt(0, -1.5)).toBeUndefined();
  expect(spans).toEqual(before);
});

it('rejects ambiguous rows, admits touching spans and represents an empty draft', () => {
  expect(() =>
    indexFloor([
      { y: 0, start: 2, end: 5, areaId: null },
      { y: 0, start: 0, end: 3, areaId: null },
    ])
  ).toThrow('Overlapping Office floor.');
  const touching = indexFloor([
    { y: 0, start: 2, end: 5, areaId: 'right' },
    { y: 0, start: 0, end: 2, areaId: 'left' },
  ]);
  expect(touching.areaAt(1, 0)).toBe('left');
  expect(touching.areaAt(2, 0)).toBe('right');
  expect(indexFloor([])).toMatchObject({ tileCount: 0, bounds: null });
  expect(indexFloor([]).areaAt(0, 0)).toBeUndefined();
});

it('handles the full tile budget through sparse spans, including the final exclusive edge', () => {
  const index = indexFloor(
    Array.from({ length: 512 }, (_, y) => ({
      y,
      start: -256,
      end: 256,
      areaId: null,
    }))
  );
  expect(index.tileCount).toBe(262144);
  expect(index.areaAt(255, 511)).toBeNull();
  expect(index.areaAt(256, 511)).toBeUndefined();
  expect(index.areaAt(-256, 512)).toBeUndefined();
});
