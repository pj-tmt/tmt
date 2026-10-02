import { describe, expect, it } from 'vite-plus/test';
import vectors from '../../../../../contracts/modules-central-grid-vectors.json';
import { decodeModuleMap } from './module-contract.js';
import { moduleBounds, projectModules, officeExpansionPassages } from './module-geometry.js';
import { officeExpansionSlots } from './module-authoring.js';
import { decodeMapSource } from './map-source.js';

const source = () => decodeModuleMap(vectors.map);
describe('central Lobby lattice', () => {
  it('omits budget-breaking ghosts while retaining valid in-bounds additions', () => {
    const original = source();
    const map = {
      ...original,
      modules: [
        ...original.modules.slice(0, 5),
        {
          ...original.modules[5]!,
          slot: { type: 'office', column: -14, row: -18 } as const,
        },
      ],
    };
    expect(projectModules(map).floor.reduce((sum, span) => sum + span.end - span.start, 0)).toBe(
      260160
    );
    const slots = officeExpansionSlots(map);
    expect(slots).not.toContainEqual({ type: 'office', column: -15, row: -18 });
    expect(slots).not.toContainEqual({ type: 'office', column: -14, row: -19 });
    expect(slots).toContainEqual({ type: 'office', column: -13, row: -18 });
    expect(slots).toContainEqual({ type: 'office', column: -14, row: -17 });
  });
  it('matches native literal bounds, public lanes, empty cells and central entrances', () => {
    const map = source();
    const geometry = projectModules(map);
    const at = (x: number, y: number) =>
      geometry.floor.find((span) => span.y === y && span.start <= x && x < span.end);
    expect(moduleBounds(map.modules[0]!, map.version)).toEqual(vectors.lobbyBounds);
    for (const [x, y] of vectors.publicSamples)
      expect(at(x!, y!)?.areaId, `public ${x},${y}`).toBeNull();
    for (const [x, y] of vectors.emptySamples)
      expect(at(x!, y!), `empty ${x},${y}`).toBeUndefined();
    for (const edge of vectors.lobbyDoorSamples) expect(geometry.doors).toContainEqual(edge);
    expect(projectModules({ ...map, modules: [...map.modules].reverse() })).toEqual(geometry);
    expect(decodeMapSource(map)).toEqual(map);
  });

  it('reserves all four Lobby cells without offering private rooms in the meeting wing', () => {
    const map = source();
    const slots = officeExpansionSlots(map);
    expect(slots).toContainEqual({ type: 'office', column: -1, row: 1 });
    for (const [column, row] of [
      [0, 0],
      [1, 0],
      [0, 1],
      [1, 1],
      [2, 0],
      [2, 1],
    ])
      expect(slots).not.toContainEqual({ type: 'office', column, row });
    expect(() =>
      projectModules({
        ...map,
        modules: [
          ...map.modules,
          {
            ...map.modules[1]!,
            slot: { type: 'office', column: 0, row: 1 },
          },
        ],
      })
    ).toThrow(/overlaps/);
  });

  it('previews only newly generated circulation, not an older Lobby or private floor', () => {
    const map = source();
    const slot = { type: 'office', column: 0, row: 3 } as const;
    const before = projectModules(map);
    const after = projectModules({
      ...map,
      modules: [
        ...map.modules,
        {
          ...map.modules[1]!,
          area: { ...map.modules[1]!.area, id: '10000000-0000-4000-8000-000000000009' },
          slot,
        },
      ],
    });
    const preview = officeExpansionPassages(map, slot);
    const at = (floor: typeof after.floor, x: number, y: number) =>
      floor.find((span) => span.y === y && span.start <= x && x < span.end);
    expect(preview.length).toBeGreaterThan(0);
    for (const rect of preview)
      for (let x = rect.x; x < rect.x + rect.width; x++) {
        expect(at(before.floor, x, rect.y)).toBeUndefined();
        expect(at(after.floor, x, rect.y)?.areaId).toBeNull();
      }
    for (const span of after.floor.filter((span) => span.areaId === null))
      for (let x = span.start; x < span.end; x++)
        if (!at(before.floor, x, span.y))
          expect(
            preview.some((rect) => rect.y === span.y && rect.x <= x && x < rect.x + rect.width)
          ).toBe(true);
  });

  it('rejects excessive empty-grid expansion before constructing a dense world', () => {
    const map = source();
    expect(() =>
      projectModules({
        ...map,
        modules: [
          ...map.modules,
          {
            ...map.modules[1]!,
            slot: { type: 'office', column: -70, row: -70 },
          },
        ],
      })
    ).toThrow(/budgets/);
  });
});
