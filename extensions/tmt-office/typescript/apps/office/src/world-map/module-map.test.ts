import { describe, expect, it } from 'vitest';
import vectors from '../../../../../../../contracts/office/modules-v2-vectors.json';
import { officeWorldFixture } from '../../../../../../../typescript/test/support/office-world.js';
import { decodeModuleMap } from './module-contract.js';
import type { OfficeModule } from './module-contract.js';
import { MODULE_METRICS, moduleBounds, projectModules } from './module-geometry.js';
import { decodeMapSource, mapGeometry } from './map-source.js';
import { decodeWorldDocument, sameWorld } from './world-contract.js';
import { updateWorldMap, worldHistory } from './world-draft.js';

const starter = () => decodeModuleMap(structuredClone(vectors.starter));
function meeting(index: number): OfficeModule {
  const id = `20000000-0000-4000-8000-${String(index + 1).padStart(12, '0')}`;
  return {
    area: { id, name: `Meeting ${index + 1}`, binding: { type: 'meeting', roomId: id } },
    slot: { type: 'meeting', index },
    material: 'workshop',
  };
}

describe('module source and shared native/browser geometry', () => {
  it('keeps old bridges readable while grid drafts connect common lanes and junctions', () => {
    const old = starter();
    const source = decodeModuleMap({ ...old, version: 3 });
    const grid = projectModules(source);
    const at = (x: number, y: number) =>
      grid.floor.find((row) => row.y === y && row.start <= x && x < row.end);
    expect(grid.floor.reduce((sum, row) => sum + row.end - row.start, 0)).toBe(14144);
    for (const [x, y] of [
      [50, -40],
      [50, -4],
      [1, -4],
      [100, -4],
      [50, 44],
      [50, 80],
    ] as const)
      expect(at(x, y)?.areaId).toBeNull();
    expect(at(20, -20)?.areaId).toBe(old.modules[1]!.area.id);
    expect(grid.doors).toEqual(projectModules(old).doors);
    expect(grid.areas).toEqual(projectModules(old).areas);
    expect(projectModules(old).floor.reduce((sum, row) => sum + row.end - row.start, 0)).toBe(
      12224
    );
    const original = { ...officeWorldFixture().layout, map: old };
    const next = { ...original, map: source };
    const history = worldHistory.commit(worldHistory.create(original), next);
    expect(worldHistory.current(worldHistory.undo(history))).toEqual(original);
    expect(worldHistory.current(worldHistory.redo(worldHistory.undo(history)))).toEqual(next);
    expect(JSON.parse(JSON.stringify(decodeMapSource(source)))).toEqual(source);
  });

  it('matches literal starter dimensions, floor coverage and doorway coordinates', () => {
    const source = starter();
    const geometry = projectModules(source);
    expect(MODULE_METRICS).toEqual(vectors.metrics);
    expect(source.modules.map((module) => moduleBounds(module, source.version))).toEqual(
      vectors.expected.bounds
    );
    expect(geometry.floor.reduce((count, row) => count + row.end - row.start, 0)).toBe(
      vectors.expected.tiles
    );
    expect(geometry.doors).toHaveLength(vectors.expected.openings);
    for (const edge of vectors.expected.doorSamples) expect(geometry.doors).toContainEqual(edge);
    expect(geometry.areas).toEqual(source.modules.map((module) => module.area));
    expect(source.modules.slice(1).map((module) => module.area.binding)).toEqual(
      Array.from({ length: 4 }, () => ({ type: 'personal', identityId: null }))
    );
  });

  it('keeps meeting indices and main offices stable across spatial removal', () => {
    const base = starter();
    const first = meeting(0),
      last = meeting(3);
    const full = { ...base, modules: [...base.modules, first, last] };
    const removed = { ...full, modules: full.modules.filter((module) => module !== first) };
    const geometry = projectModules(removed);
    expect(moduleBounds(last, base.version)).toEqual({ x: 120, y: 144, width: 48, height: 40 });
    expect(geometry.floor.filter((row) => row.areaId === last.area.id)).toEqual(
      projectModules(full).floor.filter((row) => row.areaId === last.area.id)
    );
    for (const module of base.modules)
      expect(geometry.floor.filter((row) => row.areaId === module.area.id)).toEqual(
        projectModules(base).floor.filter((row) => row.areaId === module.area.id)
      );
    // The gap is circulation only: removing a room never invents a new area.
    expect(geometry.areas).not.toContainEqual(first.area);
    expect(geometry.floor).toContainEqual({ y: 100, start: 112, end: 120, areaId: null });
    expect(geometry.doors).toContainEqual({ x: 120, y: 160, axis: 'vertical' });
    expect(projectModules(base).floor.some((row) => row.start >= 104)).toBe(false);
  });

  it('stores only owned source values and treats material edits as real undoable changes', () => {
    const input = structuredClone(vectors.starter);
    const world = decodeWorldDocument({ ...officeWorldFixture().layout, map: input });
    input.modules[0]!.area.name = 'Mutated caller';
    expect(mapGeometry(world.map).areas[0]!.name).toBe('Lobby');
    expect(mapGeometry(world.map)).toBe(mapGeometry(world.map));
    const map = starter();
    const changed = decodeWorldDocument({
      ...world,
      map: { ...map, modules: map.modules.map((module) => ({ ...module, material: 'copper' })) },
    });
    expect(mapGeometry(changed.map)).toEqual(mapGeometry(world.map));
    expect(changed.objects).toEqual(world.objects);
    expect(sameWorld(world, changed)).toBe(false);
    const history = worldHistory.commit(worldHistory.create(world), changed);
    expect(worldHistory.current(worldHistory.undo(history))).toEqual(world);
    expect(worldHistory.current(worldHistory.redo(worldHistory.undo(history)))).toEqual(changed);
    expect(JSON.parse(JSON.stringify(changed)).map).toEqual(changed.map);
    expect(changed.map).not.toHaveProperty('floor');
    expect(changed.map).not.toHaveProperty('doors');
    expect(changed.map).not.toHaveProperty('areas');
    expect(() => updateWorldMap(world, mapGeometry(world.map))).toThrow('Edit module slots');
  });

  it('normalizes source ordering for equality without changing object paint order', () => {
    const map = starter();
    const world = { ...officeWorldFixture().layout, map };
    expect(
      sameWorld(world, { ...world, map: { ...map, modules: [...map.modules].reverse() } })
    ).toBe(true);
  });

  it('rejects duplicate identities of modules and competing geometry on the wire', () => {
    const source = starter();
    for (const extra of [{ floor: [] }, { doors: [] }, { areas: [] }])
      expect(() => decodeMapSource({ ...source, ...extra })).toThrow();
    expect(() =>
      decodeMapSource({ ...source, modules: [...source.modules, source.modules[0]] })
    ).toThrow(/Duplicate/);
    expect(() => decodeMapSource({ ...source, modules: [] })).toThrow(/primary Lobby/);
  });

  it('rejects unrenderable slots at the boundary instead of deferring a canvas crash to Save', () => {
    const base = starter();
    function replacing(module: unknown) {
      return { ...base, modules: [base.modules[0]!, module] };
    }
    const office = base.modules[1]!;
    for (const slot of [
      { type: 'office', column: 0, row: 0 },
      { type: 'office', column: 2, row: 1 },
      { type: 'office', column: 2147483647, row: 0 },
      { type: 'office', column: 0.5, row: 0 },
      { type: 'meeting', index: -1 },
      { type: 'meeting', index: 0 },
    ])
      expect(() => decodeMapSource(replacing({ ...office, slot }))).toThrow();
    expect(() => decodeMapSource(replacing({ ...office, material: 'unknown' }))).toThrow();
  });
});
