import { expect, it } from 'vite-plus/test';
import vectors from '../../../../../contracts/modules-central-grid-vectors.json';
import { officeWorldFixture } from '../../../../../../../typescript/test/support/office-world.js';
import { decodeModuleMap } from './module-contract.js';
import { addMeetingModule, meetingExpansionPassages, nextMeetingSlot } from './meeting-module.js';
import { mapGeometry } from './map-source.js';
import { worldHistory } from './world-draft.js';
import { previewModuleRemoval, removeModule } from './module-authoring.js';

const room = {
  id: '30000000-0000-4000-8000-000000000001',
  name: 'Design review',
  revision: 1,
  retired: false,
  memberIds: ['40000000-0000-4000-8000-000000000001'],
};
const areaId = '50000000-0000-4000-8000-000000000001';
function starter() {
  return {
    ...officeWorldFixture().layout,
    map: decodeModuleMap({ ...vectors.map, modules: vectors.map.modules.slice(0, 5) }),
  };
}

it('creates a furnished draft attached to a canonical room without changing existing contents', () => {
  const original = starter();
  const baseline = structuredClone(original);
  const slot = nextMeetingSlot(original.map)!;
  expect(slot).toEqual({ type: 'meeting', index: 0 });
  const next = addMeetingModule(original, slot, room, areaId);
  expect(next.map.version).toBe(4);
  if (next.map.version === 1) throw new Error('Expected modular source');
  expect(next.map.modules.slice(0, 5)).toEqual(original.map.modules);
  expect(next.map.modules[5]).toEqual({
    area: { id: areaId, name: room.name, binding: { type: 'meeting', roomId: room.id } },
    slot,
    material: 'workshop',
  });
  expect(next.objects.slice(0, original.objects.length)).toEqual(original.objects);
  const added = next.objects.slice(original.objects.length);
  expect(added).toHaveLength(9);
  expect(added.flatMap((object) => (object.extension ? [object.extension.binding] : []))).toEqual([
    { kind: 'whiteboard', documentId: room.id },
    { kind: 'office-board', roomId: room.id },
    { kind: 'office-broadcast' },
  ]);
  const history = worldHistory.commit(worldHistory.create(original), next);
  expect(worldHistory.current(worldHistory.undo(history))).toEqual(original);
  expect(worldHistory.current(worldHistory.redo(worldHistory.undo(history)))).toEqual(next);
  expect(original).toEqual(baseline);
  expect(room.memberIds).toEqual(['40000000-0000-4000-8000-000000000001']);
  expect(previewModuleRemoval(next, areaId).blockedObjects.map((object) => object.id)).toEqual(
    added.map((object) => object.id)
  );
});

it('previews only new public circulation and never repacks a sparse meeting wing', () => {
  const source = decodeModuleMap(vectors.map);
  const next = nextMeetingSlot(source)!;
  expect(next).toEqual({ type: 'meeting', index: 4 });
  const before = mapGeometry(source).floor;
  const preview = meetingExpansionPassages(source, next);
  expect(preview.length).toBeGreaterThan(0);
  const cells = new Set<string>();
  for (const rect of preview) {
    for (let y = rect.y; y < rect.y + rect.height; y++) {
      for (let x = rect.x; x < rect.x + rect.width; x++) {
        expect(before.some((row) => row.y === y && row.start <= x && x < row.end)).toBe(false);
        const key = `${x},${y}`;
        expect(cells.has(key)).toBe(false);
        cells.add(key);
      }
    }
  }
  const world = { ...starter(), map: source };
  const removed = removeModule(world, source.modules[6]!.area.id);
  if (removed.map.version === 1) throw new Error('Expected modular source');
  expect(nextMeetingSlot(removed.map)).toEqual(next);
  expect(removed.map.modules.find((module) => module.slot.type === 'meeting')?.slot).toEqual({
    type: 'meeting',
    index: 3,
  });
});

it('rejects stale slots and retired or duplicate room bindings without mutating the world', () => {
  const original = starter();
  const slot = nextMeetingSlot(original.map)!;
  expect(() => addMeetingModule(original, slot, { ...room, retired: true }, areaId)).toThrow(
    'retired'
  );
  expect(() => addMeetingModule(original, { type: 'meeting', index: 2 }, room, areaId)).toThrow(
    'no longer available'
  );
  const next = addMeetingModule(original, slot, room, areaId);
  if (next.map.version === 1) throw new Error('Expected modular source');
  const appendSlot = nextMeetingSlot(next.map)!;
  expect(() => addMeetingModule(next, slot, room, areaId)).toThrow('no longer available');
  expect(() =>
    addMeetingModule(next, appendSlot, room, '50000000-0000-4000-8000-000000000002')
  ).toThrow('already has a space');
  expect(original.map.modules).toHaveLength(5);
});
