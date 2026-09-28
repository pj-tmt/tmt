import { Buffer } from 'node:buffer';
import { createMemoryHistory } from '@tanstack/react-router';
import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { createSession } from '../auth/session.js';
import type { SessionUser } from '../auth/session.js';
import { createWorldState } from '../worlds/world-state.js';
import { createOfficeRouter } from '../router.js';
import { OfficeApp } from '../office-app.js';
import type { ApprovedPairing, PairingRequest, PairingPort } from './pairing-contract.js';
import type { SpacePort, SpacePage } from '../spaces/space-contract.js';
import vectors from '../../../../../../../contracts/office/pairing-examples.json' with { type: 'json' };

async function fixture(change: Record<string, unknown> = {}, spaces?: SpacePort) {
  const request = { ...vectors.valid[0], ...change } as PairingRequest;
  const world = { id: request.worldId, name: 'Private studio', ownerUid: 'owner', createdAtMs: 1 };
  let identity: (user: SessionUser | null) => void = () => {};
  let admission: (value: boolean) => void = () => {};
  const session = createSession({
    observe: (changed) => {
      identity = changed;
      changed({ uid: 'owner', displayName: 'Owner' });
      return () => {};
    },
    signIn: async () => {},
    signOut: async () => identity(null),
    dispose: async () => {},
  });
  const worlds = createWorldState(session, {
    draft: () => {
      throw new Error('No world creation in this scenario');
    },
    create: async () => {
      throw new Error('No world creation in this scenario');
    },
    watchAdmission: (_uid, changed) => {
      admission = changed;
      changed(true);
      return () => {};
    },
    watch: (_id, changed) => {
      changed(world);
      return () => {};
    },
  });
  const approved: ApprovedPairing = {
    ...request,
    principalUid: 'office-agent:00000000-0000-4000-8000-000000000003',
    blockId: '00000000-0000-4000-8000-000000000004',
    expiresAt: 2_000_000_000_000,
  };
  const port: PairingPort = { approve: vi.fn(async () => approved), revoke: vi.fn(async () => {}) };
  const fragment = `tmt-pair=${Buffer.from(JSON.stringify(request)).toString('base64url')}`;
  const router = createOfficeRouter(
    createMemoryHistory({ initialEntries: [`/worlds/${world.id}/pair#${fragment}`] })
  );
  const view = render(
    <OfficeApp router={router} session={session} worlds={worlds} pairing={port} spaces={spaces} />
  );
  return {
    port,
    approved,
    router,
    request,
    revokeAdmission: () => admission(false),
    async dispose() {
      view.unmount();
      worlds.dispose();
      await session.dispose();
    },
  };
}

it('pages revoked spaces without approving, then fixes explicit selection across uncertain retry', async () => {
  const id = '00000000-0000-4000-8000-000000000004';
  const source = {
    principalUid: `office-agent:${id}`,
    identityId: id,
    installationId: id,
    blockId: id,
    capabilities: ['layout.read'] as ['layout.read'],
    enabled: false,
    expiresAtMs: 1000,
  };
  const spaces: SpacePort = {
    list: vi
      .fn()
      .mockResolvedValueOnce({ entries: [{ ...source, enabled: true }], next: source.principalUid })
      .mockResolvedValueOnce({ entries: [source], next: null }),
    block: () => {
      throw new Error('Approval must not open or mutate a block');
    },
  };
  const f = await fixture({}, spaces);
  try {
    const user = userEvent.setup();
    await screen.findByText('No revoked grants on this page.');
    expect(f.port.approve).not.toHaveBeenCalled();
    await user.click(screen.getByRole('button', { name: 'Next retained spaces' }));
    await user.click(await screen.findByRole('radio', { name: /Retained block/ }));
    expect(spaces.list).toHaveBeenLastCalledWith(f.request.worldId, 'owner', source.principalUid);
    vi.mocked(f.port.approve).mockRejectedValueOnce(new Error('Lost response'));
    await user.click(screen.getByRole('checkbox'));
    await user.click(screen.getByRole('button', { name: 'Approve pairing' }));
    await screen.findByText(/The choice is fixed/);
    const fresh = screen.getByRole('radio', { name: 'New empty block' });
    await user.click(fresh);
    await user.click(screen.getByRole('button', { name: 'Retry same approval' }));
    await screen.findByText(/Approved\. Return/);
    expect(f.port.approve).toHaveBeenCalledTimes(2);
    for (const call of vi.mocked(f.port.approve).mock.calls)
      expect(call).toEqual([
        f.request,
        'owner',
        { principalUid: source.principalUid, blockId: id },
      ]);
  } finally {
    await f.dispose();
  }
});

