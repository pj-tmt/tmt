import { mapGeometry } from '../world-map/map-source.js';
import { expect, it } from 'vite-plus/test';
import {
  officeWorldFixture,
  WORLD_LOBBY_ID,
} from '../../../../../../../typescript/test/support/office-world.js';
import { PROFILE_CATALOG } from '../profiles/profile-contract.js';
import type { ProfileProjection } from '../profiles/profile-contract.js';
import { officePopulation } from './office-population.js';
import { officeSceneModel } from './office-scene-model.js';
import type { WorldDocument } from '../world-map/world-contract.js';

function profile(
  identityId: string,
  lifetime: 'saved' | 'temporary' = 'saved',
  online = true
): ProfileProjection {
  return {
    identityId,
    identityName: identityId,
    lifetime,
    presence: online ? 'active' : 'offline',
    selfReportedStatus: null,
    exists: false,
    revision: 0,
    updatedAtMs: null,
    catalog: PROFILE_CATALOG,
    profile: {
      displayLabel: '',
      description: '',
      appearance: {
        hairStyle: 'short',
        hairColor: 'ink',
        skinTone: 'medium',
        shirtColor: 'blue',
        shirtMark: '',
      },
    },
  };
}
function fixture() {
  const initial = officeWorldFixture().layout;
  const world: WorldDocument = {
    ...initial,
    map: {
      ...initial.map,
      areas: [
        ...mapGeometry(initial.map).areas,
        { id: 'personal', name: 'Office', binding: { type: 'personal', identityId: 'alice' } },
        { id: 'meeting-a', name: 'Room A', binding: { type: 'meeting', roomId: 'room-a' } },
        { id: 'meeting-b', name: 'Room B', binding: { type: 'meeting', roomId: 'room-b' } },
      ],
    },
  };
  const profiles = [
    profile('alice'),
    profile('contractor', 'temporary'),
    profile('offline', 'saved', false),
  ];
  const rooms = [
    {
      id: 'room-a',
      name: 'Review',
      revision: 2,
      retired: false,
      memberIds: ['alice', 'contractor', 'offline', 'unavailable'],
    },
    { id: 'room-b', name: 'Planning', revision: 5, retired: false, memberIds: ['alice'] },
  ];
  return { world, profiles, rooms };
}

it('uses one expiring status across home and meetings, while conversation cues replace it without changing data', () => {
  const { world, profiles, rooms } = fixture();
  profiles[0]!.selfReportedStatus = {
    activity: 'Reviewing',
    mood: 'focused',
    updatedAtMs: 1000,
    expiresAtMs: 2000,
    stale: false,
  };
  profiles[1]!.selfReportedStatus = {
    activity: 'Testing',
    mood: null,
    updatedAtMs: 1000,
    expiresAtMs: 3000,
    stale: false,
  };
  const original = structuredClone({ world, profiles, rooms });
  const population = officePopulation(world.map, profiles, rooms);
  const scene = (nowMs: number, conversation?: string) =>
    officeSceneModel(
      population,
      world,
      { catalogRevision: 0, packs: [] },
      [],
      [],
      nowMs,
      conversation
    );
  expect(
    scene(1999)
      .actors.filter((actor) => actor.identityId === 'alice')
      .map((actor) => actor.activity)
  ).toEqual(['focused · Reviewing', 'focused · Reviewing', 'focused · Reviewing']);
  expect(
    scene(1999, 'alice')
      .actors.filter((actor) => actor.identityId === 'alice')
      .every((actor) => actor.activity === undefined)
  ).toBe(true);
  expect(
    scene(2000)
      .actors.filter((actor) => actor.identityId === 'alice')
      .every((actor) => actor.activity === undefined)
  ).toBe(true);
  expect(
    scene(2000)
      .actors.filter((actor) => actor.identityId === 'contractor')
      .map((actor) => actor.activity)
  ).toEqual(['Testing', 'Testing']);
  expect({ world, profiles, rooms }).toEqual(original);
});

