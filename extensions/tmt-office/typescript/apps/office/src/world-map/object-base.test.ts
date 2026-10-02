import { expect, it } from 'vite-plus/test';
import { officeWorldFixture } from '../../../../../../../typescript/test/support/office-world.js';
import { decodeFloorBase, floorObjectBounds } from './object-base.js';
import { decodeWorldDocument } from './world-contract.js';
import { worldObjectRect, worldObjectPosition } from '../rendering/world-geometry.js';

it('rotates physical support independently while retaining the entire artwork rectangle', () => {
  const world = officeWorldFixture().layout;
  const source = world.objects[0]!;
  const placement = { ...source.placement, x: -2, y: -2, footprint: { width: 8, height: 8 } };
  const base = { x: 2, y: 2, width: 4, height: 2 };
  const expected = [
    { x: 0, y: 0, width: 4, height: 2 },
    { x: 2, y: 0, width: 2, height: 4 },
    { x: 0, y: 2, width: 4, height: 2 },
    { x: 0, y: 0, width: 2, height: 4 },
  ];
  for (const rotation of [0, 1, 2, 3]) {
    const object = {
      ...source,
      placement: { ...placement, rotation },
      surface: { type: 'floor' as const, base },
    };
    expect(floorObjectBounds(object)).toEqual(expected[rotation]);
    const rect = worldObjectRect(object);
    expect({ width: rect.width, height: rect.height }).toEqual({ width: 8, height: 8 });
    expect(worldObjectPosition(object, rect)).toEqual({ x: -2, y: -2 });
    const decoded = decodeWorldDocument({ ...world, objects: [object] });
    expect(decoded.objects[0]).toEqual(object);
    expect(decoded.objects[0]!.surface).not.toBe(object.surface);
  }
  for (const invalid of [
    null,
    {},
    { ...base, x: -1 },
    { ...base, width: 7 },
    { ...base, height: 0 },
    { ...base, y: 0.5 },
    { ...base, extra: true },
  ]) {
    expect(() => decodeFloorBase(invalid, placement)).toThrow();
  }
});
