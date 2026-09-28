import { mapGeometry } from '../world-map/map-source.js';
import { createMemoryHistory } from '@tanstack/react-router';
import {
  act,
  fireEvent,
  render,
  renderHook,
  screen,
  waitFor,
  within,
} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import {
  officeWorldFixture,
  WORLD_LOBBY_ID,
} from '../../../../../../../typescript/test/support/office-world.js';
import { defaultCatalog } from '../blocks/block-contract.js';
import { OfficeApp } from '../office-app.js';
import { createOfficeRouter } from '../router.js';
import type { LocalRuntime } from './local-runtime.js';
import { LocalRuntimeContext } from './local-runtime.js';
import { useLocalOffice } from './use-local-office.js';
import { PROFILE_CATALOG } from '../profiles/profile-contract.js';
import type { ProfileProjection } from '../profiles/profile-contract.js';
import type { OfficeSceneModel, OfficeSceneEditor } from '../rendering/office-scene.js';
import { WorldConflict, WorldValidationError } from '../world-map/world-port.js';
import type { WorldSnapshot } from '../world-map/world-port.js';
import type { OfficeSelection } from '../rendering/office-selection.js';
import type { ReactNode } from 'react';

const canvas = vi.hoisted(() => ({
  model: undefined as OfficeSceneModel | undefined,
  editor: undefined as OfficeSceneEditor | undefined,
  select: undefined as ((selection: OfficeSelection) => void) | undefined,
}));
// Contract/controller tests inspect the projection; GPU and pointer capture are browser-tested separately.
vi.mock('./office-canvas.js', () => ({
  OfficeCanvas: ({
    model,
    editor,
    select,
    agentOverlay,
    officeOverlay,
  }: {
    model: OfficeSceneModel;
    editor?: OfficeSceneEditor;
    select: (selection: OfficeSelection) => void;
    agentOverlay?: () => ReactNode;
    officeOverlay?: () => ReactNode;
  }) => {
    canvas.model = model;
    canvas.editor = editor;
    canvas.select = select;
    return (
      <>
        {agentOverlay?.()}
        {officeOverlay?.()}
      </>
    );
  },
}));
const identityId = '22222222-2222-4222-8222-222222222222';
const profile: ProfileProjection = {
  identityId,
  identityName: 'Alice',
  exists: true,
  revision: 1,
  lifetime: 'saved',
  presence: 'active' as const,
  selfReportedStatus: null,
  profile: {
    displayLabel: 'Signal lead',
    description: 'Reviews architecture.',
    appearance: {
      hairStyle: 'short',
      hairColor: 'ink',
      skinTone: 'medium',
      shirtColor: 'blue',
      shirtMark: 'AI',
    },
  },
  updatedAtMs: 1,
  catalog: PROFILE_CATALOG,
};
function runtime(profiles = [profile]): LocalRuntime {
  let world = officeWorldFixture();
  const unused = async (): Promise<never> => {
    throw new Error('Not used by this scenario');
  };
  return {
    world: {
      show: vi.fn(async () => structuredClone(world)),
      save: vi.fn(async (input) => {
        if (input.expectedRevision !== world.revision) throw new Error('Conflict');
        world = {
          ...world,
          revision: world.revision + 1,
          layout: structuredClone(input.layout),
          updatedAtMs: world.updatedAtMs + 1,
          changed: true,
        };
        return world;
      }),
    },
    profiles: {
      list: vi.fn(async () => profiles),
      show: vi.fn(async () => profile),
      apply: vi.fn(async () => ({ ...profile, changed: false })),
    },
    avatars: { list: vi.fn(async () => ({ catalogRevision: 0, packs: [] })) },
    rooms: { list: vi.fn(async () => []), save: vi.fn(unused), retire: vi.fn() },
    resolveProps: vi.fn(async () => defaultCatalog()),
    whiteboards: { show: vi.fn(unused), save: vi.fn(unused) },
    notebooks: { read: vi.fn(unused) },
    propCatalog: { list: vi.fn(unused), load: vi.fn(unused), install: vi.fn(unused) },
    whiteboardSnapshots: {
      capture: vi.fn(unused),
      show: vi.fn(unused),
      attachImage: vi.fn(unused),
      image: vi.fn(unused),
    },
    dispatch: { send: vi.fn(unused) },
    requests: { list: vi.fn(unused), show: vi.fn(unused), receipt: vi.fn(unused) },
    board: {
      categories: unused,
      list: unused,
      show: unused,
      post: unused,
      reply: unused,
      edit: unused,
      delete: unused,
    },
    preview: unused,
    avatarPreview: unused,
    dispose: vi.fn(),
  };
}
async function show(local: LocalRuntime, path = '/local', ready = true) {
  const router = createOfficeRouter(createMemoryHistory({ initialEntries: [path] }));
  const view = render(<OfficeApp router={router} local={local} />);
  if (ready) await screen.findByRole('button', { name: 'Office menu' });
  return { ...view, router };
}
async function directory() {
  const menu = await screen.findByRole('button', { name: 'Office menu' });
  if (!screen.queryByRole('complementary', { name: 'Office directory' })) {
    if (menu.getAttribute('aria-expanded') !== 'true') await userEvent.click(menu);
    await userEvent.click(screen.getByRole('button', { name: /^Directory · / }));
  }
  return menu;
}

