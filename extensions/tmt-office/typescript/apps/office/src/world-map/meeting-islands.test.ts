import { expect, it } from 'vite-plus/test';
import vectors from '../../../../../contracts/modules-island-vectors.json';
import { decodeMapSource, mapGeometry } from './map-source.js';
import { moduleBounds, meetingCirculation } from './module-geometry.js';
import { nextMeetingSlot, meetingExpansionPassages } from './meeting-module.js';

it('projects the shared independent-island vectors without meeting bridges or door ends', () => {
  const source = decodeMapSource(vectors.map);
  if (source.version === 1) throw new Error('Expected modules');
  const map = mapGeometry(source);
  const at = (x: number, y: number) =>
    map.floor.find((row) => row.y === y && row.start <= x && x < row.end);
  for (const [x, y] of vectors.publicSamples) expect(at(x!, y!)?.areaId).toBe(null);
  for (const [x, y] of vectors.emptySamples) expect(at(x!, y!)).toBeUndefined();
  expect(map.floor.reduce((sum, row) => sum + row.end - row.start, 0)).toBe(vectors.floorTiles);
  expect(map.doors).toHaveLength(vectors.doorEdges);
  expect(
    source.modules
      .filter((module) => module.slot.type === 'meeting')
      .map((module) => moduleBounds(module, 7))
  ).toEqual(vectors.meetingBounds);
  expect(
    meetingCirculation(
      source.modules.map((module) => module.slot),
      7
    )
  ).toEqual({ passages: [], openings: [] });
  expect(nextMeetingSlot(source)).toEqual({ type: 'meeting', index: 3 });
  expect(meetingExpansionPassages(source, nextMeetingSlot(source)!)).toEqual([]);
  const legacy = decodeMapSource({ ...vectors.map, version: 6 });
  expect(mapGeometry(legacy).floor.length).toBeGreaterThan(map.floor.length);
  expect(mapGeometry(source)).toBe(map);
});
