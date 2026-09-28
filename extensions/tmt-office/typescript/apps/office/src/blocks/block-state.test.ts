import { describe, expect, it, vi } from 'vitest';
import { createBlockState } from './block-state.js';
import { BlockConflict, builtinFurniture } from './block-contract.js';
import type { Block, BlockPort, Furniture } from './block-contract.js';
const objects: Furniture[] = [builtinFurniture('desk', 0, 0, 0)];
function fixture() {
  let changed: (block: Block | null) => void = () => {};
  let failed = () => {};
  const stop = vi.fn();
  const port: BlockPort = {
    watch: (_id, next, error) => {
      changed = next;
      failed = error;
      return stop;
    },
    apply: vi.fn(async (_id, revision, next) => ({
      revision: revision + 1,
      objects: next,
      updatedAtMs: 1,
    })),
  };
  const state = createBlockState(port, 'a'.repeat(20));
  return {
    state,
    port,
    stop,
    changed: (block: Block | null) => changed(block),
    failed: () => failed(),
  };
}
describe('one block editor lifetime', () => {
  it('late observations cannot roll back a confirmed save or a newer remote revision', async () => {
    const f = fixture();
    f.changed(null);
    f.state.edit(objects);
    await f.state.save();
    f.changed(null);
    expect(f.state.getSnapshot().remote?.revision).toBe(1);
    f.changed({ revision: 3, objects: [], updatedAtMs: 3 });
    f.state.edit(objects);
    f.changed({ revision: 2, objects, updatedAtMs: 2 });
    expect(f.state.getSnapshot()).toMatchObject({
      remote: { revision: 3, objects: [] },
      draft: { revision: 3, objects },
    });
    f.state.dispose();
  });
  it('a newer watch revision survives an older pending save confirmation', async () => {
    const f = fixture();
    f.changed(null);
    f.state.edit(objects);
    let resolve: (block: Block) => void = () => {};
    vi.mocked(f.port.apply).mockImplementation(
      () =>
        new Promise((done) => {
          resolve = done;
        })
    );
    const pending = f.state.save();
    f.changed({ revision: 2, objects: [], updatedAtMs: 2 });
    resolve({ revision: 1, objects, updatedAtMs: 1 });
    await pending;
    expect(f.state.getSnapshot()).toMatchObject({
      remote: { revision: 2, objects: [] },
      draft: null,
      busy: false,
    });
    f.state.dispose();
  });
  it.each(['success', 'failure'])(
    'a terminal watch failure fences late snapshots and save %s',
    async (outcome) => {
      const f = fixture();
      f.changed(null);
      f.state.edit(objects);
      let resolve: (block: Block) => void = () => {};
      let reject: (error: Error) => void = () => {};
      vi.mocked(f.port.apply).mockImplementation(
        () =>
          new Promise((done, fail) => {
            resolve = done;
            reject = fail;
          })
      );
      const pending = f.state.save();
      f.failed();
      const block = { revision: 1, objects, updatedAtMs: 1 };
      f.changed(block);
      if (outcome === 'success') resolve(block);
      else reject(new Error('Late network failure'));
      await pending;
      expect(f.state.getSnapshot()).toMatchObject({
        ready: false,
        remote: null,
        draft: null,
        busy: false,
        error: 'Block unavailable. Reopen the world to retry.',
      });
      f.state.edit(objects);
      await f.state.save();
      expect(f.port.apply).toHaveBeenCalledOnce();
      f.state.dispose();
    }
  );
  it('keeps edits local until explicit save commits the captured revision', async () => {
    const f = fixture();
    f.changed(null);
    f.state.edit(objects);
    expect(f.port.apply).not.toHaveBeenCalled();
    await f.state.save();
    expect(f.port.apply).toHaveBeenCalledExactlyOnceWith('a'.repeat(20), 0, objects);
    expect(f.state.getSnapshot()).toMatchObject({
      draft: null,
      remote: { revision: 1, objects },
      busy: false,
    });
    f.state.dispose();
  });
  it('retains the original draft across remote edits and conflicts', async () => {
    const f = fixture();
    f.changed(null);
    f.state.edit(objects);
    f.changed({ revision: 1, objects: [], updatedAtMs: 1 });
    vi.mocked(f.port.apply).mockRejectedValueOnce(new BlockConflict());
    await f.state.save();
    expect(f.state.getSnapshot().draft).toEqual({ revision: 0, objects });
    expect(f.state.getSnapshot().error).toContain('changed');
    f.state.reset();
    expect(f.state.getSnapshot()).toMatchObject({
      draft: null,
      remote: { revision: 1, objects: [] },
    });
    f.state.dispose();
  });
  it('transport failure preserves a retry and blocks concurrent saves', async () => {
    const f = fixture();
    f.changed(null);
    f.state.edit(objects);
    vi.mocked(f.port.apply).mockRejectedValueOnce(new Error('network'));
    const first = f.state.save();
    await f.state.save();
    await first;
    expect(f.port.apply).toHaveBeenCalledTimes(1);
    expect(f.state.getSnapshot().error).toContain('could not be confirmed');
    await f.state.save();
    expect(f.port.apply).toHaveBeenNthCalledWith(2, 'a'.repeat(20), 0, objects);
    f.state.dispose();
  });
  it('disposal clears content and fences late callbacks and saves', async () => {
    const f = fixture();
    f.changed(null);
    f.state.edit(objects);
    let resolve: (block: Block) => void = () => {};
    vi.mocked(f.port.apply).mockImplementation(
      () =>
        new Promise((done) => {
          resolve = done;
        })
    );
    const pending = f.state.save();
    f.state.dispose();
    f.state.dispose();
    const block = { revision: 1, objects, updatedAtMs: 1 };
    f.changed(block);
    resolve(block);
    await pending;
    expect(f.stop).toHaveBeenCalledOnce();
    expect(f.state.getSnapshot()).toMatchObject({ ready: false, remote: null, draft: null });
  });
  it('permission failure clears content and blocks edits', async () => {
    const f = fixture();
    f.changed({ revision: 1, objects, updatedAtMs: 1 });
    f.state.edit([]);
    f.failed();
    f.state.edit(objects);
    await f.state.save();
    expect(f.state.getSnapshot()).toMatchObject({ ready: false, remote: null, draft: null });
    expect(f.port.apply).not.toHaveBeenCalled();
    f.state.dispose();
  });
});