it.each(['saved', 'temporary'] as const)(
  'adds a %s identity from its Info panel through the canonical room editor only',
  async (lifetime) => {
    const local = runtime([{ ...profile, lifetime }]);
    const room = {
      id: '44444444-4444-4444-8444-444444444444',
      name: 'Review',
      revision: 3,
      retired: false,
      memberIds: [] as string[],
    };
    vi.mocked(local.rooms.list).mockResolvedValue([room]);
    vi.mocked(local.rooms.save).mockImplementation(async (_id, input) => ({
      ...room,
      revision: 4,
      memberIds: input.memberIds,
    }));
    await show(local);
    await directory();
    const original = structuredClone(canvas.model!.world);
    act(() => canvas.select!({ kind: 'agent', identityId, areaId: WORLD_LOBBY_ID }));
    await userEvent.click(screen.getByRole('tab', { name: 'Info' }));
    await userEvent.click(screen.getByRole('button', { name: 'Add to meeting…' }));
    await screen.findByRole('option', { name: 'Review' });
    await userEvent.selectOptions(screen.getByRole('combobox', { name: 'Meeting room' }), room.id);
    expect(local.rooms.save).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole('button', { name: 'Add Alice' }));
    expect(screen.getByRole('checkbox', { name: /Alice/ })).toHaveProperty('checked', true);
    await userEvent.click(screen.getByRole('button', { name: 'Save room' }));
    expect(local.rooms.save).toHaveBeenCalledExactlyOnceWith(
      room.id,
      { expectedRevision: 3, name: 'Review', memberIds: [identityId] },
      expect.any(AbortSignal)
    );
    expect(canvas.model!.world).toEqual(original);
    expect(local.world.save).not.toHaveBeenCalled();
    expect(local.profiles.apply).not.toHaveBeenCalled();
    expect(local.dispatch.send).not.toHaveBeenCalled();
    expect(local.rooms.retire).not.toHaveBeenCalled();
  }
);

it('keeps status independent of appearance saves and appearance revision ordering during refresh', async () => {
  const previous = {
    activity: 'Reviewing',
    mood: null,
    updatedAtMs: 1000,
    expiresAtMs: 2000,
    stale: false,
  };
  const renewed = { ...previous, activity: 'Testing', updatedAtMs: 1500, expiresAtMs: 2500 };
  const local = runtime([{ ...profile, selfReportedStatus: previous }]);
  const { result } = renderHook(() => useLocalOffice(), {
    wrapper: ({ children }) => (
      <LocalRuntimeContext.Provider value={local}>{children}</LocalRuntimeContext.Provider>
    ),
  });
  await waitFor(() => expect(result.current.load.status).toBe('ready'));
  let finish!: (profiles: ProfileProjection[]) => void;
  vi.mocked(local.profiles.list).mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      })
  );
  act(() => result.current.refresh());
  const {
    presence: _presence,
    lifetime: _lifetime,
    selfReportedStatus: _status,
    ...snapshot
  } = profile;
  act(() =>
    result.current.profileChanged({
      ...snapshot,
      revision: 2,
      profile: { ...snapshot.profile, description: 'New appearance' },
    })
  );
  expect(result.current.load).toMatchObject({
    profiles: [{ revision: 2, selfReportedStatus: previous }],
  });
  await act(async () => finish([{ ...profile, selfReportedStatus: renewed }]));
  expect(result.current.load).toMatchObject({
    profiles: [
      { revision: 2, profile: { description: 'New appearance' }, selfReportedStatus: renewed },
    ],
  });
  act(() => result.current.profileChanged({ ...snapshot, revision: 3 }));
  expect(result.current.load).toMatchObject({
    profiles: [{ revision: 3, selfReportedStatus: renewed }],
  });
  vi.mocked(local.profiles.list).mockResolvedValue([{ ...profile, selfReportedStatus: null }]);
  act(() => result.current.refresh());
  await waitFor(() =>
    expect(result.current.load).toMatchObject({
      refreshing: false,
      profiles: [{ revision: 3, selfReportedStatus: null }],
    })
  );
});

it('shows exact status and separate endpoint presence, and suppresses the ordinary cue while its HUD is open', async () => {
  const now = Date.now();
  const local = runtime([
    {
      ...profile,
      selfReportedStatus: {
        activity: 'Reviewing the room',
        mood: 'focused',
        updatedAtMs: now,
        expiresAtMs: now + 60000,
        stale: false,
      },
    },
  ]);
  await show(local);
  await waitFor(() =>
    expect(canvas.model?.actors[0]?.activity).toBe('focused · Reviewing the room')
  );
  act(() => canvas.select!({ kind: 'agent', identityId, areaId: WORLD_LOBBY_ID }));
  await userEvent.click(screen.getByRole('tab', { name: 'Info' }));
  const status = screen.getByRole('region', { name: 'Self-reported status' });
  expect(within(status).getByText('Reviewing the room')).toBeDefined();
  expect(within(status).getByText('Mood: focused')).toBeDefined();
  expect(within(status).getByText('Current')).toBeDefined();
  expect(status.querySelectorAll('time')).toHaveLength(2);
  expect(screen.getByText('Saved identity · Online')).toBeDefined();
  expect(canvas.model!.actors[0]!.activity).toBeUndefined();
  await userEvent.click(screen.getByRole('button', { name: 'Close agent conversation' }));
  expect(canvas.model!.actors[0]!.activity).toBe('focused · Reviewing the room');
});

it('loads the world without saving or dispatching work', async () => {
  const local = runtime();
  await show(local);
  const toggle = await screen.findByRole('button', { name: 'Office menu' });
  expect(toggle.getAttribute('aria-expanded')).toBe('false');
  expect(screen.queryByRole('button', { name: /^Directory · / })).toBeNull();
  expect(screen.queryByRole('button', { name: 'Meeting rooms' })).toBeNull();
  expect(screen.queryByRole('button', { name: 'Add meeting room' })).toBeNull();
  expect(screen.queryByRole('complementary', { name: 'Agent details' })).toBeNull();
  expect(screen.queryByRole('complementary', { name: 'Area details' })).toBeNull();
  expect(canvas.model!.world).toEqual(officeWorldFixture().layout);
  expect(local.world.save).not.toHaveBeenCalled();
  expect(local.dispatch.send).not.toHaveBeenCalled();
});