it('requires explicit recognition, renders labels as text and never approves just by viewing', async () => {
  const f = await fixture({ identityLabel: '<img src=x onerror=alert(1)>' });
  try {
    const user = userEvent.setup();
    const approve = await screen.findByRole('button', { name: 'Approve pairing' });
    expect((approve as HTMLButtonElement).disabled).toBe(true);
    expect(f.port.approve).not.toHaveBeenCalled();
    const region = screen.getByRole('region', { name: 'Agent pairing request' });
    expect(region.querySelector('img')).toBeNull();
    expect(region.textContent).toContain('<img src=x onerror=alert(1)>');
    expect(region.textContent).toContain(
      'renew its access in leases of up to 24 hours until you revoke it'
    );
    await user.click(screen.getByRole('checkbox'));
    await user.click(approve);
    await screen.findByText(/Approved\. Return to the requesting terminal/);
    expect(f.port.approve).toHaveBeenCalledExactlyOnceWith(f.request, 'owner');
    await user.click(screen.getByRole('button', { name: 'Revoke request' }));
    await screen.findByText(/Request revoked\. Existing workspace content is retained/);
    expect(f.port.revoke).toHaveBeenCalledExactlyOnceWith(f.request, 'owner');
  } finally {
    await f.dispose();
  }
});

it('offers explicit cancellation before approval and does not approve while cancellation is uncertain', async () => {
  const f = await fixture();
  try {
    const user = userEvent.setup();
    vi.mocked(f.port.revoke).mockRejectedValueOnce(new Error('Lost response'));
    await user.click(await screen.findByRole('button', { name: 'Cancel request' }));
    await screen.findByText(/may already have completed/);
    expect(f.port.revoke).toHaveBeenCalledExactlyOnceWith(f.request, 'owner');
    expect(screen.queryByRole('button', { name: 'Approve pairing' })).toBeNull();
    await user.click(screen.getByRole('button', { name: 'Retry cancel request' }));
    await screen.findByText(/Request cancelled\. Existing workspace content is retained/);
    expect(f.port.approve).not.toHaveBeenCalled();
    expect(f.port.revoke).toHaveBeenCalledTimes(2);
    expect(f.port.revoke).toHaveBeenLastCalledWith(f.request, 'owner');
  } finally {
    await f.dispose();
  }
});

it.each(['admission', 'route'] as const)(
  'a %s change discards the request view and fences pending approval',
  async (boundary) => {
    const f = await fixture();
    try {
      let finish: (value: ApprovedPairing) => void = () => {};
      vi.mocked(f.port.approve).mockImplementation(
        () =>
          new Promise((resolve) => {
            finish = resolve;
          })
      );
      const user = userEvent.setup();
      await user.click(await screen.findByRole('checkbox'));
      await user.click(screen.getByRole('button', { name: 'Approve pairing' }));
      expect(f.port.approve).toHaveBeenCalledOnce();
      await act(async () => {
        if (boundary === 'admission') f.revokeAdmission();
        else await f.router.navigate({ to: '/setup' });
      });
      expect(screen.queryByRole('region', { name: 'Agent pairing request' })).toBeNull();
      await act(async () => finish(f.approved));
      expect(screen.queryByText(/Approved\. Return/)).toBeNull();
    } finally {
      await f.dispose();
    }
  }
);

it.each(['secret', 'replacesPrincipalUid'])(
  'native link field %s fails before any approval operation',
  async (field) => {
    const f = await fixture({ [field]: 'must-not-be-accepted' });
    try {
      await screen.findByText(/Invalid pairing link/);
      expect(screen.queryByRole('button', { name: 'Approve pairing' })).toBeNull();
      expect(f.port.approve).not.toHaveBeenCalled();
      expect(f.port.revoke).not.toHaveBeenCalled();
    } finally {
      await f.dispose();
    }
  }
);

it('late retained-space results cannot restore a departed approval view', async () => {
  let finish: (page: SpacePage) => void = () => {};
  const spaces: SpacePort = {
    list: vi.fn(
      () =>
        new Promise<SpacePage>((resolve) => {
          finish = resolve;
        })
    ),
    block: () => {
      throw new Error('No block access');
    },
  };
  const f = await fixture({}, spaces);
  try {
    await screen.findByText('Loading retained spaces…');
    await act(async () => {
      f.revokeAdmission();
    });
    await act(async () => {
      finish({ entries: [], next: null });
    });
    expect(screen.queryByRole('group', { name: 'Assigned block' })).toBeNull();
    expect(f.port.approve).not.toHaveBeenCalled();
  } finally {
    await f.dispose();
  }
});
