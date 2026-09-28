import { mapGeometry } from './map-source.js';
import { expect, it } from 'vitest';
import { officeWorldFixture } from '../../../../../../../typescript/test/support/office-world.js';
import { addMeetingPreset } from './meeting-preset.js';
import { worldHistory, updateWorldMap } from './world-draft.js';
import { removeArea } from './map-draft.js';
import { footprint } from '../blocks/block-contract.js';
import { platformModuleWorld } from './module-upgrade.js';
import vectors from '../../../../../../../contracts/office/modules-unified-vectors.json';
import { decodeModuleMap } from './module-contract.js';
import { projectMap } from './map-geometry.js';
import { placementProblem } from './world-object-placement.js';

const roomId = '20000000-0000-4000-8000-000000000002';
const areaId = '20000000-0000-4000-8000-000000000003';

it('keeps all four cardinal entrances clear in a unified meeting area', () => {
  const meeting = vectors.map.modules[2]!;
  const map = decodeModuleMap({
    ...vectors.map,
    modules: [
      vectors.map.modules[0],
      { ...meeting, slot: { type: 'office', column: 2, row: 0 } },
      ...[
        [2, -1],
        [3, 0],
        [2, 1],
      ].map(([column, row], index) => ({
        ...vectors.map.modules[1],
        area: {
          id: `30000000-0000-4000-8000-00000000000${index}`,
          name: `Neighbor ${index}`,
          binding: { type: 'personal', identityId: null },
        },
        slot: { type: 'office', column, row },
      })),
    ],
  });
  const world = { version: 1 as const, map, objects: [] };
  const geometry = projectMap(map);
  // Four center connections plus the south neighbor's own Lobby connection.
  expect(geometry.boundaries.filter((edge) => edge.open)).toHaveLength(80);
  const candidate = addMeetingPreset(world, meeting.area.id);
  expect(candidate.map).toEqual(map);
  expect(candidate.objects).toHaveLength(9);
  for (const object of candidate.objects)
    expect(placementProblem(geometry, object, candidate.objects)).toBeUndefined();
  expect(world.objects).toEqual([]);
});
function fixture() {
  const base = officeWorldFixture().layout;
  return {
    ...base,
    map: {
      ...base.map,
      areas: [
        ...mapGeometry(base.map).areas,
        { id: areaId, name: 'Design', binding: { type: 'meeting' as const, roomId } },
      ],
      floor: [
        ...mapGeometry(base.map).floor,
        ...Array.from({ length: 36 }, (_, y) => ({ y, start: 36, end: 72, areaId })),
      ],
      doors: [{ x: 36, y: 16, axis: 'vertical' as const }],
    },
  };
}

it('builds platform meeting facilities on the floor with distinct resource placements', () => {
  const original = platformModuleWorld(fixture());
  const before = structuredClone(original);
  const candidate = addMeetingPreset(original, areaId);
  const added = candidate.objects.slice(original.objects.length);
  expect(added).toHaveLength(9);
  expect(
    added.every((object) => object.surface.type === 'floor' && object.kind === 'decoration')
  ).toBe(true);
  expect(added.slice(0, 3).map((object) => object.extension?.binding)).toEqual([
    { kind: 'whiteboard', documentId: roomId },
    { kind: 'office-board', roomId },
    { kind: 'office-broadcast' },
  ]);
  expect(new Set(added.slice(0, 3).map((object) => object.placement.x)).size).toBe(3);
  expect(original).toEqual(before);
});

it('adds one undoable set without modifying existing objects, floor, membership or resources', () => {
  const original = fixture();
  const baseline = structuredClone(original);
  const candidate = addMeetingPreset(original, areaId);
  expect(original).toEqual(baseline);
  expect(candidate.map).toEqual(original.map);
  expect(candidate.objects[0]).toEqual(original.objects[0]);
  const added = candidate.objects.slice(original.objects.length);
  expect(added).toHaveLength(9);
  expect(new Set(candidate.objects.map((object) => object.id)).size).toBe(candidate.objects.length);
  expect(added.flatMap((object) => (object.extension ? [object.extension.binding] : []))).toEqual([
    { kind: 'whiteboard', documentId: roomId },
    { kind: 'office-board', roomId },
    { kind: 'office-broadcast' },
  ]);
  expect(added.slice(0, 3).map((object) => object.surface)).toEqual(
    Array.from({ length: 3 }, () => ({
      type: 'wall',
      axis: 'horizontal',
      face: 'positive',
      elevation: 3,
    }))
  );
  expect(added[3]!.placement).toMatchObject({
    footprint: { width: 16, height: 16 },
    x: 46,
    y: 0,
  });
  for (const object of added) {
    const size = footprint(object.placement);
    expect(object.placement.x).toBeGreaterThanOrEqual(36);
    expect(object.placement.x + size.width).toBeLessThanOrEqual(72);
    expect(object.placement.y).toBeGreaterThanOrEqual(0);
    expect(object.placement.y + size.height).toBeLessThanOrEqual(36);
  }
  const history = worldHistory.commit(worldHistory.create(original), candidate);
  expect(worldHistory.current(worldHistory.undo(history))).toEqual(original);
  expect(worldHistory.current(worldHistory.redo(worldHistory.undo(history)))).toEqual(candidate);
  const detached = updateWorldMap(candidate, removeArea(mapGeometry(candidate.map), areaId));
  expect(detached.objects).toEqual(candidate.objects);
  expect(mapGeometry(detached.map).floor.filter((span) => span.areaId === null)).toHaveLength(36);
});

it('does not overwrite occupied floor or fit across missing rows or a different area', () => {
  const world = fixture();
  const full = addMeetingPreset(world, areaId);
  expect(() => addMeetingPreset(full, areaId)).toThrow('clear 36 × 32');
  for (const y of [4, 18, 31]) {
    const hole = {
      ...world,
      map: {
        ...mapGeometry(world.map),
        floor: mapGeometry(world.map).floor.filter((row) => row.areaId !== areaId || row.y !== y),
      },
    };
    expect(() => addMeetingPreset(hole, areaId)).toThrow('clear 36 × 32');
  }
  expect(() => addMeetingPreset(world, world.map.primaryLobbyId)).toThrow('Choose a meeting area');
  expect(world).toEqual(fixture());
});

it('accepts segmented floor and finds the next free space without moving existing objects', () => {
  const world = fixture();
  const expanded = {
    ...world,
    map: {
      ...mapGeometry(world.map),
      floor: mapGeometry(world.map).floor.flatMap((row) =>
        row.areaId === areaId
          ? [
              { ...row, end: 54 },
              { ...row, start: 54, end: 108 },
            ]
          : [row]
      ),
    },
  };
  const first = addMeetingPreset(expanded, areaId);
  const second = addMeetingPreset(first, areaId);
  expect(second.objects.slice(0, first.objects.length)).toEqual(first.objects);
  expect(
    second.objects
      .slice(first.objects.length)
      .filter((object) => object.surface.type === 'floor')
      .every((object) => object.placement.x >= 72)
  ).toBe(true);
  expect(
    second.objects
      .filter((object) => object.extension?.binding.kind === 'whiteboard')
      .map((object) => object.extension!.binding)
  ).toEqual([
    { kind: 'whiteboard', documentId: roomId },
    { kind: 'whiteboard', documentId: roomId },
  ]);
});