it('has no mode switch and selects room properties directly without writing', async () => {
  const local = runtime();
  await show(local);
  expect(screen.queryByRole('button', { name: 'Edit layout' })).toBeNull();
  expect(screen.queryByRole('button', { name: 'Save layout' })).toBeNull();
  expect(screen.getByRole('heading', { name: 'Furniture & devices' })).toBeDefined();
  act(() => canvas.select!({ kind: 'area', areaId: WORLD_LOBBY_ID }));
  expect(screen.getByLabelText('Area name')).toHaveProperty('value', 'Lobby');
  expect(screen.getByText('Keyboard selection', { exact: true }).closest('details')).toHaveProperty(
    'hidden',
    true
  );
  expect(screen.queryByRole('heading', { name: /^Lobby$/ })).toBeNull();
  await userEvent.click(screen.getByRole('button', { name: 'Clear selection' }));
  await userEvent.click(screen.getByText('Keyboard selection', { exact: true }));
  const object = canvas.model!.world.objects[0]!;
  await userEvent.selectOptions(screen.getByRole('combobox', { name: 'Object' }), object.id);
  expect(canvas.editor!.selected).toBe(object.id);
  expect(screen.getByRole('heading', { name: /^Selected object:/ })).toBeDefined();
  await userEvent.keyboard('{Escape}');
  expect(screen.getByRole('heading', { name: 'Furniture & devices' })).toBeDefined();
  expect(local.world.save).not.toHaveBeenCalled();
});

it('shares one right inspector across agent, object and floor selection without losing the chat draft', async () => {
  const local = runtime();
  local.requests.list = vi.fn(async () => ({ items: [], nextBefore: null }));
  local.requests.receipt = vi.fn(async () => null);
  await show(local);
  act(() => canvas.select!({ kind: 'agent', identityId }));
  const inspector = screen.getByRole('complementary', { name: 'Agent inspector' });
  expect(within(inspector).getByRole('dialog', { name: 'Agent conversation' })).toBeDefined();
  expect(screen.queryByRole('heading', { name: 'Furniture & devices' })).toBeNull();
  expect(screen.queryByRole('region', { name: 'Layout changes' })).toBeNull();
  const input = await screen.findByRole('textbox', { name: 'Message' });
  fireEvent.change(input, { target: { value: 'A retained draft' } });
  const header = inspector.querySelector('header')!;
  const historyReads = vi.mocked(local.requests.list).mock.calls.length;
  fireEvent.click(within(header).getByRole('button', { name: 'Refresh' }));
  await waitFor(() => expect(local.requests.list).toHaveBeenCalledTimes(historyReads + 1));
  fireEvent.click(within(header).getByRole('tab', { name: 'Info' }));
  const worldReads = vi.mocked(local.world.show).mock.calls.length;
  fireEvent.click(within(header).getByRole('button', { name: 'Refresh agent information' }));
  await waitFor(() => expect(local.world.show).toHaveBeenCalledTimes(worldReads + 1));
  fireEvent.click(within(header).getByRole('tab', { name: 'Chat' }));
  expect(input).toHaveProperty('value', 'A retained draft');
  const object = canvas.model!.world.objects[0]!;
  act(() => canvas.editor!.select!(object.id));
  expect(screen.queryByRole('dialog', { name: 'Agent conversation' })).toBeNull();
  expect(screen.getByRole('heading', { name: /^Selected object:/ })).toBeDefined();
  act(() => canvas.select!({ kind: 'agent', identityId }));
  expect(screen.getByRole('textbox', { name: 'Message' })).toBe(input);
  expect(input).toHaveProperty('value', 'A retained draft');
  act(() => canvas.select!({ kind: 'area', areaId: WORLD_LOBBY_ID }));
  expect(screen.getByLabelText('Area name')).toHaveProperty('value', 'Lobby');
  expect(screen.queryByRole('dialog', { name: 'Agent conversation' })).toBeNull();
  act(() => canvas.select!({ kind: 'agent', identityId }));
  fireEvent.keyDown(input, { key: 'Escape' });
  expect(screen.queryByRole('dialog', { name: 'Agent conversation' })).toBeNull();
  expect(screen.getByRole('heading', { name: 'Furniture & devices' })).toBeDefined();
  act(() => canvas.select!({ kind: 'agent', identityId }));
  expect(input).toHaveProperty('value', 'A retained draft');
  expect(local.world.save).not.toHaveBeenCalled();
  expect(local.dispatch.send).not.toHaveBeenCalled();
});

it('handles layout history keys from body without intercepting native input history', async () => {
  const local = runtime();
  const view = await show(local);
  const before = canvas.model!.world;
  act(() =>
    canvas.editor!.placeObject({
      ...before.objects[0]!,
      placement: { ...before.objects[0]!.placement, x: 8, y: 4 },
    })
  );
  expect(canvas.model!.world.objects[0]!.placement.x).toBe(8);
  fireEvent.keyDown(document.body, { key: 'z', ctrlKey: true });
  expect(canvas.model!.world).toEqual(before);
  fireEvent.keyDown(document.body, { key: 'z', ctrlKey: true, shiftKey: true });
  expect(canvas.model!.world.objects[0]!.placement.x).toBe(8);
  act(() => canvas.select!({ kind: 'area', areaId: WORLD_LOBBY_ID }));
  const input = screen.getByLabelText('Area name');
  expect(fireEvent.keyDown(input, { key: 'z', metaKey: true })).toBe(true);
  expect(canvas.model!.world.objects[0]!.placement.x).toBe(8);
  view.unmount();
  expect(fireEvent.keyDown(document.body, { key: 'z', ctrlKey: true })).toBe(true);
});

