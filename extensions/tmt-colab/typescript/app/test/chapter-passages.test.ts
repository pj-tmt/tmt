import { expect, it, vi } from 'vite-plus/test';
import { ChapterPassages, CHAPTER_SURFACE_LIMIT } from '../src/chapter-passages.js';
import { MAX_RENDER_SOURCE_BYTES } from '../src/renderer.js';
import type { PageLease } from '../src/transport.js';

function lease(id: string, events: string[] = []): PageLease {
  return {
    snapshot: { id, title: id, source: '<p>Admitted page</p>', sharing: 'private' },
    release: vi.fn(async () => {
      events.push(`release:${id}`);
    }),
  };
}

it('admits only three intersecting chapters in chapter order, one acquisition at a time', async () => {
  let concurrent = 0;
  let peak = 0;
  const acquired: string[] = [];
  const passages = new ChapterPassages(async (id) => {
    peak = Math.max(peak, ++concurrent);
    acquired.push(id);
    await Promise.resolve();
    concurrent--;
    return lease(id);
  });
  passages.update(['a', 'b', 'c', 'd', 'e'], new Set(['e', 'd', 'c', 'b']));
  await passages.settled();
  expect(CHAPTER_SURFACE_LIMIT).toBe(3);
  expect(acquired).toEqual(['b', 'c', 'd']);
  expect(peak).toBe(1);
  expect(passages.snapshot.map((item) => item.id)).toEqual(['b', 'c', 'd']);
  await passages.dispose();
});

it('joins the retired chapter before its successor acquisition begins', async () => {
  const events: string[] = [];
  let joined!: () => void;
  const boundary = new Promise<void>((resolve) => {
    joined = resolve;
  });
  const first = lease('a');
  first.release = vi.fn(async () => {
    events.push('release:a');
    await boundary;
    events.push('joined:a');
  });
  const passages = new ChapterPassages(async (id) => {
    events.push(`acquire:${id}`);
    return id === 'a' ? first : lease(id, events);
  });
  passages.update(['a', 'b'], new Set(['a']));
  await passages.settled();
  passages.update(['a', 'b'], new Set(['b']));
  await Promise.resolve();
  expect(events).not.toContain('acquire:b');
  joined();
  await passages.settled();
  expect(events).toEqual(['acquire:a', 'release:a', 'joined:a', 'acquire:b']);
  await passages.dispose();
});

it('keeps a busy parent action admitted while hidden and releases it only after it settles', async () => {
  const first = lease('a');
  const acquire = vi.fn(async (id: string) => (id === 'a' ? first : lease(id)));
  const passages = new ChapterPassages(acquire);
  passages.update(['a', 'b'], new Set(['a']));
  await passages.settled();
  const retire = vi.fn();
  passages.frame('a', retire);
  passages.working('a', 'review:section', true);
  passages.update(['a', 'b'], new Set(['b']));
  await Promise.resolve();
  expect(first.release).not.toHaveBeenCalled();
  expect(retire).not.toHaveBeenCalled();
  expect(acquire).toHaveBeenCalledTimes(1);
  expect(passages.snapshot.find((item) => item.id === 'a')?.visible).toBe(false);
  expect(passages.snapshot.find((item) => item.id === 'a')?.busy).toBe(true);
  passages.working('a', 'review:section', false);
  expect(retire).toHaveBeenCalledOnce();
  await passages.settled();
  expect(first.release).toHaveBeenCalledOnce();
  expect(acquire).toHaveBeenCalledTimes(2);
  await passages.dispose();
});

it('hiding the tab cancels in-flight acquisition and joins without starting another page', async () => {
  let entered!: () => void;
  const started = new Promise<void>((resolve) => {
    entered = resolve;
  });
  const acquire = vi.fn(
    (_id: string, signal: AbortSignal) =>
      new Promise<PageLease>((_resolve, reject) => {
        signal.addEventListener('abort', () => reject(signal.reason), { once: true });
        entered();
      }),
  );
  const passages = new ChapterPassages(acquire);
  passages.update(['a', 'b'], new Set(['a', 'b']));
  await started;
  passages.update(['a', 'b'], new Set(['a', 'b']), false);
  await passages.settled();
  expect(acquire).toHaveBeenCalledOnce();
  expect(passages.snapshot).toEqual([]);
  await passages.dispose();
});

