import { afterEach, expect, it } from 'vite-plus/test';
import { officeWorldFixture } from '../../../../../../../typescript/test/support/office-world.js';
import { createWorldYjs } from './world-yjs.js';
import type { WorldDocument } from './world-contract.js';
import vectors from '../../../../../contracts/modules-v2-vectors.json';
import { decodeModuleMap } from './module-contract.js';
import { compactModuleWorld, skybridgeModuleWorld } from './module-upgrade.js';

const sessions: ReturnType<typeof createWorldYjs>[] = [];
afterEach(() => {
  for (const session of sessions.splice(0)) session.destroy();
});
function fixture() {
  const world = officeWorldFixture().layout;
  const session = createWorldYjs(world);
  sessions.push(session);
  return { world, session };
}
function moved(world: WorldDocument, x: number): WorldDocument {
  return {
    ...world,
    objects: world.objects.map((object, index) =>
      index === 0 ? { ...object, placement: { ...object.placement, x } } : object
    ),
  };
}

it('commits physical support and placement as one undoable object without changing artwork', () => {
  const { world, session } = fixture();
  const source = world.objects[0]!;
  const candidate: WorldDocument = {
    ...world,
    objects: [
      {
        ...source,
        placement: { ...source.placement, x: 8, y: 4, rotation: 1 },
        surface: { type: 'floor', base: { x: 0, y: 0, width: 1, height: 1 } },
      },
      ...world.objects.slice(1),
    ],
  };
  session.change(candidate);
  expect(session.world).toEqual(candidate);
  session.undo();
  expect(session.world).toEqual(world);
  expect(session.canUndo).toBe(false);
  session.redo();
  expect(session.world).toEqual(candidate);
  expect(session.world.objects[0]!.placement.prop).toBe(source.placement.prop);
});

it('records each completed gesture separately, excluding bootstrap and no-op changes', () => {
  const { world, session } = fixture();
  expect(session.canUndo).toBe(false);
  session.change(world);
  expect(session.canUndo).toBe(false);
  session.change(moved(world, 8));
  session.change(moved(world, 12));
  session.undo();
  expect(session.world.objects[0]!.placement.x).toBe(8);
  session.undo();
  expect(session.world).toEqual(world);
  expect(session.canUndo).toBe(false);
  session.redo();
  expect(session.world.objects[0]!.placement.x).toBe(8);
  session.change(moved(world, 16));
  expect(session.canRedo).toBe(false);
});

it('undoes a local move without reverting an observed insertion, and preserves order', () => {
  const { world, session } = fixture();
  session.change(moved(world, 8));
  const other = { ...world.objects[0]!, id: '30000000-0000-4000-8000-000000000002' };
  session.observe({ ...session.world, objects: [...session.world.objects, other] });
  session.undo();
  expect(session.world.objects).toEqual([...world.objects, other]);
  expect(session.canUndo).toBe(false);
  session.redo();
  expect(session.world.objects).toEqual([...moved(world, 8).objects, other]);
});

it('does not overwrite a later observed change to the same entity', () => {
  const { world, session } = fixture();
  session.change(moved(world, 8));
  session.observe(moved(world, 12));
  session.undo();
  expect(session.world).toEqual(moved(world, 12));
  expect(session.canUndo).toBe(false);
});

it('does not redo a move over a later observed replacement of that entity', () => {
  const { world, session } = fixture();
  session.change(moved(world, 8));
  session.undo();
  session.observe(moved(world, 12));
  session.redo();
  expect(session.world).toEqual(moved(world, 12));
  expect(session.canRedo).toBe(false);
});

it('does not resurrect an externally deleted object when undoing an older move', () => {
  const { world, session } = fixture();
  session.change(moved(world, 8));
  session.observe({ ...world, objects: [] });
  session.undo();
  expect(session.world.objects).toEqual([]);
  session.redo();
  expect(session.world.objects).toEqual([]);
});

it('does not redo creation after the created object was externally deleted', () => {
  const { world, session } = fixture();
  const added = { ...world.objects[0]!, id: '30000000-0000-4000-8000-000000000002' };
  session.change({ ...world, objects: [...world.objects, added] });
  session.observe(world);
  session.undo();
  session.redo();
  expect(session.world).toEqual(world);
});

it('undoes creation without deleting an independently observed object', () => {
  const { world, session } = fixture();
  const local = { ...world.objects[0]!, id: '30000000-0000-4000-8000-000000000002' };
  const external = { ...world.objects[0]!, id: '30000000-0000-4000-8000-000000000003' };
  session.change({ ...world, objects: [...world.objects, local] });
  session.observe({ ...world, objects: [...world.objects, local, external] });
  session.undo();
  expect(session.world.objects).toEqual([...world.objects, external]);
  session.redo();
  expect(session.world.objects).toEqual([...world.objects, local, external]);
});

