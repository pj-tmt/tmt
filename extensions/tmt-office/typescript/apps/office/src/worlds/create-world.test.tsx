import { createMemoryHistory } from '@tanstack/react-router';
import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { createSession } from '../auth/session.js';
import { OfficeApp } from '../office-app.js';
import { createOfficeRouter } from '../router.js';
import { createWorldState } from './world-state.js';

const worldId = 'a'.repeat(20);

function fixture() {
  let changeUser: (user: null) => void = () => {};
  const session = createSession({
    observe: (changed) => {
      changeUser = changed;
      changed({ uid: 'alice', displayName: 'Alice' });
      return () => {};
    },
    signIn: async () => {},
    signOut: async () => changeUser(null),
    dispose: async () => {},
  });
  let complete: (id: string) => void = () => {};
  let fail: (error: Error) => void = () => {};
  const create = vi.fn(
    () =>
      new Promise<string>((resolve, reject) => {
        complete = resolve;
        fail = reject;
      })
  );
  let admit: (enabled: boolean) => void = () => {};
  const draft = vi.fn((name: string) => ({ id: worldId, name }));
  const worlds = createWorldState(session, {
    draft,
    create,
    watchAdmission: (_uid, changed) => {
      admit = changed;
      changed(true);
      return () => {};
    },
    watch: (_id, changed) => {
      changed(null);
      return () => {};
    },
  });
  const router = createOfficeRouter(createMemoryHistory({ initialEntries: ['/'] }));
  const view = render(<OfficeApp router={router} session={session} worlds={worlds} />);
  return {
    router,
    worlds,
    create,
    draft,
    complete: () => complete(worldId),
    fail: () => fail(new Error('Uncertain transport')),
    revoke: () => admit(false),
    signOut: () => changeUser(null),
    async start() {
      const user = userEvent.setup();
      await user.type(await screen.findByLabelText('World name'), 'Studio');
      await user.click(screen.getByRole('button', { name: 'Create world' }));
      expect(create).toHaveBeenCalledOnce();
    },
    async dispose() {
      view.unmount();
      worlds.dispose();
      await session.dispose();
    },
  };
}

it('the mounted form navigates after confirmed creation', async () => {
  const f = fixture();
  try {
    await f.start();
    await act(async () => f.complete());
    await waitFor(() => expect(f.router.state.location.pathname).toBe(`/worlds/${worldId}`));
  } finally {
    await f.dispose();
  }
});

it.each(['setup', 'world', 'return'] as const)(
  'late creation does not take over a %s navigation',
  async (destination) => {
    const f = fixture();
    try {
      await f.start();
      await act(async () => {
        if (destination === 'world')
          await f.router.navigate({ to: '/worlds/$worldId', params: { worldId: 'b'.repeat(20) } });
        else await f.router.navigate({ to: '/setup' });
      });
      if (destination === 'return')
        await act(async () => {
          await f.router.navigate({ to: '/' });
        });
      const location = f.router.state.location.pathname;
      await act(async () => f.complete());
      expect(f.router.state.location.pathname).toBe(location);
      expect(f.worlds.getSnapshot()).toMatchObject({ busy: false, draft: null });
      expect(f.create).toHaveBeenCalledOnce();
    } finally {
      await f.dispose();
    }
  }
);

it.each(['revoke', 'signOut'] as const)(
  '%s prevents late creation from navigating',
  async (action) => {
    const f = fixture();
    try {
      await f.start();
      await act(async () => f[action]());
      await act(async () => f.complete());
      expect(f.router.state.location.pathname).toBe('/');
      expect(screen.queryByLabelText('World name')).toBeNull();
    } finally {
      await f.dispose();
    }
  }
);

it('a remounted form retries the same uncertain creation explicitly', async () => {
  const f = fixture();
  try {
    await f.start();
    await act(async () => {
      await f.router.navigate({ to: '/setup' });
    });
    await act(async () => f.fail());
    await act(async () => {
      await f.router.navigate({ to: '/' });
    });
    expect(f.create).toHaveBeenCalledOnce();
    await userEvent.setup().click(await screen.findByRole('button', { name: 'Retry creation' }));
    expect(f.draft).toHaveBeenCalledOnce();
    expect(f.create.mock.calls[1]).toEqual(f.create.mock.calls[0]);
    await act(async () => f.complete());
    await waitFor(() => expect(f.router.state.location.pathname).toBe(`/worlds/${worldId}`));
  } finally {
    await f.dispose();
  }
});
