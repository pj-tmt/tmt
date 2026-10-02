import { act, renderHook } from '@testing-library/react';
import { StrictMode } from 'react';
import { afterEach, expect, it, vi } from 'vite-plus/test';
import { officeWorldFixture } from '../../../../../../../typescript/test/support/office-world.js';
import { useWorldEditor } from './use-world-editor.js';
import { WorldConflict, WorldValidationError } from './world-port.js';
import type { WorldSnapshot, WorldWrite } from './world-port.js';

afterEach(() => vi.useRealTimers());
const advance = () =>
  act(async () => {
    await vi.advanceTimersByTimeAsync(350);
  });

function fixture() {
  const initial: WorldSnapshot = officeWorldFixture();
  let saved: WorldSnapshot = initial;
  const port = {
    show: vi.fn(async () => saved),
    save: vi.fn(async (write: WorldWrite) => {
      saved = { ...saved, revision: write.expectedRevision + 1, layout: write.layout };
      return saved;
    }),
  };
  return { initial, port };
}

it('auto-applies changes once after a burst and keeps persisted undo/redo history', async () => {
  vi.useFakeTimers();
  const { initial, port } = fixture();
  const { result, unmount } = renderHook(() => useWorldEditor(initial, port));
  await advance();
  expect(port.save).not.toHaveBeenCalled();
  act(() => {
    result.current.change((world) => ({ ...world, objects: [] }));
    result.current.change((world) => ({
      ...world,
      objects: initial.layout.objects.map((object) => ({
        ...object,
        placement: { ...object.placement, x: 5 },
      })),
    }));
  });
  expect(port.save).not.toHaveBeenCalled();
  await advance();
  expect(port.save).toHaveBeenCalledTimes(1);
  expect(result.current.dirty).toBe(false);
  expect(result.current.canUndo).toBe(true);
  act(() => result.current.undo());
  await advance();
  expect(result.current.world.objects).toEqual([]);
  expect(port.save.mock.calls.at(-1)![0].expectedRevision).toBe(initial.revision + 1);
  act(() => result.current.redo());
  await advance();
  expect(result.current.world.objects[0]!.placement.x).toBe(5);
  expect(result.current.dirty).toBe(false);
  unmount();
});

it('serializes acknowledgements without losing edits made during a write', async () => {
  vi.useFakeTimers();
  const { initial, port } = fixture();
  let finish!: (value: WorldSnapshot) => void;
  port.save.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      })
  );
  const { result, unmount } = renderHook(() => useWorldEditor(initial, port));
  act(() => result.current.change((world) => ({ ...world, objects: [] })));
  await advance();
  expect(result.current.saving).toBe(true);
  act(() => result.current.undo());
  await advance();
  expect(port.save).toHaveBeenCalledTimes(1);
  await act(async () =>
    finish({
      ...initial,
      revision: initial.revision + 1,
      layout: { ...initial.layout, objects: [] },
    })
  );
  expect(result.current.world).toEqual(initial.layout);
  await advance();
  expect(port.save).toHaveBeenCalledTimes(2);
  expect(port.save.mock.calls[1]![0]).toMatchObject({
    expectedRevision: initial.revision + 1,
    layout: initial.layout,
  });
  unmount();
});

it('retains failed changes without automatic retry or observation rebase; reload is explicit', async () => {
  vi.useFakeTimers();
  const { initial, port } = fixture();
  port.save.mockRejectedValueOnce(new WorldConflict());
  const { result, rerender, unmount } = renderHook(
    ({ snapshot }) => useWorldEditor(snapshot, port),
    {
      initialProps: { snapshot: initial },
    }
  );
  act(() => result.current.change((world) => ({ ...world, objects: [] })));
  await advance();
  expect(result.current.blocked).toBe(true);
  rerender({ snapshot: { ...initial, revision: 99 } });
  await advance();
  expect(result.current.world.objects).toEqual([]);
  expect(port.save).toHaveBeenCalledTimes(1);
  expect(port.show).not.toHaveBeenCalled();
  await act(() => result.current.reload());
  expect(result.current.world).toEqual(initial.layout);
  expect(result.current.blocked).toBe(false);
  expect(result.current.canUndo).toBe(false);
  unmount();
});

it('clears a rejected validation state when Undo restores the acknowledged layout', async () => {
  vi.useFakeTimers();
  const { initial, port } = fixture();
  const issue = { objectId: initial.layout.objects[0]!.id, reason: 'outsideFloor' as const };
  port.save.mockRejectedValueOnce(
    new WorldValidationError('Move affected objects before saving.', [issue])
  );
  const { result, unmount } = renderHook(() => useWorldEditor(initial, port));
  act(() => result.current.change((world) => ({ ...world, objects: [] })));
  await advance();
  expect(result.current).toMatchObject({
    blocked: true,
    error: 'Move affected objects before saving.',
    issues: [issue],
  });
  act(() => result.current.undo());
  expect(result.current.world).toEqual(initial.layout);
  expect(result.current).toMatchObject({ blocked: false, dirty: false, issues: [] });
  expect(result.current.error).toBeUndefined();
  expect(result.current.canRedo).toBe(true);
  expect(port.save).toHaveBeenCalledTimes(1);
  act(() => result.current.redo());
  expect(result.current.world.objects).toEqual([]);
  unmount();
});