it('does not grow terrain for saved or temporary identities; Contractors stay in the Lobby', async () => {
  const local = runtime([
    profile,
    {
      ...profile,
      identityId: '33333333-3333-4333-8333-333333333333',
      identityName: 'Pip',
      lifetime: 'temporary',
    },
    {
      ...profile,
      identityId: '44444444-4444-4444-8444-444444444444',
      identityName: 'Offline',
      presence: 'offline' as const,
    },
  ]);
  await show(local);
  await directory();
  expect(canvas.model!.world.map).toEqual(officeWorldFixture().layout.map);
  expect(
    canvas.model!.actors.map((actor) => [actor.avatar.name, actor.areaId, actor.contractor])
  ).toEqual([
    ['Alice', WORLD_LOBBY_ID, false],
    ['Pip', WORLD_LOBBY_ID, true],
  ]);
  expect(screen.getByText('Contractor · Lobby')).toBeDefined();
  expect(screen.getByRole('button', { name: /Offline · Offline/ })).toBeDefined();
});

it('switches between browse panels, moves focus into details and restores the visible trigger on close', async () => {
  const local = runtime();
  await show(local);
  const toggle = await directory();
  const trigger = screen.getByRole('button', { name: /Alice · Online/ });
  await userEvent.click(trigger);
  expect(screen.getByRole('complementary', { name: 'Agent details' })).toBeDefined();
  expect(screen.queryByRole('complementary', { name: 'Office directory' })).toBeNull();
  expect(document.activeElement).toBe(screen.getByRole('heading', { name: 'Alice' }));
  await userEvent.click(screen.getByRole('button', { name: 'Close agent conversation' }));
  expect(document.activeElement).toBe(toggle);
  await userEvent.click(toggle);
  await userEvent.keyboard('{Escape}');
  expect(screen.queryByRole('complementary', { name: 'Office directory' })).toBeNull();
  expect(document.activeElement).toBe(toggle);
  const details = screen.getByLabelText('Local office details');
  await userEvent.click(details);
  expect(details.closest('details')!.open).toBe(true);
  await userEvent.keyboard('{Escape}');
  expect(details.closest('details')!.open).toBe(false);
  expect(local.world.show).toHaveBeenCalledTimes(1);
  expect(local.world.save).not.toHaveBeenCalled();
});

it('accepts fresh effective membership at the same room revision without rolling back a newer definition', async () => {
  const local = runtime();
  const room = {
    id: '44444444-4444-4444-8444-444444444444',
    name: 'Review',
    revision: 3,
    retired: false,
    memberIds: [identityId],
  };
  vi.mocked(local.rooms.list).mockResolvedValue([room]);
  const { result } = renderHook(() => useLocalOffice(), {
    wrapper: ({ children }) => (
      <LocalRuntimeContext.Provider value={local}>{children}</LocalRuntimeContext.Provider>
    ),
  });
  await waitFor(() => expect(result.current.load.status).toBe('ready'));
  const empty = { ...room, memberIds: [] };
  act(() => result.current.roomChanged(empty));
  expect(result.current.load).toMatchObject({ rooms: [empty] });
  act(() => result.current.roomChanged({ ...room, revision: 2 }));
  expect(result.current.load).toMatchObject({ rooms: [empty] });
  expect(local.world.show).toHaveBeenCalledTimes(1);
  expect(local.world.save).not.toHaveBeenCalled();
});

it('preserves room observations made during a stale refresh, including equal revisions and newly created rooms', async () => {
  const local = runtime();
  const room = {
    id: '44444444-4444-4444-8444-444444444444',
    name: 'Review',
    revision: 3,
    retired: false,
    memberIds: [identityId],
  };
  vi.mocked(local.rooms.list).mockResolvedValue([room]);
  const { result } = renderHook(() => useLocalOffice(), {
    wrapper: ({ children }) => (
      <LocalRuntimeContext.Provider value={local}>{children}</LocalRuntimeContext.Provider>
    ),
  });
  await waitFor(() => expect(result.current.load.status).toBe('ready'));
  let finish!: (rooms: (typeof room)[]) => void;
  vi.mocked(local.rooms.list).mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      })
  );
  act(() => result.current.refresh());
  const empty = { ...room, memberIds: [] };
  const created = {
    ...room,
    id: '55555555-5555-4555-8555-555555555555',
    name: 'Planning',
    revision: 1,
  };
  act(() => {
    result.current.roomChanged(empty);
    result.current.roomChanged(created);
  });
  await act(async () => finish([room]));
  expect(result.current.load).toMatchObject({
    status: 'ready',
    refreshing: false,
    rooms: [empty, created],
  });
  // The fence belongs only to the completed read, not a second room store.
  vi.mocked(local.rooms.list).mockResolvedValue([room]);
  act(() => result.current.refresh());
  await waitFor(() =>
    expect(result.current.load).toMatchObject({ refreshing: false, rooms: [room] })
  );
});

