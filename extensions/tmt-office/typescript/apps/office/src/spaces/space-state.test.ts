import { expect, it, vi } from 'vitest';
import { createSpaceState } from './space-state.js';
import type { SpacePage, SpacePort } from './space-contract.js';

function fixture() {
  const port: SpacePort = {
    list: vi.fn(async () => ({ entries: [], next: 'cursor' })),
    block: () => {
      throw new Error('No block access in inventory state.');
    },
  };
  return { port, state: createSpaceState(port, 'world', 'owner') };
}
it('loads one bounded page at a time and refresh restarts rather than merging a second cache', async () => {
  const { port, state } = fixture();
  await state.next();
  expect(port.list).not.toHaveBeenCalled();
  await state.refresh();
  await state.next();
  expect(port.list).toHaveBeenNthCalledWith(2, 'world', 'owner', 'cursor');
  await state.refresh();
  expect(port.list).toHaveBeenNthCalledWith(3, 'world', 'owner', undefined);
  expect(state.getSnapshot().ready).toBe(true);
  state.dispose();
});
it.each(['success', 'failure'])(
  'suppresses concurrent loads and fences late %s after disposal',
  async (outcome) => {
    const { port, state } = fixture();
    let resolve!: (page: SpacePage) => void;
    let reject!: (error: Error) => void;
    vi.mocked(port.list).mockImplementation(
      () =>
        new Promise((yes, no) => {
          resolve = yes;
          reject = no;
        })
    );
    const pending = state.refresh();
    await state.refresh();
    expect(port.list).toHaveBeenCalledOnce();
    state.dispose();
    if (outcome === 'success') resolve({ entries: [], next: 'late' });
    else reject(new Error('late'));
    await pending;
    await state.refresh();
    expect(state.getSnapshot()).toMatchObject({
      entries: [],
      next: null,
      ready: false,
      busy: false,
      error: null,
    });
    expect(port.list).toHaveBeenCalledOnce();
  }
);
it('a failed refresh clears the prior page and allows an explicit retry', async () => {
  const { port, state } = fixture();
  await state.refresh();
  vi.mocked(port.list).mockRejectedValueOnce(new Error('permission-denied'));
  await state.refresh();
  expect(state.getSnapshot()).toMatchObject({ entries: [], next: null, ready: false, busy: false });
  expect(state.getSnapshot().error).toContain('unavailable');
  await state.refresh();
  expect(state.getSnapshot()).toMatchObject({ ready: true, error: null });
  state.dispose();
});
