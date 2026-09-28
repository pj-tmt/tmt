import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { builtinFurniture } from '../blocks/block-contract.js';
import { startLocalRuntime } from './local-runtime.js';
import { PROFILE_CATALOG, ProfileConflict } from '../profiles/profile-contract.js';
import { BUILTIN_CATALOG, WORKSHOP_DIGEST } from '../props/prop-contract.js';
import { officeWorldFixture } from '../../../../../../../typescript/test/support/office-world.js';

const token = 'a'.repeat(43);
const identityId = '22222222-2222-4222-8222-222222222222';

describe('local Office runtime', () => {
  beforeEach(() => history.replaceState(null, '', `/local#token=${token}`));
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it('moves the fragment token to memory and authenticates world reads', async () => {
    const world = officeWorldFixture();
    const fetch = vi.fn(async () => new Response(JSON.stringify(world)));
    vi.stubGlobal('fetch', fetch);
    const runtime = startLocalRuntime(window.location);
    expect(location.hash).toBe('');
    await expect(runtime.world.show()).resolves.toEqual(world);
    expect(fetch).toHaveBeenCalledWith(
      '/api/v1/local/world',
      expect.objectContaining({ headers: { Authorization: `Bearer ${token}` } })
    );
    runtime.dispose();
  });

  it('rejects missing sessions before fetching', () => {
    history.replaceState(null, '', '/local');
    const fetch = vi.fn();
    vi.stubGlobal('fetch', fetch);
    expect(() => startLocalRuntime(window.location)).toThrow('session is missing');
    expect(fetch).not.toHaveBeenCalled();
  });

  it('resolves shared room props once in batches of at most sixteen unique digests', async () => {
    const digests = Array.from(
      { length: 18 },
      (_, index) => `sha256:${index.toString(16).padStart(64, '0')}`
    );
    const fetch = vi.fn(
      async (_path: string, init?: RequestInit) =>
        new Response(
          JSON.stringify({
            catalogRevision: 0,
            packs: [],
            unavailable: JSON.parse(String(init?.body)).digests,
          })
        )
    );
    vi.stubGlobal('fetch', fetch);
    const runtime = startLocalRuntime(window.location);
    const props = [...digests, digests[0]!].map((digest) => ({
      ...builtinFurniture('desk', 1, 1, 0),
      prop: `${digest}/desk`,
    }));
    const catalog = await runtime.resolveProps([
      ...props,
      builtinFurniture('desk', 1, 1, 0),
      {
        prop: `${WORKSHOP_DIGEST}/oak-desk`,
        footprint: { width: 12, height: 8 },
        x: 1,
        y: 1,
        rotation: 0,
      },
    ]);
    expect(catalog).toEqual(BUILTIN_CATALOG);
    expect(fetch).toHaveBeenCalledTimes(2);
    expect(
      fetch.mock.calls.map(([url, init]) => [url, JSON.parse(String(init?.body)).digests])
    ).toEqual([
      ['/api/v1/local/props/resolve', digests.slice(0, 16)],
      ['/api/v1/local/props/resolve', digests.slice(16)],
    ]);
    runtime.dispose();
  });

  it('uses the same in-memory browser credential for typed board operations', async () => {
    const operationId = '33333333-3333-4333-8333-333333333333';
    const fetch = vi.fn(async (path: string, _init?: RequestInit) => {
      if (path.endsWith('/categories/list'))
        return new Response(
          JSON.stringify({
            categories: [{ kind: 'general' }],
            nextCursor: null,
            boardRevision: 0,
          }),
          { status: 200 }
        );
      if (path.endsWith('/threads/list'))
        return new Response(JSON.stringify({ threads: [], nextCursor: null, boardRevision: 0 }), {
          status: 200,
        });
      return new Response(
        JSON.stringify({
          entryId: '11111111-1111-4111-8111-111111111111',
          threadId: '11111111-1111-4111-8111-111111111111',
          revision: 1,
          created: true,
          operationId,
        }),
        { status: 200 }
      );
    });
    vi.stubGlobal('fetch', fetch);
    const runtime = startLocalRuntime(window.location);
    await expect(runtime.board.categories()).resolves.toMatchObject({ boardRevision: 0 });
    await runtime.board.list({
      category: { kind: 'general' },
      author: { kind: 'identity', identityId: '22222222-2222-4222-8222-222222222222' },
    });
    await runtime.board.post({
      category: { kind: 'general' },
      title: 'Status',
      body: 'Plain text',
      operationId,
    });
    expect(fetch).toHaveBeenNthCalledWith(
      1,
      '/api/v1/local/board/categories/list',
      expect.objectContaining({
        method: 'POST',
        headers: {
          Authorization: `Bearer ${token}`,
          'Content-Type': 'application/json',
        },
      })
    );
    const list = fetch.mock.calls[1];
    expect(list?.[0]).toBe('/api/v1/local/board/threads/list');
    expect(JSON.parse(String(list?.[1]?.body)).author).toEqual({
      kind: 'identity',
      identityId: '22222222-2222-4222-8222-222222222222',
    });
    const post = fetch.mock.calls[2];
    expect(post?.[0]).toBe('/api/v1/local/board/threads');
    expect(JSON.parse(String(post?.[1]?.body))).toEqual({
      category: { kind: 'general' },
      title: 'Status',
      body: 'Plain text',
      operationId,
    });
    expect(JSON.parse(String(post?.[1]?.body))).not.toHaveProperty('actor');
    runtime.dispose();
  });

  it('reads and conditionally writes typed local profiles with the same browser credential', async () => {
    const snapshot = {
      identityId: identityId,
      identityName: 'Alice',
      exists: false,
      revision: 0,
      profile: {
        displayLabel: '',
        description: '',
        appearance: {
          hairStyle: 'short',
          hairColor: 'ink',
          skinTone: 'medium',
          shirtColor: 'blue',
          shirtMark: 'AI',
        },
      },
      updatedAtMs: null,
      catalog: PROFILE_CATALOG,
    } as const;
    const fetch = vi.fn(async (path: string, init?: RequestInit) => {
      if (init?.method === 'PUT')
        return new Response('{"error":"REVISION_CONFLICT"}', { status: 409 });
      return new Response(
        JSON.stringify(
          path.endsWith('/profiles')
            ? [
                {
                  ...snapshot,
                  presence: 'offline' as const,
                  lifetime: 'saved',
                  selfReportedStatus: null,
                },
              ]
            : snapshot
        ),
        { status: 200 }
      );
    });
    vi.stubGlobal('fetch', fetch);
    const runtime = startLocalRuntime(window.location);
    await expect(runtime.profiles.list()).resolves.toEqual([
      { ...snapshot, presence: 'offline' as const, lifetime: 'saved', selfReportedStatus: null },
    ]);
    await expect(runtime.profiles.show(identityId)).resolves.toEqual(snapshot);
    await expect(runtime.profiles.apply(identityId, 0, snapshot.profile)).rejects.toBeInstanceOf(
      ProfileConflict
    );
    expect(fetch.mock.calls[2]?.[0]).toBe(`/api/v1/local/profiles/${identityId}`);
    expect(JSON.parse(String(fetch.mock.calls[2]?.[1]?.body))).toEqual({
      expectedRevision: 0,
      profile: snapshot.profile,
    });
    runtime.dispose();
  });

  it('loads the bounded avatar catalog once through the authenticated runtime port', async () => {
    const value = { catalogRevision: 0, packs: [] };
    const fetch = vi.fn(async () => new Response(JSON.stringify(value), { status: 200 }));
    vi.stubGlobal('fetch', fetch);
    const runtime = startLocalRuntime(window.location);
    await expect(runtime.avatars.list()).resolves.toEqual(value);
    expect(fetch).toHaveBeenCalledOnce();
    expect(fetch).toHaveBeenCalledWith(
      '/api/v1/local/avatar-catalog',
      expect.objectContaining({ headers: { Authorization: `Bearer ${token}` } })
    );
    runtime.dispose();
  });

  it('distinguishes an unavailable avatar from a profile revision conflict', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => new Response('{"error":"AVATAR_UNAVAILABLE"}', { status: 409 }))
    );
    const runtime = startLocalRuntime(window.location);
    const profile = {
      displayLabel: '',
      description: '',
      appearance: {
        hairStyle: 'short' as const,
        hairColor: 'ink' as const,
        skinTone: 'medium' as const,
        shirtColor: 'blue' as const,
        shirtMark: '',
      },
    };
    await expect(runtime.profiles.apply(identityId, 0, profile)).rejects.toThrow(
      'selected avatar is no longer installed'
    );
    runtime.dispose();
  });
});