it('projects one identity into its home and multiple meetings without cloning profile or membership', () => {
  const { world, profiles, rooms } = fixture();
  const before = structuredClone({ world, profiles, rooms });
  const population = officePopulation(world.map, profiles, rooms);
  expect(population.homes.get('alice')).toBe('personal');
  expect(population.homes.get('contractor')).toBe(WORLD_LOBBY_ID);
  expect(population.areas.get('meeting-a')!.room).toBe(rooms[0]);
  expect(population.areas.get('meeting-a')!.members).toHaveLength(4);
  expect(
    population.areas.get('meeting-a')!.members.find((member) => member.identityId === 'offline')
      ?.profile?.presence
  ).toBe('offline');
  expect(
    population.areas.get('meeting-a')!.members.find((member) => member.identityId === 'unavailable')
      ?.profile
  ).toBeUndefined();
  const scene = officeSceneModel(
    population,
    world,
    { catalogRevision: 0, packs: [] },
    [],
    [],
    1000
  );
  const alice = scene.actors.filter((actor) => actor.identityId === 'alice');
  expect(alice.map((actor) => actor.areaId)).toEqual(['personal', 'meeting-a', 'meeting-b']);
  expect(alice.every((actor) => actor.avatar === alice[0]!.avatar)).toBe(true);
  expect(scene.actors.filter((actor) => actor.contractor).map((actor) => actor.areaId)).toEqual([
    WORLD_LOBBY_ID,
    'meeting-a',
  ]);
  expect(scene.actors.some((actor) => ['offline', 'unavailable'].includes(actor.identityId))).toBe(
    false
  );
  expect({ world, profiles, rooms }).toEqual(before);
});

it('retains unknown identities and their homes without rendering them as online actors', () => {
  const { world, profiles, rooms } = fixture();
  profiles[0]!.presence = 'unknown';
  const population = officePopulation(world.map, profiles, rooms);
  expect(population.identities.get('alice')?.presence).toBe('unknown');
  expect(population.homes.get('alice')).toBe('personal');
  expect(
    population.areas.get('meeting-a')!.members.some((member) => member.identityId === 'alice')
  ).toBe(true);
  const scene = officeSceneModel(
    population,
    world,
    { catalogRevision: 0, packs: [] },
    [],
    [],
    1000
  );
  expect(scene.actors.some((actor) => actor.identityId === 'alice')).toBe(false);
  expect(scene.actors.some((actor) => actor.identityId === 'contractor')).toBe(true);
});

it('does not infer membership from area names and preserves homes when a room is empty or unavailable', () => {
  const { world, profiles, rooms } = fixture();
  rooms[0]!.memberIds = [];
  const population = officePopulation(world.map, profiles, rooms.slice(0, 1));
  expect(population.areas.get('meeting-a')!.members).toEqual([]);
  expect(population.areas.get('meeting-b')!.members).toEqual([]);
  expect(population.areas.get('meeting-b')!.room).toBeUndefined();
  expect(population.homes.get('alice')).toBe('personal');
});

it('treats promotion, offline presence and spatial detachment independently of canonical room members', () => {
  const { world, profiles, rooms } = fixture();
  const map = {
    ...mapGeometry(world.map),
    areas: mapGeometry(world.map).areas.map((area) =>
      area.id === 'personal'
        ? { ...area, binding: { type: 'personal' as const, identityId: 'contractor' } }
        : area
    ),
  };
  expect(officePopulation(map, profiles, rooms).homes.get('contractor')).toBe(WORLD_LOBBY_ID);
  profiles[1]!.lifetime = 'saved';
  profiles[1]!.presence = 'offline';
  expect(officePopulation(map, profiles, rooms).homes.get('contractor')).toBe('personal');
  const detached = {
    ...map,
    areas: map.areas.filter((area) => !['personal', 'meeting-a'].includes(area.id)),
  };
  const population = officePopulation(detached, profiles, rooms);
  expect(population.homes.get('contractor')).toBe(WORLD_LOBBY_ID);
  expect(population.areas.get('meeting-b')!.members[0]!.profile).toBe(profiles[0]);
  expect(rooms[0]!.memberIds).toEqual(['alice', 'contractor', 'offline', 'unavailable']);
});