it('a refused preview is unavailable, never retried by another identical visibility update', async () => {
  const acquire = vi.fn(async (id: string) => {
    if (id === 'a') throw new Error('Denied');
    return lease(id);
  });
  const passages = new ChapterPassages(acquire);
  passages.update(['a', 'b'], new Set(['a', 'b']));
  await passages.settled();
  expect(passages.snapshot.find((item) => item.id === 'a')).toMatchObject({ state: 'unavailable' });
  expect(passages.snapshot.find((item) => item.id === 'b')).toMatchObject({ state: 'ready' });
  passages.update(['a', 'b'], new Set(['a', 'b']));
  await passages.settled();
  expect(acquire).toHaveBeenCalledTimes(2);
  await passages.dispose();
});

it('a source beyond the existing renderer limit is released and never retained as ready', async () => {
  const base = lease('a');
  const oversized: PageLease = {
    ...base,
    snapshot: { ...base.snapshot, source: 'x'.repeat(MAX_RENDER_SOURCE_BYTES + 1) },
  };
  const passages = new ChapterPassages(async () => oversized);
  passages.update(['a'], new Set(['a']));
  await passages.settled();
  expect(passages.snapshot).toEqual([{ id: 'a', visible: true, state: 'unavailable' }]);
  expect(oversized.release).toHaveBeenCalledOnce();
  await passages.dispose();
});

it('a hidden and immediately reopened chapter cannot reuse its aborted lease', async () => {
  const first = lease('a');
  const second = lease('a');
  const acquire = vi.fn().mockResolvedValueOnce(first).mockResolvedValueOnce(second);
  const passages = new ChapterPassages(acquire);
  passages.update(['a'], new Set(['a']));
  await passages.settled();
  passages.update(['a'], new Set(), false);
  passages.update(['a'], new Set(['a']));
  await passages.settled();
  expect(first.release).toHaveBeenCalledOnce();
  expect(acquire).toHaveBeenCalledTimes(2);
  expect(second.release).not.toHaveBeenCalled();
  await passages.dispose();
});

it('keeps repeated page references as distinct surfaces under the same global limit', async () => {
  const acquire = vi.fn(async (id: string) => lease(id));
  const passages = new ChapterPassages(acquire);
  for (const key of ['first', 'second', 'third', 'fourth']) passages.source(key, 'same-page');
  passages.update(
    ['first', 'second', 'third', 'fourth'],
    new Set(['first', 'second', 'third', 'fourth']),
  );
  await passages.settled();
  expect(acquire.mock.calls.map(([id]) => id)).toEqual(['same-page', 'same-page', 'same-page']);
  expect(passages.snapshot.map((view) => view.id)).toEqual(['first', 'second', 'third']);
  for (const view of passages.snapshot) {
    expect(view.state).toBe('ready');
    if (view.state === 'ready') expect(view.snapshot.id).toBe('same-page');
  }
  await passages.dispose();
});

it('a changed reference retires its frame and joins its busy binding before borrowing the replacement', async () => {
  const events: string[] = [];
  const passages = new ChapterPassages(async (id) => {
    events.push(`acquire:${id}`);
    return lease(id, events);
  });
  passages.source('reference', 'before');
  passages.update(['reference'], new Set(['reference']));
  await passages.settled();
  const retire = vi.fn();
  passages.frame('reference', retire);
  passages.working('reference', 'answer', true);
  passages.source('reference', 'after');
  expect(retire).toHaveBeenCalledWith(true);
  expect(passages.snapshot[0].visible).toBe(false);
  await Promise.resolve();
  expect(events).toEqual(['acquire:before']);
  passages.working('reference', 'answer', false);
  await passages.settled();
  expect(events).toEqual(['acquire:before', 'release:before', 'acquire:after']);
  expect(passages.snapshot[0].visible).toBe(true);
  await passages.dispose();
});
