import { expect, it } from 'vite-plus/test';
import vectors from '../../../../../contracts/modules-v2-vectors.json';
import { officeWorldFixture } from '../../../../../../../typescript/test/support/office-world.js';
import { decodeModuleMap, officeSlotKey } from './module-contract.js';
import {
  addOfficeModule,
  officeExpansionSlots,
  previewModuleRemoval,
  removeModule,
  setModuleMaterial,
} from './module-authoring.js';
import { worldHistory } from './world-draft.js';
import { mapGeometry } from './map-source.js';

const starter = () => ({ ...officeWorldFixture().layout, map: decodeModuleMap(vectors.starter) });
const id = '20000000-0000-4000-8000-000000000042';

it('changes only the selected module finish through the existing history without changing geometry or content', () => {
  const original = starter();
  const areaId = original.map.modules[1]!.area.id;
  const before = structuredClone(original);
  const next = setModuleMaterial(original, areaId, 'copper');
  expect(mapGeometry(next.map)).toEqual(mapGeometry(original.map));
  expect(next.objects).toBe(original.objects);
  expect(next.map).toEqual({
    ...original.map,
    modules: original.map.modules.map((module) =>
      module.area.id === areaId ? { ...module, material: 'copper' } : module
    ),
  });
  const history = worldHistory.commit(worldHistory.create(original), next);
  expect(worldHistory.current(worldHistory.undo(history))).toEqual(original);
  expect(worldHistory.current(worldHistory.redo(worldHistory.undo(history)))).toEqual(next);
  expect(original).toEqual(before);
  expect(() => setModuleMaterial(original, id, 'moonlight')).toThrow('no longer available');
  expect(() => setModuleMaterial(officeWorldFixture().layout, areaId, 'moonlight')).toThrow(
    'modular layout'
  );
});

it('offers only empty cardinal neighbors and reserves the independent meeting wing', () => {
  const slots = officeExpansionSlots(starter().map).map(officeSlotKey);
  expect(slots).toEqual(['0,-2', '1,-2', '-1,-1', '2,-1', '-1,0', '-1,1', '0,2', '1,2']);
  expect(slots).not.toContain('2,0');
  expect(slots).not.toContain('2,1');
  expect(slots).not.toContain('-1,-2');
});

it('removes only an empty module and its derived circulation through the same history', () => {
  const source = starter();
  const original = { ...source, map: { ...source.map, version: 3 as const } };
  const target = original.map.modules[1]!;
  const next = removeModule(original, target.area.id);
  expect(next.objects).toBe(original.objects);
  expect(next.map).toEqual({
    ...original.map,
    modules: original.map.modules.filter((module) => module !== target),
  });
  expect(mapGeometry(next.map).floor.some((span) => span.areaId === target.area.id)).toBe(false);
  const history = worldHistory.commit(worldHistory.create(original), next);
  expect(worldHistory.current(worldHistory.undo(history))).toEqual(original);
  expect(worldHistory.current(worldHistory.redo(worldHistory.undo(history)))).toEqual(next);
  expect(previewModuleRemoval(original, original.map.primaryLobbyId).reason).toBe(
    'The central Lobby is required.'
  );
  expect(() => removeModule(original, original.map.primaryLobbyId)).toThrow('central Lobby');
});

it('reports room furniture, disappearing common-floor furniture and mounted objects without deleting them', () => {
  const base = starter();
  const prop = base.objects[0]!;
  const objects = [
    {
      ...prop,
      id: '20000000-0000-4000-8000-000000000051',
      placement: { ...prop.placement, x: 4, y: -30 },
    },
    {
      ...prop,
      id: '20000000-0000-4000-8000-000000000052',
      placement: { ...prop.placement, x: 50, y: -40 },
    },
    {
      ...prop,
      id: '20000000-0000-4000-8000-000000000053',
      surface: {
        type: 'wall' as const,
        axis: 'horizontal' as const,
        face: 'positive' as const,
        elevation: 3,
      },
      placement: { ...prop.placement, x: 4, y: -48 },
    },
  ];
  const world = {
    ...base,
    map: { ...base.map, version: 3 as const },
    objects: [...base.objects, ...objects],
  };
  const snapshot = structuredClone(world);
  const preview = previewModuleRemoval(world, base.map.modules[1]!.area.id);
  expect(preview.blockedObjects.map((object) => object.id)).toEqual(
    objects.map((object) => object.id)
  );
  expect(() => removeModule(world, base.map.modules[1]!.area.id)).toThrow('affected placements');
  expect(world).toEqual(snapshot);
  expect(preview.next.objects).toBe(world.objects);
});

it('detaches a meeting module without renumbering later rooms or editing canonical room bindings', () => {
  const base = starter();
  const rooms = [0, 3].map((index) => ({
    area: {
      id: `30000000-0000-4000-8000-00000000000${index}`,
      name: `Room ${index}`,
      binding: { type: 'meeting' as const, roomId: `40000000-0000-4000-8000-00000000000${index}` },
    },
    slot: { type: 'meeting' as const, index },
    material: 'workshop' as const,
  }));
  const world = {
    ...base,
    map: { ...base.map, version: 3 as const, modules: [...base.map.modules, ...rooms] },
  };
  const next = removeModule(world, rooms[0]!.area.id);
  expect(next.map).toEqual({ ...world.map, modules: [...base.map.modules, rooms[1]] });
  expect(next.objects).toBe(world.objects);
  expect(() => removeModule(next, rooms[0]!.area.id)).toThrow('no longer available');
});

it('adds one unassigned office through undoable source state without changing existing contents', () => {
  const original = starter();
  const slot = { type: 'office' as const, column: -1, row: 0 };
  const next = addOfficeModule(original, slot, '  Design lab  ', id);
  expect(next.objects).toBe(original.objects);
  expect(next.map).toEqual({
    ...original.map,
    modules: [
      ...original.map.modules,
      {
        area: { id, name: 'Design lab', binding: { type: 'personal', identityId: null } },
        slot,
        material: 'workshop',
      },
    ],
  });
  const history = worldHistory.commit(worldHistory.create(original), next);
  expect(worldHistory.current(worldHistory.undo(history))).toEqual(original);
  expect(worldHistory.current(worldHistory.redo(worldHistory.undo(history)))).toEqual(next);
  expect(mapGeometry(next.map).doors).toContainEqual({ x: 0, y: 16, axis: 'vertical' });
  expect(() => addOfficeModule(next, slot, 'Duplicate', id)).toThrow('no longer available');
  expect(() => addOfficeModule(original, slot, '  ', id)).toThrow('name');
  expect(original.map.modules).toHaveLength(5);
});

it('does not invent a modular conversion or allow diagonal-only and reserved additions', () => {
  for (const [column, row] of [
    [-1, -2],
    [2, 0],
    [0, -1],
    [9999, 9999],
  ])
    expect(() =>
      addOfficeModule(starter(), { type: 'office', column: column!, row: row! }, 'Invalid', id)
    ).toThrow('no longer available');
  expect(() =>
    addOfficeModule(
      officeWorldFixture().layout,
      { type: 'office', column: -1, row: 0 },
      'Invalid',
      id
    )
  ).toThrow('modular layout');
});
