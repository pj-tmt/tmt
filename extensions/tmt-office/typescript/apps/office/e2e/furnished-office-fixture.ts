import type { Page } from '@playwright/test';
import lobbyPreset from '../../../../../../contracts/office/lobby-preset-v1.json' with { type: 'json' };
import { workshopStarter } from '../src/blocks/workshop-starter.js';
import { PROFILE_CATALOG } from '../src/profiles/profile-contract.js';
import type { ProfileProjection } from '../src/profiles/profile-contract.js';
import {
  officeWorldFixture,
  WORLD_LOBBY_ID,
} from '../../../../../../typescript/test/support/office-world.js';
import type { WorldObject } from '../src/world-map/world-contract.js';
import definition from '../../../../../../contracts/office/discussion-extension-v1.json' with { type: 'json' };
import whiteboard from '../../../../../../contracts/office/whiteboard-extension-v1.json' with { type: 'json' };
import broadcaster from '../../../../../../contracts/office/broadcaster-extension-v1.json' with { type: 'json' };

/** Isolated HTTP state; router, draft controller and GPU remain real. Not native admission evidence. */
export async function furnishedOfficeFixture(page: Page) {
  const token = 'f'.repeat(43);
  const people = [
    {
      name: 'Alice',
      color: 'blue',
      shell: 'light',
      hair: 'bald',
      mark: 'AI',
      tint: '#b8d9b2',
      presence: 'active' as const,
      style: 'study',
    },
    {
      name: 'Bob',
      color: 'green',
      shell: 'medium',
      hair: 'short',
      mark: 'OPS',
      tint: '#edd49c',
      presence: 'active' as const,
      style: 'library',
    },
    {
      name: 'Casey',
      color: 'plum',
      shell: 'warm',
      hair: 'tied',
      mark: 'QA',
      tint: '#aecfe5',
      presence: 'offline' as const,
      style: 'studio',
    },
  ] as const;
  const profiles: ProfileProjection[] = people.map((person, index) => ({
    identityId: `22222222-2222-4222-8222-${String(index + 1).padStart(12, '0')}`,
    identityName: person.name,
    exists: true,
    revision: 1,
    updatedAtMs: 1,
    catalog: PROFILE_CATALOG,
    presence: person.presence,
    selfReportedStatus: null,
    lifetime: 'saved',
    profile: {
      displayLabel: 'Personal workspace',
      description: 'A place for focused work and shared ideas.',
      appearance: {
        hairStyle: person.hair,
        hairColor: 'silver',
        skinTone: person.shell,
        shirtColor: person.color,
        shirtMark: person.mark,
      },
    },
  }));
  const layouts = [
    structuredClone(lobbyPreset.objects),
    ...people.map((person) =>
      workshopStarter(person.style).map((item) =>
        item.prop.endsWith('/woven-rug') ? { ...item, customization: { tint: person.tint } } : item
      )
    ),
  ];
  let world = officeWorldFixture();
  const areas = profiles.slice(0, 2).map((profile, index) => ({
    id: `10000000-0000-4000-8000-${String(index + 2).padStart(12, '0')}`,
    name: index === 0 ? 'Studio' : 'Operations',
    binding: { type: 'personal' as const, identityId: profile.identityId },
  }));
  const objects: WorldObject[] = [];
  function add(placement: WorldObject['placement'], extension: WorldObject['extension'] = null) {
    objects.push({
      id: `30000000-0000-4000-8000-${String(objects.length + 1).padStart(12, '0')}`,
      kind: 'decoration',
      surface: { type: 'floor' },
      placement,
      extension,
    });
  }
  layouts[0]!.forEach((item) => add({ ...item, x: item.x + 38, y: item.y + 42 }));
  for (const [index, layout] of layouts.slice(1, 3).entries())
    layout.forEach((item) => add({ ...item, x: item.x + index * 36 + 2, y: item.y + 2 }));
  for (const [index, resource] of [definition, whiteboard, broadcaster].entries())
    add(
      { ...resource.appearance, x: [3, 17, 6][index]!, y: [45, 45, 60][index]!, rotation: 0 },
      {
        definition: resource.id,
        binding:
          index === 0
            ? { kind: 'office-board' }
            : index === 1
              ? { kind: 'whiteboard', documentId: 'lobby' }
              : { kind: 'office-broadcast' },
      }
    );
  world = {
    ...world,
    layout: {
      version: 1,
      objects,
      map: {
        ...world.layout.map,
        areas: [...world.layout.map.areas, ...areas],
        floor: [
          ...Array.from({ length: 36 }, (_, y) =>
            areas.map((area, i) => ({ y, start: i * 36, end: (i + 1) * 36, areaId: area.id }))
          ).flat(),
          ...Array.from({ length: 4 }, (_, i) => ({ y: i + 36, start: 0, end: 72, areaId: null })),
          ...Array.from({ length: 36 }, (_, i) => ({
            y: i + 40,
            start: 0,
            end: 72,
            areaId: WORLD_LOBBY_ID,
          })),
        ],
        doors: [14, 15, 16, 17, 50, 51, 52, 53]
          .map((x) => ({ x, y: 36, axis: 'horizontal' as const }))
          .concat([32, 33, 34, 35].map((x) => ({ x, y: 40, axis: 'horizontal' as const }))),
      },
    },
  };
  const writes: unknown[] = [];
  const profileWrites: unknown[] = [];
  const responses = new Map<string, unknown>([
    ['/api/v1/local/profiles', profiles],
    ['/api/v1/local/rooms', []],
    ['/api/v1/local/avatar-catalog', { catalogRevision: 0, packs: [] }],
    [
      '/api/v1/local/whiteboards/lobby',
      {
        id: 'lobby',
        revision: 0,
        updatedAtMs: 0,
        scene: { formatVersion: 1, width: 1600, height: 1000, background: '#fff7e7', elements: [] },
      },
    ],
  ]);
  const unexpected: string[] = [];
  const reads: string[] = [];
  const completedReads: string[] = [];
  const failedReads: { path: string; error: string | undefined }[] = [];
  page.on('requestfinished', (request) => {
    if (request.method() === 'GET') completedReads.push(new URL(request.url()).pathname);
  });
  page.on('requestfailed', (request) => {
    if (request.method() === 'GET')
      failedReads.push({
        path: new URL(request.url()).pathname,
        error: request.failure()?.errorText,
      });
  });
  await page.route('**/api/v1/local/**', async (route) => {
    const request = route.request();
    const pathname = new URL(request.url()).pathname;
    if (request.method() === 'GET') reads.push(pathname);
    const profileIndex = profiles.findIndex(
      (profile) => pathname === `/api/v1/local/profiles/${profile.identityId}`
    );
    if (profileIndex >= 0 && request.headers().authorization === `Bearer ${token}`) {
      const current = profiles[profileIndex]!;
      if (request.method() === 'PUT') {
        if (request.headers().origin !== 'http://127.0.0.1:4176')
          throw new Error('Unexpected fixture write origin.');
        const input = request.postDataJSON();
        profileWrites.push(input);
        if (input.expectedRevision !== current.revision) {
          await route.fulfill({ status: 409, json: { error: 'REVISION_CONFLICT' } });
          return;
        }
        profiles[profileIndex] = {
          ...current,
          profile: input.profile,
          revision: current.revision + 1,
          updatedAtMs: current.updatedAtMs! + 1,
        };
      } else if (request.method() !== 'GET') {
        unexpected.push(`${request.method()} ${pathname}`);
        await route.fulfill({ status: 405 });
        return;
      }
      const {
        presence: _presence,
        lifetime: _lifetime,
        selfReportedStatus: _status,
        ...snapshot
      } = profiles[profileIndex]!;
      await route.fulfill({
        json: { ...snapshot, ...(request.method() === 'PUT' ? { changed: true } : {}) },
      });
      return;
    }
    if (
      pathname === '/api/v1/local/world' &&
      request.headers().authorization === `Bearer ${token}`
    ) {
      if (request.method() === 'GET') {
        await route.fulfill({ json: { ...world, changed: false } });
        return;
      }
      if (request.method() === 'PUT') {
        const input = request.postDataJSON();
        writes.push(input);
        if (input.expectedRevision !== world.revision) {
          await route.fulfill({ status: 409, json: { error: 'WORLD_REVISION_CONFLICT' } });
          return;
        }
        world = {
          ...world,
          layout: input.layout,
          revision: world.revision + 1,
          updatedAtMs: world.updatedAtMs + 1,
          changed: true,
        };
        await route.fulfill({ json: world });
        return;
      }
    }
    if (
      request.method() !== 'GET' ||
      request.headers().authorization !== `Bearer ${token}` ||
      !responses.has(pathname)
    ) {
      unexpected.push(`${request.method()} ${pathname}`);
      await route.fulfill({ status: 400, json: { error: 'UNEXPECTED_FIXTURE_REQUEST' } });
      return;
    }
    await route.fulfill({ json: responses.get(pathname) });
  });
  return {
    url: `http://127.0.0.1:4176/local#token=${token}`,
    unexpected,
    writes,
    profileWrites,
    profiles,
    reads,
    completedReads,
    failedReads,
    read: () => world,
  };
}
