import { expect, it } from 'vite-plus/test';
import vectors from '../../../../../contracts/modules-central-grid-vectors.json';
import { officeWorldFixture } from '../../../../../../../typescript/test/support/office-world.js';
import { decodeModuleMap } from './module-contract.js';
import { platformModuleWorld } from './module-upgrade.js';
import { worldHistory } from './world-draft.js';
import type { WorldDocument } from './world-contract.js';

function source(): WorldDocument {
  const object = officeWorldFixture().layout.objects[0]!;
  return {
    version: 1,
    map: decodeModuleMap({ ...vectors.map, version: 6, modules: vectors.map.modules.slice(0, 5) }),
    objects: [
      object,
      {
        ...object,
        id: '30000000-0000-4000-8000-000000000099',
        placement: { ...object.placement, x: 8, y: 88 },
        surface: { type: 'wall', axis: 'horizontal', face: 'negative', elevation: 2 },
        extension: { definition: 'tmt-discussion-board', binding: { kind: 'office-board' } },
      },
    ],
  };
}

it('converts mounted content in an undoable draft without replacing art, IDs or resource bindings', () => {
  const original = source();
  const before = structuredClone(original);
  const converted = platformModuleWorld(original);
  expect(converted.objects[0]).toEqual(original.objects[0]);
  expect(converted.objects[1]).toEqual({
    ...original.objects[1],
    surface: { type: 'floor' },
    placement: { ...original.objects[1]!.placement, y: 86 },
  });
  expect(converted.map).toEqual(original.map);
  expect(original).toEqual(before);
  expect(platformModuleWorld(converted)).toBe(converted);
  const history = worldHistory.commit(worldHistory.create(original), converted);
  expect(worldHistory.current(worldHistory.undo(history))).toEqual(before);
  expect(worldHistory.current(worldHistory.redo(worldHistory.undo(history)))).toEqual(converted);
});

it('rejects an ownerless mounted object without losing any original content', () => {
  const world = source();
  world.objects[1]!.placement.x = -1000;
  const before = structuredClone(world);
  expect(() => platformModuleWorld(world)).toThrow('has no owning platform');
  expect(world).toEqual(before);
});

it('rejects an oversized mounted object rather than truncating its footprint', () => {
  const world = source();
  world.objects[1]!.placement.footprint = { width: 105, height: 2 };
  const before = structuredClone(world);
  expect(() => platformModuleWorld(world)).toThrow('does not fit its platform');
  expect(world).toEqual(before);
});
