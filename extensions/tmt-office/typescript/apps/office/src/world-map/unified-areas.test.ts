import { expect, it } from 'vitest';
import vectors from '../../../../../../../contracts/office/modules-unified-vectors.json';
import old from '../../../../../../../contracts/office/modules-island-vectors.json';
import { decodeModuleMap } from './module-contract.js';
import { mapGeometry, decodeMapSource } from './map-source.js';
import { moduleBounds } from './module-geometry.js';
import { nextMeetingSlot } from './meeting-module.js';
import { officeExpansionSlots } from './module-authoring.js';
import { unifiedAreaWorld } from './module-upgrade.js';
import { officeWorldFixture } from '../../../../../../../typescript/test/support/office-world.js';
import { createWorldYjs } from './world-yjs.js';
import type { WorldDocument } from './world-contract.js';

it('uses the shared native vectors with mixed uses and no dedicated meeting wing', () => {
  const source = decodeModuleMap(vectors.map);
  const map = mapGeometry(source);
  const at = (x: number, y: number) =>
    map.floor.find((row) => row.y === y && row.start <= x && x < row.end);
  for (const [x, y] of vectors.publicSamples) expect(at(x!, y!)?.areaId).toBe(null);
  for (const [x, y] of vectors.emptySamples) expect(at(x!, y!)).toBeUndefined();
  expect(map.floor.reduce((sum, row) => sum + row.end - row.start, 0)).toBe(vectors.floorTiles);
  expect(map.doors).toHaveLength(vectors.doorEdges);
  expect(
    source.modules
      .filter(({ area }) => area.binding.type === 'meeting')
      .map((module) => moduleBounds(module, 8))
  ).toEqual(vectors.meetingBounds);
  expect(nextMeetingSlot(source)).toBeUndefined();
  expect(officeExpansionSlots(source)).toContainEqual({ type: 'office', column: 2, row: 0 });
  const switched = decodeModuleMap({
    ...source,
    modules: source.modules.map((module) => ({
      ...module,
      area:
        module.area.binding.type === 'meeting'
          ? { ...module.area, binding: { type: 'personal', identityId: null } }
          : module.area,
    })),
  });
  expect(mapGeometry(switched).floor).toEqual(map.floor);
  expect(mapGeometry(switched).doors).toEqual(map.doors);
});

it('rejects legacy meeting slots and overlapping grid areas instead of reinterpreting them', () => {
  expect(() => decodeMapSource({ ...old.map, version: 8 })).toThrow('purpose');
  expect(() =>
    decodeMapSource({
      ...vectors.map,
      modules: vectors.map.modules.map((module, index) =>
        index === 3 ? { ...module, slot: { type: 'office', column: 0, row: -1 } } : module
      ),
    })
  ).toThrow('overlap');
});

it('explicitly aligns legacy meetings with physical bases and exact Yjs Undo/Redo', () => {
  const object = officeWorldFixture().layout.objects[0]!;
  const original: WorldDocument = {
    version: 1,
    map: decodeModuleMap(old.map),
    objects: [
      {
        ...object,
        placement: { ...object.placement, x: 140, y: -1 },
        surface: { type: 'floor', base: { x: 0, y: 1, width: 4, height: 1 } },
        extension: { definition: 'tmt-discussion-board', binding: { kind: 'office-board' } },
      },
    ],
  };
  const before = structuredClone(original);
  const next = unifiedAreaWorld(original);
  expect(next.objects).toEqual([
    { ...original.objects[0], placement: { ...original.objects[0]!.placement, x: 116 } },
  ]);
  expect(next.map.version).toBe(8);
  if (next.map.version === 1) throw new Error('Expected modular areas');
  expect(
    next.map.modules.filter(({ area }) => area.binding.type === 'meeting').map(({ slot }) => slot)
  ).toEqual([
    { type: 'office', column: 2, row: 0 },
    { type: 'office', column: 2, row: 2 },
  ]);
  const history = createWorldYjs(original);
  try {
    history.change(next);
    history.undo();
    expect(history.world).toEqual(before);
    history.redo();
    expect(history.world).toEqual(next);
  } finally {
    history.destroy();
  }
  expect(original).toEqual(before);
  expect(unifiedAreaWorld(next)).toBe(next);
});

it('does not discard bridge placements during alignment', () => {
  const object = officeWorldFixture().layout.objects[0]!;
  const original: WorldDocument = {
    version: 1,
    map: decodeModuleMap({ ...old.map, version: 6 }),
    objects: [
      {
        ...object,
        placement: { ...object.placement, x: 120, y: 16, footprint: { width: 1, height: 1 } },
      },
    ],
  };
  const before = structuredClone(original);
  expect(() => unifiedAreaWorld(original)).toThrow('fully inside');
  expect(original).toEqual(before);
});
