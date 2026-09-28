import { expect, it } from 'vitest';
import vectors from '../../../../../contracts/modules-v2-vectors.json';
import { officeWorldFixture } from '../../../../../../../typescript/test/support/office-world.js';
import { decodeModuleMap } from './module-contract.js';
import { upgradeModuleWorld } from './module-upgrade.js';
import { worldHistory } from './world-draft.js';
import type { WorldDocument } from './world-contract.js';
import { mapGeometry } from './map-source.js';

function source(version: 2 | 3): WorldDocument {
  const fixture = officeWorldFixture().layout;
  const object = fixture.objects[0]!;
  return {
    ...fixture,
    map: decodeModuleMap({ ...vectors.starter, version }),
    objects: [
      {
        ...object,
        placement: { ...object.placement, x: 8, y: 52 },
        extension: { definition: 'tmt-discussion-board', binding: { kind: 'office-board' } },
      },
      {
        ...object,
        id: '30000000-0000-4000-8000-000000000099',
        surface: { type: 'wall', axis: 'horizontal', face: 'negative', elevation: 2 },
        placement: { ...object.placement, x: 8, y: 40 },
      },
    ],
  };
}

for (const version of [2, 3] as const)
  it(`upgrades v${version} in one reversible draft without replacing identities or attachments`, () => {
    const original = source(version);
    const before = structuredClone(original);
    const converted = upgradeModuleWorld(original);
    expect(converted.map.version).toBe(4);
    if (converted.map.version === 1 || original.map.version === 1)
      throw new Error('Expected modules');
    expect(converted.map.modules.map((module) => module.area)).toEqual(
      original.map.modules.map((module) => module.area)
    );
    expect(converted.map.modules.map((module) => module.slot)).toEqual([
      { type: 'lobby' },
      { type: 'office', column: 0, row: -1 },
      { type: 'office', column: 1, row: -1 },
      { type: 'office', column: 0, row: 2 },
      { type: 'office', column: 1, row: 2 },
    ]);
    expect(converted.objects).toEqual([
      { ...original.objects[0], placement: { ...original.objects[0]!.placement, y: 100 } },
      { ...original.objects[1], placement: { ...original.objects[1]!.placement, y: 88 } },
    ]);
    const history = worldHistory.commit(worldHistory.create(original), converted);
    expect(worldHistory.current(worldHistory.undo(history))).toEqual(before);
    expect(worldHistory.current(worldHistory.redo(worldHistory.undo(history)))).toEqual(converted);
    expect(original).toEqual(before);
    expect(upgradeModuleWorld(converted)).toBe(converted);
  });

it('rejects ambiguous common-floor contents without mutating or silently dropping them', () => {
  const world = source(3);
  const object = world.objects[0]!;
  const input = { ...world, objects: [{ ...object, placement: { ...object.placement, y: 42 } }] };
  const before = structuredClone(input);
  expect(() => upgradeModuleWorld(input)).toThrow(`Move object ${object.id}`);
  expect(input).toEqual(before);
});

it('converts free-form areas explicitly without overwriting their IDs or contents', () => {
  const modular = source(3);
  const world = { ...modular, map: mapGeometry(modular.map) };
  const before = structuredClone(world);
  expect(upgradeModuleWorld(world)).toEqual(upgradeModuleWorld(modular));
  expect(
    upgradeModuleWorld({ ...world, map: { ...world.map, areas: [...world.map.areas].reverse() } })
  ).toEqual(upgradeModuleWorld(world));
  expect(world).toEqual(before);
});

it('converts a single retained Lobby without inventing residents or extra offices', () => {
  const world = officeWorldFixture().layout;
  const before = structuredClone(world);
  const converted = upgradeModuleWorld(world);
  expect(converted.map).toEqual({
    version: 4,
    primaryLobbyId: world.map.primaryLobbyId,
    modules: [{ area: world.map.areas[0], slot: { type: 'lobby' }, material: 'workshop' }],
  });
  expect(converted.objects).toEqual(world.objects);
  expect(world).toEqual(before);
});

it('rejects an object crossing an unzoned hole even when it fits the area bounding box', () => {
  const world = officeWorldFixture().layout;
  const input = {
    ...world,
    map: {
      ...world.map,
      floor: world.map.floor.map((span) => (span.y === 5 ? { ...span, start: 6 } : span)),
    },
  };
  expect(() => upgradeModuleWorld(input)).toThrow('fully inside a room');
});

it('does not discard an additional Lobby to force a modular conversion', () => {
  const world = officeWorldFixture().layout;
  const extra = { ...world.map.areas[0]!, id: '10000000-0000-4000-8000-000000000098' };
  const input = {
    ...world,
    map: {
      ...world.map,
      areas: [...world.map.areas, extra],
      floor: [...world.map.floor, { y: 40, start: 0, end: 8, areaId: extra.id }],
    },
  };
  expect(() => upgradeModuleWorld(input)).toThrow('Consolidate extra Lobby');
  expect(input.map.areas).toHaveLength(2);
});

it('keeps oversized-room contents instead of moving them into another module or corridor', () => {
  const world = officeWorldFixture().layout;
  const area = {
    id: '10000000-0000-4000-8000-000000000097',
    name: 'Wide studio',
    binding: { type: 'personal' as const, identityId: null },
  };
  const input = {
    ...world,
    map: {
      ...world.map,
      areas: [...world.map.areas, area],
      floor: [
        ...world.map.floor,
        ...Array.from({ length: 36 }, (_, y) => ({ y, start: 40, end: 140, areaId: area.id })),
      ],
    },
    objects: world.objects.map((object) => ({
      ...object,
      placement: { ...object.placement, x: 110 },
    })),
  };
  const before = structuredClone(input);
  expect(() => upgradeModuleWorld(input)).toThrow('does not fit its destination room');
  expect(input).toEqual(before);
});

it('preserves meeting slots, personal assignments and materials independently of southern expansion', () => {
  const world = source(3);
  if (world.map.version === 1) throw new Error('Expected modules');
  const modules = world.map.modules.map((module, index) =>
    index === 3
      ? {
          ...module,
          material: 'copper' as const,
          area: {
            ...module.area,
            binding: {
              type: 'personal' as const,
              identityId: '20000000-0000-4000-8000-000000000001',
            },
          },
        }
      : module
  );
  const meeting = {
    area: {
      id: '10000000-0000-4000-8000-000000000090',
      name: 'Review',
      binding: { type: 'meeting' as const, roomId: '20000000-0000-4000-8000-000000000090' },
    },
    material: 'moonlight' as const,
    slot: { type: 'meeting' as const, index: 0 },
  };
  const object = {
    ...world.objects[0]!,
    placement: { ...world.objects[0]!.placement, x: 128, y: 8 },
  };
  const converted = upgradeModuleWorld({
    ...world,
    map: { ...world.map, modules: [...modules, meeting] },
    objects: [object],
  });
  if (converted.map.version === 1) throw new Error('Expected modules');
  expect(converted.map.modules[3]).toEqual({
    ...modules[3],
    slot: { type: 'office', column: 0, row: 2 },
  });
  expect(converted.map.modules[5]).toEqual(meeting);
  expect(converted.objects).toEqual([object]);
});
