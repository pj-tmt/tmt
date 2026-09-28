import { expect, it } from 'vitest';
import { compactCirculation } from './compact-circulation.js';

const lobby = { x: 0, y: 0, width: 104, height: 88 };
const office = (x: number, y: number) => ({ x, y, width: 48, height: 40 });
const contains = (rooms: ReturnType<typeof compactCirculation>, x: number, y: number) =>
  rooms.some(
    (rect) => x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height
  );

it('does not build speculative corridors for an empty office', () => {
  expect(compactCirculation([lobby])).toEqual([]);
});

it('reaches a distant office without filling branches on intervening empty rows', () => {
  const paths = compactCirculation([lobby, office(-112, -144)]);
  expect(paths).toEqual([
    { x: -92, y: -104, width: 148, height: 8 },
    { x: 48, y: -104, width: 8, height: 152 },
  ]);
  expect(contains(paths, -88, -104)).toBe(true); // Actual entrance.
  expect(contains(paths, 50, -50)).toBe(true); // Necessary trunk.
  expect(contains(paths, -88, -56)).toBe(false); // No unused horizontal branch.
  expect(contains(paths, -110, -104)).toBe(false); // No dead-end beyond the entrance.
});

it('removal drops only the unused branch while preserving the route to remaining offices', () => {
  const a = office(-56, -48),
    b = office(56, 96);
  const before = compactCirculation([lobby, a, b]);
  const after = compactCirculation([lobby, b]);
  expect(contains(before, -30, -4)).toBe(true);
  expect(contains(after, -30, -4)).toBe(false);
  expect(contains(after, 78, 90)).toBe(true);
  expect(contains(after, 50, 90)).toBe(true);
});

it('uses the Lobby centerline for lateral offices without requiring an opposing room', () => {
  const paths = compactCirculation([lobby, office(-56, 0)]);
  expect(contains(paths, -30, 42)).toBe(true);
  expect(contains(paths, -30, -4)).toBe(false);
});