it('keeps complete searchable meeting rosters and typed visual context without duplicating identities', async () => {
  const offline = {
    ...profile,
    identityId: '33333333-3333-4333-8333-333333333333',
    identityName: 'Bob',
    presence: 'offline' as const,
    lifetime: 'temporary' as const,
  };
  const local = runtime([profile, offline]);
  const world = officeWorldFixture();
  const roomId = '44444444-4444-4444-8444-444444444444';
  const areaId = '55555555-5555-4555-8555-555555555555';
  const missing = '66666666-6666-4666-8666-666666666666';
  const populated: WorldSnapshot = {
    ...world,
    layout: {
      ...world.layout,
      map: {
        ...world.layout.map,
        areas: [
          ...mapGeometry(world.layout.map).areas,
          {
            id: areaId,
            name: 'West meeting',
            binding: { type: 'meeting', roomId },
          },
        ],
      },
    },
  };
  vi.mocked(local.world.show).mockResolvedValue(populated);
  vi.mocked(local.rooms.list).mockResolvedValue([
    {
      id: roomId,
      name: 'Architecture review',
      revision: 3,
      retired: false,
      memberIds: [identityId, offline.identityId, missing],
    },
  ]);
  await show(local);
  await directory();
  await userEvent.type(screen.getByRole('searchbox', { name: 'Search directory' }), 'architecture');
  await userEvent.click(screen.getByRole('button', { name: /West meeting · meeting/ }));
  const roster = screen.getByRole('region', { name: 'Area roster' });
  expect(within(roster).getByRole('heading', { name: 'Members · 3' })).toBeDefined();
  expect(within(roster).getByText(missing)).toBeDefined();
  expect(within(roster).getByRole('button', { name: /Bob · Offline Contractor/ })).toBeDefined();
  await userEvent.type(within(roster).getByRole('searchbox', { name: 'Search members' }), 'bob');
  expect(within(roster).queryByRole('button', { name: /Alice/ })).toBeNull();
  await userEvent.click(within(roster).getByRole('button', { name: /Bob/ }));
  expect(screen.getByRole('heading', { name: 'Bob', level: 2 })).toBeDefined();
  expect(
    screen.getByText('Viewing in Architecture review. Membership is independent of the home area.')
  ).toBeDefined();
  expect(screen.getByRole('button', { name: 'Message Bob' })).toBeDefined();
  act(() => canvas.select!({ kind: 'agent', identityId, areaId }));
  expect(screen.getByRole('heading', { name: 'Alice', level: 2 })).toBeDefined();
  await userEvent.click(screen.getByRole('tab', { name: 'Info' }));
  await userEvent.click(screen.getByRole('button', { name: 'View West meeting roster' }));
  expect(screen.getByRole('heading', { name: 'Members · 3' })).toBeDefined();
  expect(local.world.save).not.toHaveBeenCalled();
  expect(local.rooms.save).not.toHaveBeenCalled();
  expect(local.dispatch.send).not.toHaveBeenCalled();
});

it('retains an unsaved appearance draft while browsing suspends the non-modal HUD', async () => {
  const local = runtime();
  await show(local);
  const toggle = await directory();
  await userEvent.click(screen.getByRole('button', { name: /Alice · Online/ }));
  await userEvent.click(screen.getByRole('button', { name: 'Appearance' }));
  const mark = screen.getByLabelText('Shirt mark');
  await userEvent.clear(mark);
  await userEvent.type(mark, 'UX');
  await directory();
  expect(screen.queryByRole('complementary', { name: 'Agent details' })).toBeNull();
  expect(screen.getByRole('complementary', { name: 'Office directory' })).toBeDefined();
  await userEvent.keyboard('{Escape}');
  expect(document.activeElement).toBe(toggle);
  await directory();
  await userEvent.click(screen.getByRole('button', { name: /Alice · Online/ }));
  expect(screen.getByLabelText('Shirt mark')).toBe(mark);
  expect(mark).toHaveProperty('value', 'UX');
  expect(local.profiles.apply).not.toHaveBeenCalled();
  expect(local.world.save).not.toHaveBeenCalled();
});

it('refreshes the world without unmounting a conversation draft, including failed refreshes', async () => {
  const local = runtime();
  vi.mocked(local.requests.list).mockResolvedValue({ items: [], nextBefore: null });
  vi.mocked(local.requests.receipt).mockResolvedValue(null);
  await show(local);
  await directory();
  await userEvent.click(screen.getByRole('button', { name: /Alice · Online/ }));
  await userEvent.click(screen.getByRole('tab', { name: 'Chat' }));
  const message = await screen.findByRole('textbox', { name: 'Message' });
  fireEvent.change(message, { target: { value: 'Keep this through refresh' } });
  let finish!: (value: WorldSnapshot) => void;
  vi.mocked(local.world.show).mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      })
  );
  await userEvent.click(screen.getByRole('button', { name: 'Refresh office' }));
  expect(screen.getByRole('textbox', { name: 'Message' })).toBe(message);
  expect(screen.queryByRole('button', { name: 'Edit layout' })).toBeNull();
  const base = officeWorldFixture();
  const latest = {
    ...base,
    revision: 4,
    layout: {
      ...base.layout,
      map: {
        ...base.layout.map,
        areas: mapGeometry(base.layout.map).areas.map((area) => ({
          ...area,
          name: 'Refreshed Lobby',
        })),
      },
    },
  };
  await act(async () => finish(latest));
  expect(mapGeometry(canvas.model!.world.map).areas[0]!.name).toBe('Refreshed Lobby');
  expect(screen.getByRole('textbox', { name: 'Message' })).toBe(message);
  expect(message).toHaveProperty('value', 'Keep this through refresh');
  vi.mocked(local.world.show).mockRejectedValueOnce(new Error('offline'));
  await userEvent.click(screen.getByRole('button', { name: 'Refresh office' }));
  expect(await screen.findByRole('alert')).toHaveProperty(
    'textContent',
    expect.stringContaining('Your open work is kept')
  );
  expect(message).toHaveProperty('value', 'Keep this through refresh');
  expect(local.dispatch.send).not.toHaveBeenCalled();
  expect(local.world.save).not.toHaveBeenCalled();
});

