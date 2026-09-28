import { expect, it } from 'vitest';
import { freeFloorRows, floorRectangleIntervals } from './free-floor.js';

it('merges adjacent segments, subtracts obstacles and intersects every required row without mutating input', () => {
  const rows = [
    { y: -1, start: -6, end: -2, areaId: null },
    { y: -1, start: -2, end: 6, areaId: null },
    { y: 0, start: -4, end: 6, areaId: null },
    { y: 1, start: -6, end: 6, areaId: null },
  ];
  const before = structuredClone(rows);
  const free = freeFloorRows(rows, [{ x: 1.5, y: 0, width: 1, height: 1 }], 3);
  expect(floorRectangleIntervals(free, -1, 3, 3)).toEqual([
    { start: -4, end: 1 },
    { start: 3, end: 6 },
  ]);
  expect(floorRectangleIntervals(free, -1, 4, 3)).toEqual([]);
  expect(rows).toEqual(before);
});
