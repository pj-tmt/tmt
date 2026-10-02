import { mapGeometry } from './map-source.js';
import { describe, expect, it } from 'vite-plus/test';
import maps from '../../../../../contracts/map-v1-vectors.json';
import vectors from '../../../../../contracts/world-v1-vectors.json';
import { builtinFurniture, validFurniture } from '../blocks/block-contract.js';
import { decodeWorldDocument, WORLD_LIMITS } from './world-contract.js';
import { removeWorldObject, updateWorldMap, worldHistory } from './world-draft.js';

const fixture = () => ({
  version: 1,
  map: structuredClone(maps.lobby),
  objects: [
    {
      id: '30000000-0000-4000-8000-000000000001',
      kind: 'decoration',
      placement: { ...builtinFurniture('chair', 0, 0, 0), customization: { tint: '#bb7755' } },
      surface: { type: 'floor' },
      extension: null as unknown,
    },
  ],
});

describe('whole-world draft contract', () => {
  it('shares native budgets and binding vectors without treating draft decoding as save authority', () => {
    expect(WORLD_LIMITS).toEqual(vectors.limits);
    for (const entry of vectors.attachments) {
      const source = fixture();
      source.objects[0]!.extension = entry.value;
      if (entry.draftValid) expect(decodeWorldDocument(source)).toEqual(source);
      else expect(() => decodeWorldDocument(source), entry.name).toThrow();
    }
  });

  it('owns nested values and preserves invalidated objects through topology edits and one undo history', () => {
    const source = fixture();
    source.objects[0]!.extension = {
      definition: 'tmt-whiteboard',
      binding: { kind: 'whiteboard', documentId: 'lobby' },
    };
    const history = worldHistory.create(decodeWorldDocument(source));
    source.objects[0]!.placement.customization.tint = '#ffffff';
    source.map.floor[0]!.end = 1;
    const initial = worldHistory.current(history);
    expect(initial.objects[0]!.placement.customization?.tint).toBe('#bb7755');
    expect(mapGeometry(initial.map).floor[0]!.end).toBe(3);
    const cut = updateWorldMap(initial, { ...mapGeometry(initial.map), floor: [] });
    expect(cut.objects).toEqual(initial.objects);
    const edited = worldHistory.commit(history, cut);
    expect(worldHistory.current(worldHistory.undo(edited))).toEqual(initial);
    expect(worldHistory.current(worldHistory.redo(worldHistory.undo(edited)))).toEqual(cut);
    const removed = removeWorldObject(cut, cut.objects[0]!.id);
    expect(removed.objects).toEqual([]);
    expect(removed.map).toEqual(cut.map);
    expect(worldHistory.current(worldHistory.undo(worldHistory.commit(edited, removed)))).toEqual(
      cut
    );
  });

  it('rejects executable/ambiguous shape while permitting signed placements and incomplete surface edits', () => {
    const source = fixture();
    source.objects[0]!.placement.x = -10;
    const world = decodeWorldDocument(source);
    expect(world.objects[0]!.placement.x).toBe(-10);
    expect(validFurniture(source.objects[0]!.placement)).toBe(false);
    expect(() =>
      decodeWorldDocument({ ...source, objects: [source.objects[0], source.objects[0]] })
    ).toThrow(/Duplicate/);
    expect(() => decodeWorldDocument({ ...source, script: 'run' })).toThrow();
    expect(() =>
      decodeWorldDocument({ ...source, objects: [{ ...source.objects[0], extension: undefined }] })
    ).toThrow();
    const { extension: _extension, ...missing } = source.objects[0]!;
    expect(() => decodeWorldDocument({ ...source, objects: [missing] })).toThrow();
    expect(() =>
      decodeWorldDocument({
        ...source,
        objects: [
          {
            ...source.objects[0],
            placement: { ...source.objects[0]!.placement, x: Number.MAX_SAFE_INTEGER },
          },
        ],
      })
    ).toThrow();
  });
});