it('unifies modular conversion and object changes into one draft with Undo, Redo and Cancel', async () => {
  const local = runtime();
  await show(local);
  await userEvent.click(screen.getByRole('button', { name: 'Preview modular layout' }));
  const painted = canvas.model!.world;
  expect(painted.map.version).toBe(4);
  act(() => canvas.editor!.select!(painted.objects[0]!.id));
  await userEvent.click(screen.getByText('Precise placement', { selector: 'summary' }));
  await userEvent.click(screen.getByRole('button', { name: 'Rotate 90° clockwise' }));
  expect(canvas.model!.world.objects[0]!.placement.rotation).toBe(1);
  await userEvent.click(screen.getByRole('button', { name: 'Undo' }));
  expect(canvas.model!.world).toEqual(painted);
  await userEvent.click(screen.getByRole('button', { name: 'Undo' }));
  expect(canvas.model!.world).toEqual(officeWorldFixture().layout);
  await userEvent.click(screen.getByRole('button', { name: 'Redo' }));
  expect(canvas.model!.world).toEqual(painted);
  await userEvent.click(screen.getByRole('button', { name: 'Undo' }));
  expect(canvas.model!.world).toEqual(officeWorldFixture().layout);
  expect(local.world.save).not.toHaveBeenCalled();
});

it.each([false, true])(
  'keeps a placement selectable by stable ID when its art is unavailable: %s',
  async (missing) => {
    const local = runtime();
    const snapshot = officeWorldFixture();
    const object = snapshot.layout.objects[0]!;
    if (missing) object.placement.prop = `sha256:${'f'.repeat(64)}/missing-desk`;
    vi.mocked(local.world.show).mockResolvedValue(snapshot);
    await show(local);
    act(() => canvas.editor!.select!(object.id));
    expect(canvas.editor!.selected).toBe(object.id);
    expect(screen.getByRole('heading', { name: /^Selected object: / }).textContent).toBe(
      missing ? 'Unavailable prop' : 'Desk'
    );
    expect(screen.getByRole('button', { name: /^Delete$/ })).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Move object' })).toBeNull();
    expect(local.world.save).not.toHaveBeenCalled();
  }
);

it('shows the visual catalog by default and selects newly added objects without a tool mode', async () => {
  const local = runtime();
  await show(local);
  const before = structuredClone(canvas.model!.world);
  expect(screen.getByRole('heading', { name: 'Furniture & devices' })).toBeDefined();
  expect(screen.queryByRole('toolbar', { name: 'Build tools' })).toBeNull();
  expect(local.world.save).not.toHaveBeenCalled();
  expect(screen.queryByText('Legacy pixel basics')).toBeNull();
  await userEvent.click(screen.getByRole('button', { name: 'Workshop writing desk' }));
  expect(canvas.model!.world.objects).toHaveLength(before.objects.length + 1);
  expect(canvas.editor!.selected).toBe(canvas.model!.world.objects.at(-1)!.id);
  await userEvent.click(screen.getByRole('button', { name: 'Undo' }));
  expect(canvas.model!.world).toEqual(before);
});

it('deletes the selection with Delete, preserves text editing, and restores it with Undo', async () => {
  const local = runtime();
  await show(local);
  const before = structuredClone(canvas.model!.world);
  act(() => canvas.editor!.select!(before.objects[0]!.id));
  expect(screen.queryByRole('button', { name: 'Edit room settings' })).toBeNull();
  await userEvent.click(screen.getByText('Precise placement', { selector: 'summary' }));
  await userEvent.click(screen.getByLabelText('X'));
  await userEvent.keyboard('{Delete}');
  expect(canvas.model!.world).toEqual(before);
  screen.getByRole('heading', { name: 'Selected object: Desk' }).focus();
  await userEvent.keyboard('{Delete}');
  expect(canvas.model!.world.objects).toHaveLength(before.objects.length - 1);
  await userEvent.click(screen.getByRole('button', { name: 'Undo' }));
  expect(canvas.model!.world).toEqual(before);
});

it('saves the complete world with its existing revision fence and no legacy write', async () => {
  const local = runtime();
  await show(local);
  await userEvent.click(screen.getByRole('button', { name: 'Preview modular layout' }));
  const draft = structuredClone(canvas.model!.world);
  await waitFor(() => expect(local.world.save).toHaveBeenCalled());
  expect(local.world.save).toHaveBeenCalledWith(
    { expectedRevision: 1, legacyBasis: null, layout: draft },
    expect.any(AbortSignal)
  );
  expect(canvas.model!.world).toEqual(draft);
});

