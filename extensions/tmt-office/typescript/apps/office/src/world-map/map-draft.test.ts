import { describe, expect, it } from 'vitest';
import vectors from '../../../../../contracts/map-v1-vectors.json';
import { decodeMapDocument } from './map-contract.js';
import { canonicalMapDraft, removeArea } from './map-draft.js';
import { projectMap } from './map-geometry.js';

const multi = () => decodeMapDocument({ ...vectors.lobby, ...vectors.cases[1]!.replace });

describe('retained Office map normalization and repair', () => {
  it('detaches personal and meeting areas without erasing terrain or silently removing doors', () => {
    const map = multi();
    for (const area of map.areas.filter((area) => area.binding.type !== 'lobby')) {
      const next = removeArea(map, area.id);
      expect(projectMap(next).tileCount).toBe(6);
      expect(next.areas).toHaveLength(2);
      expect(next.floor.some((span) => span.areaId === area.id)).toBe(false);
      expect(next.doors).toEqual(map.doors);
      expect(next.primaryLobbyId).toBe(map.primaryLobbyId);
    }
  });

  it('protects the primary Lobby and accepts only an explicit other Lobby replacement', () => {
    const map = multi();
    const otherId = map.areas[1]!.id;
    expect(() => removeArea(map, map.primaryLobbyId)).toThrow(/replacement/);
    expect(() => removeArea(map, map.primaryLobbyId, otherId)).toThrow(/replacement/);
    const replacement = decodeMapDocument({
      ...map,
      areas: map.areas.map((area) =>
        area.id === otherId ? { ...area, binding: { type: 'lobby' } } : area
      ),
    });
    const next = removeArea(replacement, map.primaryLobbyId, otherId);
    expect(next.primaryLobbyId).toBe(otherId);
    expect(projectMap(next).tileCount).toBe(6);
    expect(projectMap(next).areaAt(0, 0)).toBeNull();
    expect(() => removeArea(map, otherId + 'missing')).toThrow(/Unknown/);
  });

  it('canonicalizes split row runs without expanding holes or changing their owner', () => {
    const source = decodeMapDocument({
      ...vectors.lobby,
      floor: [
        { y: 2, start: 4, end: 5, areaId: null },
        { y: 0, start: 1, end: 3, areaId: vectors.lobby.primaryLobbyId },
        { y: 0, start: 0, end: 1, areaId: vectors.lobby.primaryLobbyId },
      ],
    });
    const before = structuredClone(source);
    const canonical = canonicalMapDraft(source);
    expect(canonical.floor).toEqual([
      { y: 0, start: 0, end: 3, areaId: vectors.lobby.primaryLobbyId },
      { y: 2, start: 4, end: 5, areaId: null },
    ]);
    expect(canonicalMapDraft(canonical)).toEqual(canonical);
    expect(source).toEqual(before);
    expect(() =>
      canonicalMapDraft({
        ...source,
        floor: [...source.floor, { y: 0, start: 0, end: 2, areaId: null }],
      })
    ).toThrow(/Overlapping/);
  });
});