it('does not clear a conflict block when Undo returns to the acknowledged layout', async () => {
  vi.useFakeTimers();
  const { initial, port } = fixture();
  port.save.mockRejectedValueOnce(new WorldConflict());
  const { result, unmount } = renderHook(() => useWorldEditor(initial, port));
  act(() => result.current.change((world) => ({ ...world, objects: [] })));
  await advance();
  act(() => result.current.undo());
  expect(result.current.world).toEqual(initial.layout);
  expect(result.current.blocked).toBe(true);
  expect(result.current.error).toBeDefined();
  expect(port.save).toHaveBeenCalledTimes(1);
  unmount();
});

it('explicit retry retains the revision fence', async () => {
  vi.useFakeTimers();
  const { initial, port } = fixture();
  port.save.mockRejectedValueOnce(new Error('offline'));
  const { result, unmount } = renderHook(() => useWorldEditor(initial, port));
  act(() => result.current.change((world) => ({ ...world, objects: [] })));
  await advance();
  await act(() => result.current.retry());
  expect(result.current.dirty).toBe(false);
  expect(port.save.mock.calls[1]![0].expectedRevision).toBe(initial.revision);
  unmount();
});

it('does not write a cancelled debounce and accepts only newer clean observations', async () => {
  vi.useFakeTimers();
  const { initial, port } = fixture();
  const { result, rerender, unmount } = renderHook(
    ({ snapshot }) => useWorldEditor(snapshot, port),
    {
      initialProps: { snapshot: initial },
    }
  );
  act(() => {
    result.current.change((world) => ({ ...world, objects: [] }));
    result.current.undo();
  });
  await advance();
  expect(port.save).not.toHaveBeenCalled();
  const latest = {
    ...initial,
    revision: initial.revision + 2,
    layout: { ...initial.layout, objects: [] },
  };
  rerender({ snapshot: latest });
  expect(result.current.world).toEqual(latest.layout);
  rerender({ snapshot: { ...initial } });
  expect(result.current.saved).toEqual(latest);
  unmount();
});

it('preserves external additions and native revision while undoing an acknowledged local move', async () => {
  vi.useFakeTimers();
  const { initial, port } = fixture();
  const { result, rerender, unmount } = renderHook(
    ({ snapshot }: { snapshot: WorldSnapshot }) => useWorldEditor(snapshot, port),
    {
      initialProps: { snapshot: initial },
    }
  );
  act(() =>
    result.current.change((world) => ({
      ...world,
      objects: world.objects.map((object) => ({
        ...object,
        placement: { ...object.placement, x: 8 },
      })),
    }))
  );
  await advance();
  const other = { ...initial.layout.objects[0]!, id: '30000000-0000-4000-8000-000000000002' };
  rerender({
    snapshot: {
      ...initial,
      revision: 9,
      layout: {
        ...result.current.world,
        objects: [...result.current.world.objects, other],
      },
    },
  });
  expect(result.current.canUndo).toBe(true);
  act(() => result.current.undo());
  await advance();
  expect(port.save.mock.calls.at(-1)![0]).toMatchObject({
    expectedRevision: 9,
    layout: { ...initial.layout, objects: [...initial.layout.objects, other] },
  });
  unmount();
});

it('keeps the Yjs owner usable through StrictMode effect replay', async () => {
  vi.useFakeTimers();
  const { initial, port } = fixture();
  const { result, unmount } = renderHook(() => useWorldEditor(initial, port), {
    wrapper: StrictMode,
  });
  act(() => result.current.change((world) => ({ ...world, objects: [] })));
  await advance();
  expect(result.current.canUndo).toBe(true);
  act(() => result.current.undo());
  await advance();
  expect(result.current.world).toEqual(initial.layout);
  expect(port.save).toHaveBeenCalledTimes(2);
  unmount();
});

it('reconsiders a newer observation received while the local save was in flight', async () => {
  vi.useFakeTimers();
  const { initial, port } = fixture();
  let finish!: (snapshot: WorldSnapshot) => void;
  port.save.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      })
  );
  const { result, rerender, unmount } = renderHook(
    ({ snapshot }) => useWorldEditor(snapshot, port),
    {
      initialProps: { snapshot: initial },
    }
  );
  act(() => result.current.change((world) => ({ ...world, objects: [] })));
  await advance();
  const pending = result.current.world;
  const external = { ...initial, revision: 10 };
  rerender({ snapshot: external });
  expect(result.current.world).toEqual(pending);
  await act(async () => finish({ ...initial, revision: 2, layout: pending }));
  expect(result.current.world).toEqual(external.layout);
  expect(result.current.saved.revision).toBe(10);
  await advance();
  expect(port.save).toHaveBeenCalledTimes(1);
  unmount();
});