it('edits wall mounts within the same undo history, preserving the placement and resource identity', async () => {
  const local = runtime();
  const snapshot = officeWorldFixture();
  const original = {
    ...snapshot.layout.objects[0]!,
    extension: {
      definition: 'tmt-whiteboard',
      binding: { kind: 'whiteboard' as const, documentId: 'lobby' },
    },
  };
  vi.mocked(local.world.show).mockResolvedValue({
    ...snapshot,
    layout: { ...snapshot.layout, objects: [original] },
  });
  await show(local);
  act(() => canvas.editor!.select!(original.id));
  await userEvent.selectOptions(screen.getByLabelText('Placement surface'), 'wall');
  expect(canvas.model!.world.objects[0]!.surface).toEqual({
    type: 'wall',
    axis: 'horizontal',
    face: 'positive',
    elevation: 0,
  });
  await userEvent.click(screen.getByRole('button', { name: 'Undo' }));
  expect(canvas.model!.world.objects[0]).toEqual(original);
  await userEvent.click(screen.getByRole('button', { name: 'Redo' }));
  await userEvent.selectOptions(screen.getByLabelText('Wall direction'), 'vertical');
  await userEvent.selectOptions(screen.getByLabelText('Indoor face'), 'negative');
  fireEvent.change(screen.getByLabelText('Elevation'), { target: { value: '3' } });
  fireEvent.change(screen.getByLabelText('X', { exact: true }), { target: { value: '36' } });
  expect(canvas.model!.world.objects[0]!.placement.x).toBe(original.placement.x);
  await userEvent.click(screen.getByRole('button', { name: 'Apply coordinates' }));
  const expected = {
    ...original,
    placement: { ...original.placement, x: 36 },
    surface: {
      type: 'wall',
      axis: 'vertical',
      face: 'negative',
      elevation: 3,
    },
  };
  expect(canvas.model!.world.objects[0]).toEqual(expected);
  await userEvent.click(screen.getByRole('button', { name: 'Undo' }));
  expect(canvas.model!.world.objects[0]!.placement.x).toBe(original.placement.x);
  expect(canvas.model!.world.objects[0]!.surface).toMatchObject({ elevation: 0 });
  await userEvent.click(screen.getByRole('button', { name: 'Redo' }));
  expect(canvas.model!.world.objects[0]).toEqual(expected);
  await waitFor(() => expect(local.world.save).toHaveBeenCalled());
  expect(vi.mocked(local.world.save).mock.calls.at(-1)![0].layout.objects).toEqual([expected]);
  expect(local.whiteboards.show).not.toHaveBeenCalled();
  expect(local.whiteboards.save).not.toHaveBeenCalled();
  expect(local.dispatch.send).not.toHaveBeenCalled();
});

it('preserves an invalid object draft and its resource binding when native admission rejects Save', async () => {
  const local = runtime();
  const id = officeWorldFixture().layout.objects[0]!.id;
  vi.mocked(local.world.save).mockRejectedValue(
    new WorldValidationError('Move affected objects before saving.', [
      { objectId: id, reason: 'outsideFloor' },
    ])
  );
  await show(local);
  const original = canvas.model!.world.objects.find((object) => object.id === id)!;
  act(() =>
    canvas.editor!.placeObject({
      ...original,
      placement: { ...original.placement, x: -100, y: -100 },
    })
  );
  const invalid = canvas.model!.world;
  await waitFor(() => expect(local.world.save).toHaveBeenCalled());
  expect(await screen.findByRole('alert')).toHaveProperty(
    'textContent',
    expect.stringContaining('Move affected objects')
  );
  expect(canvas.model!.world).toEqual(invalid);
  expect(canvas.model!.world.objects[0]).toEqual({
    ...officeWorldFixture().layout.objects[0],
    placement: { ...officeWorldFixture().layout.objects[0]!.placement, x: -100, y: -100 },
  });
  await userEvent.click(screen.getByRole('button', { name: 'Select affected object' }));
  expect(canvas.editor!.selected).toBe(id);
  expect(local.world.save).toHaveBeenCalledTimes(1);
  expect(local.world.show).toHaveBeenCalledTimes(1);
});

it('retains conflicts until explicit reload, with no automatic read, rebase or retry', async () => {
  const local = runtime();
  vi.mocked(local.world.save).mockRejectedValue(new WorldConflict());
  await show(local);
  await userEvent.click(screen.getByRole('button', { name: 'Preview modular layout' }));
  const draft = canvas.model!.world;
  await waitFor(() => expect(local.world.save).toHaveBeenCalled());
  await screen.findByText(/Saved layout changed. Nothing was written./);
  expect(canvas.model!.world).toEqual(draft);
  expect(local.world.show).toHaveBeenCalledTimes(1);
  expect(local.world.save).toHaveBeenCalledTimes(1);
  await userEvent.click(
    screen.getByRole('button', { name: 'Reload saved layout (discard local changes)' })
  );
  expect(canvas.model!.world).toEqual(officeWorldFixture().layout);
});

it('creates an unassigned office module explicitly and excludes Contractors from its resident options', async ({
  onTestFinished,
}) => {
  // DOM tests exercise the real form, not browser layout. Native E2E owns sizing.
  vi.stubGlobal(
    'ResizeObserver',
    class {
      observe() {}
      disconnect() {}
    }
  );
  onTestFinished(() => {
    vi.unstubAllGlobals();
  });
  const local = runtime([
    profile,
    {
      ...profile,
      identityId: '33333333-3333-4333-8333-333333333333',
      identityName: 'Pip',
      lifetime: 'temporary',
    },
  ]);
  await show(local);
  await userEvent.click(screen.getByRole('button', { name: 'Preview modular layout' }));
  act(() => canvas.editor!.chooseOffice!(canvas.editor!.officeSlots![0]!));
  const form = within(screen.getByRole('form', { name: 'New office' }));
  await userEvent.type(form.getByLabelText('Name'), 'Studio');
  await userEvent.click(form.getByRole('button', { name: 'Add office' }));
  expect(mapGeometry(canvas.model!.world.map).areas).toHaveLength(2);
  expect(canvas.model!.world.map.version).toBe(4);
  const select = screen.getByLabelText('Resident');
  expect(within(select).getByRole('option', { name: /Pip/ })).toHaveProperty('disabled', true);
  await userEvent.selectOptions(select, identityId);
  expect(
    mapGeometry(canvas.model!.world.map).areas.find((area) => area.name === 'Studio')!.binding
  ).toEqual({
    type: 'personal',
    identityId,
  });
  expect(local.world.save).not.toHaveBeenCalled();
});

