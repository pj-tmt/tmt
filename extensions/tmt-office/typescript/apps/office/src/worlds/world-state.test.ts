import { describe, expect, it, vi } from 'vitest';
import { createSession } from '../auth/session.js';
import type { SessionUser } from '../auth/session.js';
import type { World, WorldPort } from './world-contract.js';
import { createWorldState } from './world-state.js';

function fixture() {
  let changeUser: (user: SessionUser | null) => void = () => {};
  const session = createSession({
    observe: (changed) => {
      changeUser = changed;
      return vi.fn();
    },
    signIn: async () => {},
    signOut: async () => {
      changeUser(null);
    },
    dispose: async () => {},
  });
  const admissions: Array<(enabled: boolean) => void> = [];
  const worlds: Array<(world: World | null) => void> = [];
  const stopAdmission = vi.fn();
  const stopWorld = vi.fn();
  const port: WorldPort = {
    draft: vi.fn((name) => ({ id: 'a'.repeat(20), name })),
    create: vi.fn(async (draft) => draft.id),
    watch: vi.fn((_id, changed) => {
      worlds.push(changed);
      return stopWorld;
    }),
    watchAdmission: vi.fn((_uid, changed) => {
      admissions.push(changed);
      return stopAdmission;
    }),
  };
  const state = createWorldState(session, port);
  changeUser({ uid: 'alice', displayName: 'Alice' });
  return { state, port, admissions, worlds, stopAdmission, stopWorld, changeUser, session };
}
const world: World = { id: 'a'.repeat(20), name: 'Private', ownerUid: 'alice', createdAtMs: 1 };

describe('private world lifetime', () => {
  it('duplicate approval preserves the selected world and its active subscription', () => {
    const f = fixture();
    f.state.select(world.id);
    f.admissions[0](true);
    f.worlds[0](world);
    const observed: Array<World | null> = [];
    const unsubscribe = f.state.subscribe(() => observed.push(f.state.getSnapshot().world));
    f.admissions[0](true);
    expect(f.state.getSnapshot().world).toEqual(world);
    expect(observed).not.toContain(null);
    expect(f.port.watch).toHaveBeenCalledOnce();
    expect(f.stopWorld).not.toHaveBeenCalled();
    f.admissions[0](false);
    expect(f.state.getSnapshot().world).toBeNull();
    expect(f.stopWorld).toHaveBeenCalledOnce();
    unsubscribe();
    f.state.dispose();
  });
  it('invalid local input reports validation feedback without a draft or transport', async () => {
    const f = fixture();
    f.admissions[0](true);
    expect(await f.state.create('x'.repeat(81))).toBeNull();
    expect(f.port.draft).not.toHaveBeenCalled();
    expect(f.port.create).not.toHaveBeenCalled();
    expect(f.state.getSnapshot()).toMatchObject({ draft: null, busy: false });
    expect(f.state.getSnapshot().error).toContain('at most 80');
    f.state.dispose();
  });
  it('waiting does not subscribe to worlds; approval and revocation update the same session', () => {
    const f = fixture();
    f.state.select(world.id);
    f.admissions[0](false);
    expect(f.port.watch).not.toHaveBeenCalled();
    f.admissions[0](true);
    f.worlds[0](world);
    expect(f.state.getSnapshot().world).toEqual(world);
    f.admissions[0](false);
    expect(f.state.getSnapshot().world).toBeNull();
    expect(f.stopWorld).toHaveBeenCalledOnce();
    f.worlds[0](world);
    expect(f.state.getSnapshot().world).toBeNull();
    f.admissions[0](true);
    expect(f.port.watch).toHaveBeenCalledTimes(2);
    f.state.dispose();
  });
  it('account changes detach listeners and fence callbacks from the previous user', () => {
    const f = fixture();
    f.admissions[0](true);
    f.state.select(world.id);
    f.worlds[0](world);
    f.changeUser({ uid: 'bob', displayName: 'Bob' });
    expect(f.stopAdmission).toHaveBeenCalledOnce();
    expect(f.state.getSnapshot().world).toBeNull();
    f.worlds[0](world);
    f.admissions[0](true);
    expect(f.state.getSnapshot().admission).toBe('checking');
    expect(f.state.getSnapshot().world).toBeNull();
    f.admissions[1](true);
    f.worlds[1](world);
    expect(f.state.getSnapshot().world).toBeNull();
    f.state.dispose();
  });
  it('uncertain creation retries preserve the same ID and name', async () => {
    const f = fixture();
    f.admissions[0](true);
    vi.mocked(f.port.create).mockRejectedValueOnce(new Error('network'));
    expect(await f.state.create('Original')).toBeNull();
    const draft = f.state.getSnapshot().draft;
    expect(await f.state.create('Changed')).toBe(world.id);
    expect(f.port.create).toHaveBeenNthCalledWith(2, draft, 'alice');
    expect(f.port.draft).toHaveBeenCalledOnce();
    f.state.dispose();
  });
  it('a late successful create after revocation cannot navigate or restore private state', async () => {
    const f = fixture();
    f.admissions[0](true);
    let complete: (id: string) => void = () => {};
    vi.mocked(f.port.create).mockImplementation(
      () =>
        new Promise((resolve) => {
          complete = resolve;
        })
    );
    const result = f.state.create('Late');
    expect(await f.state.create('Duplicate')).toBeNull();
    f.admissions[0](false);
    complete(world.id);
    expect(await result).toBeNull();
    expect(f.state.getSnapshot()).toMatchObject({ admission: 'waiting', busy: false, world: null });
    f.state.dispose();
  });
  it('disposal fences every callback and clears private snapshots', () => {
    const f = fixture();
    f.admissions[0](true);
    f.state.select(world.id);
    f.worlds[0](world);
    f.state.dispose();
    f.worlds[0](world);
    f.admissions[0](true);
    expect(f.state.getSnapshot()).toMatchObject({ admission: 'signed-out', world: null });
  });
});
