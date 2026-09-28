import { expect, it } from 'vitest';
import { officeWorldFixture } from '../../../../../../../typescript/test/support/office-world.js';
import { DIRECTIONAL_WORKSTATION_DIGEST } from '../props/prop-contract.js';
import { withFurnitureBase } from './furniture-base.js';
import { rotatedObject } from './object-rotation.js';
import { worldObjectRect } from '../rendering/world-geometry.js';
import { platformProjection } from '../rendering/platform-projection.js';
import { createWorldProjection } from '../rendering/world-projection.js';

it('adopts a shallow bookcase base only on an explicit edit, preserving artwork and custom support', () => {
  const source = officeWorldFixture().layout.objects[0]!;
  const object = {
    ...source,
    surface: { type: 'floor' as const },
    placement: {
      ...source.placement,
      prop: `${DIRECTIONAL_WORKSTATION_DIGEST}/workstation-bookcase`,
      footprint: { width: 11, height: 11 },
    },
  };
  const original = structuredClone(object);
  const next = withFurnitureBase(object, 6);
  expect(next).toEqual({
    ...object,
    surface: { type: 'floor', base: { x: 2, y: 8, width: 7, height: 3 } },
  });
  expect(object).toEqual(original);
  expect(next.placement).toEqual(object.placement);
  const projection = platformProjection([], 7 / 8);
  expect(worldObjectRect(next, projection)).toEqual(worldObjectRect(object, projection));
  let turned = next;
  for (let i = 0; i < 4; i++) turned = rotatedObject(turned, 1);
  expect(turned).toEqual(next);
  expect(withFurnitureBase(next, 6)).toBe(next);
  const custom = {
    ...next,
    surface: { type: 'floor' as const, base: { x: 1, y: 1, width: 9, height: 9 } },
  };
  expect(withFurnitureBase(custom, 6)).toBe(custom);
  const unknown = { ...object, placement: { ...object.placement, prop: source.placement.prop } };
  expect(withFurnitureBase(unknown, 6)).toBe(unknown);
  // Earlier central-room projections use a different upright anchor. Editing
  // those maps must not silently adopt platform recipes or move their artwork.
  expect(withFurnitureBase(object, 5)).toBe(object);
  expect(rotatedObject(object, 1, undefined, 5).surface).toEqual(object.surface);
  expect(rotatedObject(object, 1, undefined, 6).surface).toEqual(next.surface);
  expect(rotatedObject(next, 1, undefined, 5).surface).toEqual(next.surface);
  const oldProjection = createWorldProjection(5, {
    bounds: new Map(),
    areaAt: () => undefined,
  });
  expect(worldObjectRect(withFurnitureBase(object, 5), oldProjection)).toEqual(
    worldObjectRect(object, oldProjection)
  );
  expect(worldObjectRect(rotatedObject(object, 1, undefined, 5), oldProjection)).toEqual(
    worldObjectRect(object, oldProjection)
  );
});
