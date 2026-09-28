import { describe, expect, it } from 'vitest';
import vectors from '../../../../../contracts/map-v1-vectors.json';
import { decodeMapDocument, MAP_LIMITS } from './map-contract.js';
import { projectMap } from './map-geometry.js';

describe('Office map document', () => {
  it('conforms to the shared versioned admission budgets', () => {
    expect(MAP_LIMITS).toEqual(vectors.limits);
  });

  it('produces stable boundary order independently of row-run input order', () => {
    const map = decodeMapDocument({ ...vectors.lobby, ...vectors.cases[1]!.replace });
    const reversed = { ...map, floor: [...map.floor].reverse(), doors: [...map.doors].reverse() };
    expect(projectMap(reversed).boundaries).toEqual(projectMap(map).boundaries);
  });
  it.each(vectors.cases.filter((test) => 'tiles' in test))(
    'projects the native geometry vector: $name',
    (test) => {
      const map = decodeMapDocument({ ...vectors.lobby, ...test.replace });
      const geometry = projectMap(map);
      expect(geometry.tileCount).toBe(test.tiles);
      expect(geometry.boundaries).toHaveLength(test.walls!);
      expect(geometry.boundaries.filter((edge) => edge.open)).toHaveLength(test.openings ?? 0);
    }
  );

  it.each(
    vectors.cases.filter(
      (test) => test.error === 'invalidJson' || test.error === 'unsupportedVersion'
    )
  )('rejects the native wire-shape vector: $name', (test) =>
    expect(() => decodeMapDocument({ ...vectors.lobby, ...test.replace })).toThrow()
  );

  it('distinguishes draft preview from native save admission', () => {
    for (const test of vectors.cases.filter((entry) =>
      ['PrimaryLobbyRequired', 'EmptyArea', 'DisconnectedArea', 'InvalidDoor'].includes(
        entry.error ?? ''
      )
    )) {
      expect(() =>
        projectMap(decodeMapDocument({ ...vectors.lobby, ...test.replace }))
      ).not.toThrow();
    }
    // An invalid exterior door must not appear as a hole in the hull.
    const geometry = projectMap(
      decodeMapDocument({ ...vectors.lobby, doors: [{ x: 0, y: 0, axis: 'horizontal' }] })
    );
    expect(geometry.boundaryAt({ x: 0, y: 0, axis: 'horizontal' })?.open).toBe(false);
  });

  it('requires explicit nulls and rejects unknown nested fields', () => {
    for (const binding of [
      { type: 'lobby', roomId: vectors.lobby.primaryLobbyId },
      { type: 'personal' },
      { type: 'personal', identityId: null, extra: true },
      { type: 'meeting', roomId: null },
    ]) {
      expect(() =>
        decodeMapDocument({ ...vectors.lobby, areas: [{ ...vectors.lobby.areas[0], binding }] })
      ).toThrow();
    }
    expect(() =>
      decodeMapDocument({ ...vectors.lobby, floor: [{ y: 0, start: 0, end: 1 }] })
    ).toThrow();
    const map = decodeMapDocument({
      ...vectors.lobby,
      floor: [{ y: 0, start: 0, end: 1, areaId: null }],
    });
    const geometry = projectMap(map);
    expect(geometry.areaAt(0, 0)).toBeNull();
    expect(geometry.areaAt(1, 0)).toBeUndefined();
  });

  it('bounds resource work before projection and rejects ambiguous floor ownership', () => {
    for (const y of [NaN, Infinity, 0.5, MAP_LIMITS.coordinate])
      expect(() =>
        decodeMapDocument({ ...vectors.lobby, floor: [{ y, start: 0, end: 1, areaId: null }] })
      ).toThrow();
    expect(() =>
      decodeMapDocument({
        ...vectors.lobby,
        floor: Array.from({ length: 33 }, (_, y) => ({ y, start: -4096, end: 4096, areaId: null })),
      })
    ).toThrow();
    expect(() =>
      projectMap(
        decodeMapDocument({
          ...vectors.lobby,
          floor: [...vectors.lobby.floor, vectors.lobby.floor[0]],
        })
      )
    ).toThrow(/Overlapping/);
  });

  it('does not allocate the bounding rectangle or invent floor in holes', () => {
    const geometry = projectMap(
      decodeMapDocument({
        ...vectors.lobby,
        floor: [
          { y: -4096, start: -4096, end: -4095, areaId: null },
          { y: 4095, start: 4095, end: 4096, areaId: null },
        ],
      })
    );
    expect(geometry.bounds).toEqual({ x: -4096, y: -4096, width: 8192, height: 8192 });
    expect(geometry.tileCount).toBe(2);
    expect(geometry.boundaries).toHaveLength(8);
    expect(geometry.areaAt(0, 0)).toBeUndefined();
  });

  it('keeps projection ownership isolated from subsequent draft replacement', () => {
    const map = decodeMapDocument(vectors.lobby);
    const geometry = projectMap(map);
    const removed = projectMap({ ...map, floor: [] });
    expect(geometry.tileCount).toBe(6);
    expect(removed.tileCount).toBe(0);
    expect(removed.bounds).toBeNull();
    expect(vectors.lobby.floor).toHaveLength(2);
  });
});
