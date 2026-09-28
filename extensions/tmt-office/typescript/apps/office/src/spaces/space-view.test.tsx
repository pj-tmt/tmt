import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { BlockContext } from '../blocks/block-view.js';
import { builtinFurniture } from '../blocks/block-contract.js';
import type { Block, BlockPort } from '../blocks/block-contract.js';
import { OfficeSpaces, SpaceContext } from './space-view.js';
import type { SpacePort } from './space-contract.js';

it('switching targets disposes the old editor and late saves cannot replace home', async () => {
  const blockId = '00000000-0000-4000-8000-000000000001';
  const homeStop = vi.fn();
  const retainedStop = vi.fn();
  let finish!: (block: Block) => void;
  const home: BlockPort = {
    watch: (_world, next) => {
      next({ revision: 3, objects: [builtinFurniture('plant', 1, 1, 0)], updatedAtMs: 1 });
      return homeStop;
    },
    apply: vi.fn(async () => {
      throw new Error('Home must not be written.');
    }),
  };
  const retained: BlockPort = {
    watch: (_world, next) => {
      next(null);
      return retainedStop;
    },
    apply: vi.fn(
      () =>
        new Promise<Block>((resolve) => {
          finish = resolve;
        })
    ),
  };
  const port: SpacePort = {
    list: async () => ({
      entries: [
        {
          principalUid: 'principal',
          identityId: 'identity',
          installationId: 'installation',
          blockId,
          capabilities: ['layout.read'],
          enabled: false,
          expiresAtMs: 1000,
        },
      ],
      next: null,
    }),
    block: vi.fn(() => retained),
  };
  const world = { id: 'a'.repeat(20), ownerUid: 'owner', name: 'Studio', createdAtMs: 1 };
  const view = render(
    <SpaceContext value={port}>
      <BlockContext value={home}>
        <OfficeSpaces world={world} />
      </BlockContext>
    </SpaceContext>
  );
  try {
    const user = userEvent.setup();
    await user.click(await screen.findByRole('button', { name: `Open block ${blockId}` }));
    expect(port.block).toHaveBeenCalledExactlyOnceWith(blockId);
    expect(homeStop).toHaveBeenCalledOnce();
    expect(screen.getByText('No saved layout yet')).toBeTruthy();
    await user.click(screen.getByRole('button', { name: 'Add desk' }));
    await user.click(screen.getByRole('button', { name: 'Save layout' }));
    expect(retained.apply).toHaveBeenCalledExactlyOnceWith(world.id, 0, [
      builtinFurniture('desk', 14, 14, 0),
    ]);
    await user.click(screen.getByRole('button', { name: 'Open home block' }));
    expect(retainedStop).toHaveBeenCalledOnce();
    await act(async () => finish({ revision: 1, objects: [], updatedAtMs: 1 }));
    expect(screen.getByText('Saved · revision 3')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Plant 1' })).toBeTruthy();
    expect(home.apply).not.toHaveBeenCalled();
  } finally {
    view.unmount();
  }
});
