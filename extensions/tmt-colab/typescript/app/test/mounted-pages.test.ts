import { expect, it, vi } from 'vite-plus/test';
import { MountedPages, type MountedPage } from '../src/mounted-pages.js';
import type { PageView } from '../src/transport.js';

function page(id: string) {
  const subscriptions = new Set<(view: PageView) => void>();
  const binding = {
    closed: false as boolean,
    blocked: false as boolean,
    foreground: vi.fn((_active: boolean) => {}),
    snapshot: vi.fn(async () => ({
      id,
      title: id,
      source: '<p>Admitted</p>',
      sharing: 'private' as const,
    })),
    subscribe: vi.fn((publish: (view: PageView) => void) => {
      subscriptions.add(publish);
      return () => subscriptions.delete(publish);
    }),
    edit: vi.fn(async () => ({ merged: false })),
    export: vi.fn(),
    close: vi.fn(() => {
      binding.closed = true;
    }),
    closeAndJoin: vi.fn(async () => {
      binding.closed = true;
    }),
  } satisfies MountedPage;
  return binding;
}

it('a chapter and foreground page share one admitted binding; releasing either cannot close the other', async () => {
  const live = page('page');
  const create = vi.fn(async () => live);
  const pages = new MountedPages(create);
  const chapter = await pages.acquire('page', 'chapter');
  const foreground = await pages.acquire('page', 'page');
  expect(create).toHaveBeenCalledOnce();
  expect(live.foreground.mock.calls.map(([active]) => active)).toContain(true);
  await chapter.release();
  expect(live.closeAndJoin).not.toHaveBeenCalled();
  await foreground.release();
  expect(live.closeAndJoin).toHaveBeenCalledOnce();
});

it('removing the foreground consumer restores passive reconnects while a chapter remains', async () => {
  const live = page('page');
  const pages = new MountedPages(async () => live);
  const chapter = await pages.acquire('page', 'chapter');
  const foreground = await pages.acquire('page', 'page');
  await foreground.release();
  expect(live.foreground).toHaveBeenLastCalledWith(false);
  expect(live.closeAndJoin).not.toHaveBeenCalled();
  await chapter.release();
});

it('a replacement page cannot begin before the previous last consumer cleanup joins', async () => {
  const first = page('first');
  let joined!: () => void;
  const boundary = new Promise<void>((resolve) => {
    joined = resolve;
  });
  first.closeAndJoin.mockImplementation(async () => {
    await boundary;
    first.closed = true;
  });
  const second = page('second');
  const create = vi.fn(async (id: string) => (id === 'first' ? first : second));
  const pages = new MountedPages(create);
  const lease = await pages.acquire('first', 'chapter');
  const retiring = lease.release();
  const replacement = pages.acquire('second', 'chapter');
  await Promise.resolve();
  expect(create).toHaveBeenCalledTimes(1);
  joined();
  await retiring;
  const next = await replacement;
  expect(create).toHaveBeenCalledTimes(2);
  await next.release();
});

it('an aborted consumer cannot retire a binding another consumer still owns', async () => {
  const live = page('page');
  const pages = new MountedPages(async () => live);
  const controller = new AbortController();
  const chapter = await pages.acquire('page', 'chapter', controller.signal);
  const foreground = await pages.acquire('page', 'page');
  controller.abort();
  await chapter.release();
  expect(live.closeAndJoin).not.toHaveBeenCalled();
  await foreground.release();
  expect(live.closeAndJoin).toHaveBeenCalledOnce();
});

it('cleanup failure fences successors instead of silently opening another connection', async () => {
  const live = page('first');
  const failure = new Error('Cleanup unconfirmed');
  live.closeAndJoin.mockRejectedValue(failure);
  const create = vi.fn(async () => live);
  const pages = new MountedPages(create);
  const lease = await pages.acquire('first', 'chapter');
  await expect(lease.release()).rejects.toBe(failure);
  await expect(pages.acquire('second', 'chapter')).rejects.toBe(failure);
  expect(create).toHaveBeenCalledOnce();
});

it('same-turn subscription replay keeps the lease, and its last unsubscribe releases it', async () => {
  const live = page('page');
  const pages = new MountedPages(async () => live);
  const lease = await pages.acquire('page', 'chapter');
  const first = lease.snapshot.binding!.subscribe(
    () => {},
    () => {},
  );
  first();
  const second = lease.snapshot.binding!.subscribe(
    () => {},
    () => {},
  );
  await Promise.resolve();
  expect(live.closeAndJoin).not.toHaveBeenCalled();
  second();
  await Promise.resolve();
  await lease.release();
  expect(live.closeAndJoin).toHaveBeenCalledOnce();
});

it('an already-aborted request starts no discovery or connection', async () => {
  const create = vi.fn(async () => page('page'));
  const pages = new MountedPages(create);
  const controller = new AbortController();
  controller.abort();
  await expect(pages.acquire('page', 'chapter', controller.signal)).rejects.toMatchObject({
    name: 'AbortError',
  });
  expect(create).not.toHaveBeenCalled();
});

it('a replaced Session retires and joins the old page before admitting a new borrower', async () => {
  let current = 'first';
  const first = page('page');
  const second = page('page');
  const create = vi.fn(async () => (current === 'first' ? first : second));
  const pages = new MountedPages(create, (live) =>
    current === 'first' ? live === first : live === second,
  );
  const old = await pages.acquire('page', 'chapter');
  current = 'second';
  const next = await pages.acquire('page', 'page');
  expect(create).toHaveBeenCalledTimes(2);
  expect(first.closeAndJoin).toHaveBeenCalledOnce();
  expect(second.closeAndJoin).not.toHaveBeenCalled();
  await old.release();
  expect(first.closeAndJoin).toHaveBeenCalledOnce();
  expect(second.closeAndJoin).not.toHaveBeenCalled();
  await next.release();
});

it('a Session replaced during the final snapshot cannot publish a stale borrowed view', async () => {
  const live = page('page');
  let current = true;
  live.snapshot.mockImplementationOnce(async () => ({
    id: 'page',
    title: 'Page',
    source: '',
    sharing: 'private',
  }));
  live.snapshot.mockImplementationOnce(async () => {
    current = false;
    return { id: 'page', title: 'Stale', source: '', sharing: 'private' };
  });
  const pages = new MountedPages(
    async () => live,
    () => current,
  );
  await expect(pages.acquire('page', 'chapter')).rejects.toThrow('Page released');
  expect(live.closeAndJoin).toHaveBeenCalledOnce();
});

it('a blocked passive binding is joined before a new foreground open can borrow its page', async () => {
  const stale = page('page');
  const fresh = page('page');
  const create = vi.fn().mockResolvedValueOnce(stale).mockResolvedValueOnce(fresh);
  const pages = new MountedPages(create);
  const chapter = await pages.acquire('page', 'chapter');
  stale.blocked = true;
  const foreground = await pages.acquire('page', 'page');
  expect(stale.closeAndJoin).toHaveBeenCalledOnce();
  expect(create).toHaveBeenCalledTimes(2);
  expect(foreground.snapshot.binding?.edit).not.toBe(chapter.snapshot.binding?.edit);
  await chapter.release();
  expect(fresh.closeAndJoin).not.toHaveBeenCalled();
  await foreground.release();
  expect(fresh.closeAndJoin).toHaveBeenCalledOnce();
});
