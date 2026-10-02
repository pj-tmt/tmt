import { expect, it } from 'vite-plus/test';
import {
  BUILTIN_CATALOG,
  MODULAR_WORKSTATION_DIGEST,
  MODULAR_LOUNGE_DIGEST,
  MODULAR_RECEPTION_DIGEST,
  MODULAR_FACILITIES_DIGEST,
  resolvePlacedProp,
} from './prop-contract.js';
import { directionalFurniture } from './furniture-upgrades.js';
import { canRotateObject, rotatedObject } from '../world-map/object-rotation.js';
import { officeWorldFixture } from '../../../../../../../typescript/test/support/office-world.js';

it.each([
  MODULAR_WORKSTATION_DIGEST,
  MODULAR_LOUNGE_DIGEST,
  MODULAR_RECEPTION_DIGEST,
  MODULAR_FACILITIES_DIGEST,
])('supplies four genuine views without rewriting the immutable %s pack', (digest) => {
  const pack = BUILTIN_CATALOG.find((p) => p.digest === digest)!.pack;
  for (const prop of pack.props) {
    const original = {
      prop: `${digest}/${prop.key}`,
      footprint: prop.footprint,
      x: 10,
      y: 20,
      rotation: 0,
    };
    const replacement = directionalFurniture(original, BUILTIN_CATALOG);
    if (prop.key === 'workstation-chair') {
      expect(replacement).toBe(original);
      continue;
    }
    expect(replacement).toEqual({ ...original, prop: replacement.prop });
    expect(replacement.prop).not.toBe(original.prop);
    const frames = resolvePlacedProp(BUILTIN_CATALOG, replacement)!.definition.frames!;
    expect(new Set(frames.map((frame) => JSON.stringify(frame))).size).toBe(4);
    for (const frame of frames) {
      expect(frame[0]).toMatch(/^0+$/);
      expect(frame.at(-1)).toMatch(/^0+$/);
      expect(frame.every((row) => row.startsWith('00') && row.endsWith('00'))).toBe(true);
    }
    expect(new Set(prop.frames!.map((frame) => JSON.stringify(frame))).size).toBe(1);
    expect(directionalFurniture(original, [{ digest, pack }])).toBe(original);
    expect(
      directionalFurniture({ ...original, footprint: { width: 1, height: 1 } }, BUILTIN_CATALOG)
        .prop
    ).toBe(original.prop);
  }
});

it('upgrades only a completed rotation, preserving identity, bindings and the old object', () => {
  const source = officeWorldFixture().layout.objects[0]!;
  const prop = BUILTIN_CATALOG.find((p) => p.digest === MODULAR_LOUNGE_DIGEST)!.pack.props[0]!;
  const object = {
    ...source,
    placement: {
      ...source.placement,
      prop: `${MODULAR_LOUNGE_DIGEST}/${prop.key}`,
      footprint: prop.footprint,
    },
  };
  const retained = structuredClone(object);
  expect(canRotateObject(object, BUILTIN_CATALOG)).toBe(true);
  expect(rotatedObject(object, 0, BUILTIN_CATALOG)).toBe(object);
  expect(rotatedObject(object, 4, BUILTIN_CATALOG)).toBe(object);
  const rotated = rotatedObject(object, 1, BUILTIN_CATALOG, 6);
  expect(rotated).toEqual({
    ...object,
    surface: { type: 'floor', base: { x: 1, y: 10, width: 14, height: 6 } },
    placement: {
      ...object.placement,
      x: object.placement.x + 5,
      y: object.placement.y + 5,
      prop: directionalFurniture(object.placement, BUILTIN_CATALOG).prop,
      rotation: (object.placement.rotation + 1) % 4,
    },
  });
  expect(object).toEqual(retained);
});