it('restores a removed object at its paint position after an observed insertion', () => {
  const { world, session } = fixture();
  const second = { ...world.objects[0]!, id: '30000000-0000-4000-8000-000000000002' };
  const third = { ...world.objects[0]!, id: '30000000-0000-4000-8000-000000000003' };
  session.reset({ ...world, objects: [...world.objects, second] });
  session.change({ ...world, objects: [second] });
  session.observe({ ...world, objects: [second, third] });
  session.undo();
  expect(session.world.objects).toEqual([...world.objects, second, third]);
  session.redo();
  expect(session.world.objects).toEqual([second, third]);
});

it('coalesces only an explicit typing group and closes it on blur or another gesture', () => {
  const { world, session } = fixture();
  session.change(moved(world, 8), 'typed-coordinate');
  session.change(moved(world, 12), 'typed-coordinate');
  session.stopCapturing();
  session.change(moved(world, 16), 'typed-coordinate');
  session.undo();
  expect(session.world.objects[0]!.placement.x).toBe(12);
  session.undo();
  expect(session.world).toEqual(world);
});

it('preserves another module while reversing a local material change', () => {
  const { world, session } = fixture();
  const map = decodeModuleMap(vectors.starter);
  const source = { ...world, map, objects: [] };
  session.reset(source);
  const local = {
    ...source,
    map: {
      ...map,
      modules: map.modules.map((module, index) =>
        index === 1 ? { ...module, material: 'copper' as const } : module
      ),
    },
  };
  session.change(local);
  const external = {
    ...local,
    map: {
      ...local.map,
      modules: local.map.modules.map((module, index) =>
        index === 2 ? { ...module, area: { ...module.area, name: 'Agent office' } } : module
      ),
    },
  };
  session.observe(external);
  session.undo();
  expect(session.world).toEqual({
    ...source,
    map: {
      ...map,
      modules: map.modules.map((module, index) => (index === 2 ? external.map.modules[2] : module)),
    },
  });
});

it('rolls back a selective inverse that would overlap an externally created module', () => {
  const { world, session } = fixture();
  const map = decodeModuleMap(vectors.starter);
  session.reset({ ...world, map, objects: [] });
  const office = map.modules[1]!;
  const local = {
    ...world,
    objects: [],
    map: { ...map, modules: map.modules.filter((module) => module !== office) },
  };
  session.change(local);
  const external = {
    ...local,
    map: {
      ...map,
      modules: [
        ...local.map.modules,
        { ...office, area: { ...office.area, id: '20000000-0000-4000-8000-000000000042' } },
      ],
    },
  };
  session.observe(external);
  expect(() => session.undo()).toThrow('conflicts with the current layout');
  expect(session.world).toEqual(external);
  expect(session.canUndo).toBe(true);
  expect(session.canRedo).toBe(false);
  // A subsequent edit must see the restored Y.Doc, not only a cached projection.
  session.change({ ...external, objects: world.objects });
  session.undo();
  expect(session.world).toEqual(external);
});

it('reverses source-format upgrades together with dependent object relocation', () => {
  const { world, session } = fixture();
  for (const version of [1, 2, 5] as const) {
    const modular: WorldDocument = { ...world, objects: [], map: decodeModuleMap(vectors.starter) };
    const source = version === 1 ? world : version === 5 ? compactModuleWorld(modular) : modular;
    session.reset(source);
    const upgraded = skybridgeModuleWorld(source);
    expect(upgraded.map.version).toBe(6);
    session.change(upgraded);
    session.undo();
    expect(session.world).toEqual(source);
    session.redo();
    expect(session.world).toEqual(upgraded);
  }
});

it('does not reverse a format conversion across external changes in the new representation', () => {
  const { world, session } = fixture();
  const upgraded = skybridgeModuleWorld(world);
  session.change(upgraded);
  const observed = moved(upgraded, 8);
  session.observe(observed);
  expect(session.canUndo).toBe(false);
  expect(session.historyNotice).toContain('format-conversion');
  session.undo();
  expect(session.world).toEqual(observed);
});

it('keeps topology and dependent objects in one reversible transaction', () => {
  const { world, session } = fixture();
  const next = {
    ...world,
    map: { ...world.map, doors: [{ x: 0, y: 1, axis: 'vertical' as const }] },
    objects: [],
  };
  session.change(next);
  session.undo();
  expect(session.world).toEqual(world);
  session.redo();
  expect(session.world).toEqual(next);
});

it('rejects malformed input before changing state or history', () => {
  const { world, session } = fixture();
  expect(() =>
    session.change({ ...world, objects: [world.objects[0]!, world.objects[0]!] })
  ).toThrow();
  expect(session.world).toEqual(world);
  expect(session.canUndo).toBe(false);
});

it('explicit reload replaces history, and disposal can resume for React effect replay', () => {
  const { world, session } = fixture();
  session.change(moved(world, 8));
  session.reset(moved(world, 12));
  expect(session.canUndo).toBe(false);
  session.destroy();
  session.destroy();
  session.resume();
  expect(session.world).toEqual(moved(world, 12));
  session.change(world);
  session.undo();
  expect(session.world).toEqual(moved(world, 12));
});
