import { createMemoryHistory } from '@tanstack/react-router';
import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { createSession } from '../auth/session.js';
import { OfficeApp } from '../office-app.js';
import { createOfficeRouter } from '../router.js';
import { createWorldState } from './world-state.js';
import type { World } from './world-contract.js';
import { builtinFurniture } from '../blocks/block-contract.js';
import type { Furniture } from '../blocks/block-contract.js';

it('reconfirmed admission retains the mounted editor draft until an explicit save', async () => {
  const world: World = {
    id: 'a'.repeat(20),
    name: 'Private studio',
    ownerUid: 'alice',
    createdAtMs: 1,
  };
  const session = createSession({
    observe: (changed) => {
      changed({ uid: 'alice', displayName: 'Alice' });
      return () => {};
    },
    signIn: async () => {},
    signOut: async () => {},
    dispose: async () => {},
  });
  let admit: (enabled: boolean) => void = () => {};
  let observeWorld: (value: World | null) => void = () => {};
  const watch = vi.fn((_id: string, changed: (value: World | null) => void) => {
    observeWorld = changed;
    return () => {};
  });
  const worlds = createWorldState(session, {
    draft: () => {
      throw new Error('This scenario opens an existing world.');
    },
    create: async () => {
      throw new Error('This scenario must not create a world.');
    },
    watchAdmission: (_uid, changed) => {
      admit = changed;
      return () => {};
    },
    watch,
  });
  const apply = vi.fn(async (_id: string, revision: number, objects: Furniture[]) => ({
    revision: revision + 1,
    objects,
    updatedAtMs: 1,
  }));
  const stopBlock = vi.fn();
  admit(true);
  const router = createOfficeRouter(
    createMemoryHistory({
      initialEntries: [`/worlds/${world.id}`],
    })
  );
  const view = render(
    <OfficeApp
      router={router}
      session={session}
      worlds={worlds}
      blocks={{
        watch: (_id, changed) => {
          changed(null);
          return stopBlock;
        },
        apply,
      }}
    />
  );
  try {
    const user = userEvent.setup();
    await waitFor(() => expect(watch).toHaveBeenCalledOnce());
    await act(async () => observeWorld(world));
    await user.click(await screen.findByRole('button', { name: 'Add desk' }));
    await act(async () => admit(true));
    expect(screen.getByText('Unsaved changes', { exact: true })).toBeDefined();
    expect(stopBlock).not.toHaveBeenCalled();
    expect(apply).not.toHaveBeenCalled();
    await user.click(screen.getByRole('button', { name: 'Save layout' }));
    await screen.findByText('Saved · revision 1', { exact: true });
    expect(apply).toHaveBeenCalledExactlyOnceWith(world.id, 0, [
      builtinFurniture('desk', 14, 14, 0),
    ]);
    await act(async () => admit(false));
    expect(screen.queryByRole('region', { name: 'Office block editor' })).toBeNull();
    expect(stopBlock).toHaveBeenCalledOnce();
  } finally {
    view.unmount();
    worlds.dispose();
    await session.dispose();
  }
});
