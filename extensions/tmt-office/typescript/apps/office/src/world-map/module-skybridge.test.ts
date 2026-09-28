import { expect, it } from 'vitest';
import { skybridgeCirculation } from './compact-circulation.js';
import vectors from '../../../../../contracts/modules-central-grid-vectors.json';
import bridges from '../../../../../contracts/modules-skybridge-vectors.json';
import { decodeModuleMap } from './module-contract.js';
import { moduleSlotBounds, projectModules } from './module-geometry.js';
import { skybridgeModuleWorld } from './module-upgrade.js';
import { worldHistory } from './world-draft.js';
import { officeWorldFixture } from '../../../../../../../typescript/test/support/office-world.js';
import { decodeMapSource } from './map-source.js';

const source = () =>
  decodeModuleMap({ ...vectors.map, version: 6, modules: vectors.map.modules.slice(0, 5) });

it('connects an extended row only across neighbor centers and remains order independent', () => {
  const map = source();
  const added = [2, 3].map((column) => ({
    ...map.modules[1]!,
    slot: { type: 'office' as const, column, row: -1 },
    area: { ...map.modules[1]!.area, id: `10000000-0000-4000-8000-00000000009${column}` },
  }));
  const input = { ...map, modules: [...map.modules, ...added] };
  const geometry = projectModules(input);
  const at = (x: number, y: number) =>
    geometry.floor.find((s) => s.y === y && s.start <= x && x < s.end);
  for (const [x, y] of bridges.extendedRowPublic) expect(at(x!, y!)?.areaId).toBeNull();
  for (const [x, y] of bridges.extendedRowEmpty) expect(at(x!, y!)).toBeUndefined();
  expect(geometry.doors).toContainEqual({ x: 104, y: -32, axis: 'vertical' });
  expect(geometry.doors).toContainEqual({ x: 112, y: -32, axis: 'vertical' });
  expect(projectModules({ ...input, modules: [...input.modules].reverse() })).toEqual(geometry);
  const removed = projectModules({
    ...input,
    modules: input.modules.filter((m) => m !== added[0]),
  });
  expect(removed.floor.some((s) => s.areaId === null && s.start >= 104 && s.y === -28)).toBe(false);
});

it('does not bridge diagonals or missing slots', () => {
  const rooms = [
    { x: 0, y: 0, width: 48, height: 40 },
    { x: 56, y: 48, width: 48, height: 40 },
    { x: 112, y: 0, width: 48, height: 40 },
  ];
  expect(skybridgeCirculation(rooms).passages).toEqual([]);
});

it('connects four neighboring offices only at their doors, leaving empty slots unbuilt', () => {
  const geometry = projectModules(source());
  const at = (x: number, y: number) =>
    geometry.floor.find((s) => s.y === y && s.start <= x && x < s.end);
  for (const [x, y] of bridges.starterPublic) expect(at(x!, y!)?.areaId).toBeNull();
  for (const [x, y] of bridges.starterEmpty) expect(at(x!, y!)).toBeUndefined();
  for (const [x, y] of bridges.lobbyDoorStarts)
    expect(geometry.doors).toContainEqual({ x, y, axis: 'horizontal' });
  expect(projectModules({ ...source(), modules: [...source().modules].reverse() })).toEqual(
    geometry
  );
});

it('separates meeting pods from the spine and keeps occupied branches through a missing slot', () => {
  const map = source();
  const meetings = bridges.meetingIndices.map((index) => ({
    ...map.modules[1]!,
    slot: { type: 'meeting' as const, index },
    area: {
      id: `10000000-0000-4000-8000-00000000001${index}`,
      name: `Meeting ${index}`,
      binding: { type: 'meeting' as const, roomId: `20000000-0000-4000-8000-00000000001${index}` },
    },
  }));
  const geometry = projectModules({ ...map, modules: [...map.modules, ...meetings] });
  const at = (x: number, y: number) =>
    geometry.floor.find((s) => s.y === y && s.start <= x && x < s.end);
  expect(moduleSlotBounds({ type: 'meeting', index: 2 }, 6)).toEqual(bridges.lastMeetingBounds);
  for (const [x, y] of bridges.meetingPublic) expect(at(x!, y!)?.areaId).toBeNull();
  for (const [x, y] of bridges.meetingEmpty) expect(at(x!, y!)).toBeUndefined();
  expect(geometry.doors).toContainEqual({ x: 136, y: 16, axis: 'vertical' });
  expect(geometry.doors).toContainEqual({ x: 136, y: 112, axis: 'vertical' });
});

it('previews skybridges reversibly without mutating saved sources', () => {
  const world = { version: 1 as const, map: { ...source(), version: 4 as const }, objects: [] };
  const before = structuredClone(world);
  const next = skybridgeModuleWorld(world);
  expect(next.map.version).toBe(6);
  expect(world).toEqual(before);
  expect(skybridgeModuleWorld(next)).toBe(next);
  const history = worldHistory.commit(worldHistory.create(world), next);
  expect(worldHistory.current(worldHistory.undo(history))).toEqual(before);
});

it('moves meeting contents with their pod while retaining identity, order and resource bindings', () => {
  const map = source();
  const meeting = {
    ...map.modules[1]!,
    slot: { type: 'meeting' as const, index: 1 },
    area: {
      id: '10000000-0000-4000-8000-000000000099',
      name: 'Review',
      binding: { type: 'meeting' as const, roomId: '20000000-0000-4000-8000-000000000099' },
    },
  };
  const object = officeWorldFixture().layout.objects[0]!;
  const world = {
    version: 1 as const,
    map: { ...map, version: 5 as const, modules: [...map.modules, meeting] },
    objects: [{ ...object, placement: { ...object.placement, x: 124, y: 44 } }],
  };
  const before = structuredClone(world);
  const next = skybridgeModuleWorld(world);
  expect(next.objects).toEqual([
    { ...world.objects[0], placement: { ...world.objects[0]!.placement, x: 140, y: 52 } },
  ]);
  expect(world).toEqual(before);
});

it('rejects public-floor contents before changing the source', () => {
  const object = officeWorldFixture().layout.objects[0]!;
  const world = {
    version: 1 as const,
    map: { ...source(), version: 5 as const },
    objects: [{ ...object, placement: { ...object.placement, x: 48, y: -4 } }],
  };
  const before = structuredClone(world);
  expect(() => skybridgeModuleWorld(world)).toThrow(/fully inside a room/);
  expect(world).toEqual(before);
});

it('counts the meeting wing together with office floor against the shared budget', () => {
  const map = source();
  const modules = [map.modules[0]!];
  for (let index = 0; index < 130; index++) {
    const id = `10000000-0000-4000-8000-${String(index + 10).padStart(12, '0')}`;
    modules.push({
      ...map.modules[1]!,
      slot:
        index < 50
          ? { type: 'office', column: -1 - (index % 5), row: Math.floor(index / 5) }
          : { type: 'meeting', index: index - 50 },
      area: {
        id,
        name: `Room ${index}`,
        binding:
          index < 50 ? { type: 'personal', identityId: null } : { type: 'meeting', roomId: id },
      },
    });
  }
  expect(() => decodeMapSource({ ...map, modules })).toThrow(/floor budgets/);
});
