import { expect, it } from 'vite-plus/test';
import central from '../../../../../contracts/modules-central-grid-vectors.json';
import { officeWorldFixture } from '../../../../../../../typescript/test/support/office-world.js';
import { decodeModuleMap } from './module-contract.js';
import { mapGeometry } from './map-source.js';
import { moduleBounds } from './module-geometry.js';
import { compactModuleWorld } from './module-upgrade.js';
import { worldHistory } from './world-draft.js';

it('projects compact rooms without changing the retained v4 representation', () => {
  const old = decodeModuleMap(central.map);
  const map = { ...old, version: 5 as const };
  const before = structuredClone(old);
  const geometry = mapGeometry(map);
  expect(geometry.floor.length).toBeGreaterThan(0);
  expect(old).toEqual(before);
  const meeting = old.modules.find((module) => module.slot.type === 'meeting');
  if (!meeting) throw new Error('Fixture requires a meeting module.');
  const first = { ...meeting, slot: { type: 'meeting' as const, index: 0 } };
  const second = { ...meeting, slot: { type: 'meeting' as const, index: 1 } };
  expect(moduleBounds(first, 5).y + moduleBounds(first, 5).height).toBe(moduleBounds(second, 5).y);
  expect(moduleBounds(second, 4).y).toBe(48);
});

it('converts room-owned objects in one reversible draft and retains their identities', () => {
  const map = decodeModuleMap(central.map);
  const fixture = officeWorldFixture().layout;
  const source = fixture.objects[0]!;
  const floor = { ...source, placement: { ...source.placement, x: 124, y: 148 } };
  const wall = {
    ...source,
    id: '30000000-0000-4000-8000-000000000009',
    surface: {
      type: 'wall' as const,
      axis: 'horizontal' as const,
      face: 'negative' as const,
      elevation: 0,
    },
    placement: { ...source.placement, x: 128, y: 184 },
  };
  const world = { ...fixture, map, objects: [floor, wall] };
  const baseline = structuredClone(world);
  const candidate = compactModuleWorld(world);
  expect(candidate.map.version).toBe(5);
  expect(world).toEqual(baseline);
  expect(candidate.objects).toEqual([
    { ...floor, placement: { ...floor.placement, y: 124 } },
    { ...wall, placement: { ...wall.placement, y: 160 } },
  ]);
  const history = worldHistory.commit(worldHistory.create(world), candidate);
  expect(worldHistory.current(worldHistory.undo(history))).toEqual(world);
  expect(compactModuleWorld(candidate)).toBe(candidate);
});