it('requires a replacement Lobby and detaches other areas without deleting placements', async () => {
  const local = runtime();
  const retained = officeWorldFixture();
  const extraId = '33333333-3333-4333-8333-333333333333';
  vi.mocked(local.world.show).mockResolvedValue({
    ...retained,
    layout: {
      ...retained.layout,
      map: {
        ...retained.layout.map,
        areas: [
          ...retained.layout.map.areas,
          {
            id: extraId,
            name: 'Empty studio',
            binding: { type: 'personal', identityId: null },
          },
        ],
      },
    },
  });
  await show(local);
  act(() => canvas.select!({ kind: 'area', areaId: WORLD_LOBBY_ID }));
  expect(screen.getByRole('button', { name: 'Remove area designation' })).toHaveProperty(
    'disabled',
    true
  );
  await userEvent.click(screen.getByRole('button', { name: 'Preview modular layout' }));
  expect(await screen.findByRole('alert')).toHaveProperty(
    'textContent',
    expect.stringContaining('Remove the empty area')
  );
  act(() => canvas.select!({ kind: 'area', areaId: extraId }));
  await userEvent.click(screen.getByRole('button', { name: 'Remove area designation' }));
  expect(mapGeometry(canvas.model!.world.map).areas).toHaveLength(1);
  expect(mapGeometry(canvas.model!.world.map).floor).toEqual(retained.layout.map.floor);
  expect(canvas.model!.world.objects).toEqual(officeWorldFixture().layout.objects);
  await userEvent.click(screen.getByRole('button', { name: 'Preview modular layout' }));
  expect(canvas.model!.world.map.version).toBe(4);
  expect(local.world.save).not.toHaveBeenCalled();
});

it('disables refresh and concurrent mutation while Save is pending and aborts on unmount', async () => {
  const local = runtime();
  let resolve!: (value: WorldSnapshot) => void;
  vi.mocked(local.world.save).mockImplementation(
    () =>
      new Promise((done) => {
        resolve = done;
      })
  );
  const view = await show(local);
  const original = canvas.model!.world.objects[0]!;
  act(() =>
    canvas.editor!.placeObject({ ...original, placement: { ...original.placement, x: 10, y: 10 } })
  );
  await waitFor(() => expect(local.world.save).toHaveBeenCalled());
  expect(screen.getByRole('button', { name: 'Undo' })).toHaveProperty('disabled', false);
  expect(screen.getByRole('button', { name: 'Refresh office' })).toHaveProperty('disabled', true);
  const signal = vi.mocked(local.world.save).mock.calls[0]![1]!;
  view.unmount();
  expect(signal.aborted).toBe(true);
  await act(async () =>
    resolve({ ...officeWorldFixture(), changed: true, revision: 2, updatedAtMs: 2 })
  );
});

it('fences retired room observations against an older in-flight directory read without changing the world', async () => {
  const local = runtime();
  const room = {
    id: '44444444-4444-4444-8444-444444444444',
    name: 'Review',
    revision: 1,
    retired: false,
    memberIds: [identityId],
  };
  vi.mocked(local.rooms.list).mockResolvedValue([room]);
  const { result } = renderHook(() => useLocalOffice(), {
    wrapper: ({ children }) => (
      <LocalRuntimeContext.Provider value={local}>{children}</LocalRuntimeContext.Provider>
    ),
  });
  await waitFor(() => expect(result.current.load.status).toBe('ready'));
  let finish!: (rooms: (typeof room)[]) => void;
  vi.mocked(local.rooms.list).mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      })
  );
  act(() => result.current.refresh());
  act(() => result.current.roomChanged({ ...room, retired: true, revision: 2 }));
  expect(result.current.load).toMatchObject({ rooms: [] });
  await act(async () => finish([room]));
  expect(result.current.load).toMatchObject({ rooms: [], refreshing: false });
  expect(local.world.save).not.toHaveBeenCalled();
});

it('opens an identity deep link in the same world, without manufacturing a private room', async () => {
  const local = runtime([{ ...profile, presence: 'offline' as const }]);
  await show(local, `/local/agents/${identityId}`);
  const panel = await screen.findByRole('complementary', { name: 'Agent details' });
  expect(within(panel).getByRole('heading', { name: 'Alice' })).toBeDefined();
  expect(canvas.model!.actors).toEqual([]);
  expect(mapGeometry(canvas.model!.world.map).areas).toHaveLength(1);
  await userEvent.click(within(panel).getByRole('button', { name: 'Appearance' }));
  expect(local.avatars.list).toHaveBeenCalledTimes(1);
  expect(local.world.save).not.toHaveBeenCalled();
});

it('fences late completion from a replaced runtime', async () => {
  const old = runtime();
  let resolve!: (value: ProfileProjection[]) => void;
  old.profiles.list = () =>
    new Promise((done) => {
      resolve = done;
    });
  const view = await show(old, '/local', false);
  await screen.findByText('Opening your office…');
  view.rerender(
    <OfficeApp router={view.router} local={runtime([{ ...profile, identityName: 'Bob' }])} />
  );
  await directory();
  await screen.findByRole('button', { name: /Bob · Online/ });
  await act(async () => resolve([profile]));
  expect(screen.queryByRole('button', { name: /Alice · Online/ })).toBeNull();
});

it('does not expose a previous ready runtime while the replacement loads', async () => {
  const view = await show(runtime());
  await directory();
  await screen.findByRole('button', { name: /Alice · Online/ });
  const next = runtime();
  let resolve!: (value: ProfileProjection[]) => void;
  next.profiles.list = () =>
    new Promise((done) => {
      resolve = done;
    });
  view.rerender(<OfficeApp router={view.router} local={next} />);
  expect(screen.queryByRole('button', { name: /Alice · Online/ })).toBeNull();
  await act(async () => resolve([{ ...profile, identityName: 'Bob' }]));
  await directory();
  await waitFor(() => expect(screen.getByRole('button', { name: /Bob · Online/ })).toBeDefined());
});
