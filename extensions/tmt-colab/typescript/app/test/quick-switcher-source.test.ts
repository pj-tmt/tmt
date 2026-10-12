import { createElement } from 'react';
import { renderToString } from 'react-dom/server';
import { expect, it } from 'vite-plus/test';
import {
  SwitcherSourceStore,
  rememberPages,
  useSwitcherSource,
} from '../src/quick-switcher-source.js';
import type { PageSummary, PageTransport, SpaceHome } from '../src/transport.js';

const SPACE = 'a'.repeat(32);
const OTHER = 'b'.repeat(32);
const page = (n: number, title: string): PageSummary => ({
  id: `00000000-0000-4000-8000-${String(n).padStart(12, '0')}`,
  title,
  sharing: 'private',
});
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((a, b) => {
    resolve = a;
    reject = b;
  });
  return { promise, resolve, reject };
}
/** A transport whose list reads are answered by the test, in the order they were asked. */
function transport() {
  const reads: ReturnType<typeof deferred<SpaceHome>>[] = [];
  return {
    reads,
    transport: {
      spaceHome: () => {
        const read = deferred<SpaceHome>();
        reads.push(read);
        return read.promise;
      },
    } as unknown as PageTransport,
  };
}
const home = (...pages: PageSummary[]): SpaceHome => ({ title: 'Space', pages });
const opened =
  (entries: [string, number][] = []) =>
  async () =>
    new Map(entries);
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

it('is idle until opened, loads when nothing is cached, then shows the list and the opened pages', async () => {
  const { transport: t, reads } = transport();
  const first = page(1, 'First');
  const store = new SwitcherSourceStore(t, SPACE, opened([[first.id, 5]]));
  expect(store.getSnapshot()).toEqual({ state: 'idle' });
  expect(reads).toHaveLength(0);
  const seen: string[] = [];
  store.subscribe(() => seen.push(store.getSnapshot().state));
  store.open();
  expect(store.getSnapshot()).toEqual({ state: 'loading' });
  reads[0].resolve(home(first));
  await settle();
  expect(store.getSnapshot()).toEqual({
    state: 'ready',
    pages: [first],
    opened: new Map([[first.id, 5]]),
  });
  store.close();
  expect(store.getSnapshot()).toEqual({ state: 'idle' });
  expect(seen).toEqual(['loading', 'ready', 'idle']);
});

it('shows the cached list at once on the next opening and refreshes it in the background', async () => {
  const { transport: t, reads } = transport();
  const store = new SwitcherSourceStore(t, SPACE, opened());
  store.open();
  reads[0].resolve(home(page(1, 'Old')));
  await settle();
  const cached = store.getSnapshot();
  store.close();
  const later = new SwitcherSourceStore(t, SPACE, opened([[page(2, 'New').id, 9]]));
  // The very first thing it reports is the cached list, without a loading step.
  expect(later.initial()).toBe(cached);
  later.open();
  expect(later.getSnapshot()).toBe(cached);
  reads[1].resolve(home(page(1, 'Old'), page(2, 'New')));
  await settle();
  const refreshed = later.getSnapshot();
  expect(refreshed).toMatchObject({ state: 'ready', pages: [{ title: 'Old' }, { title: 'New' }] });
  expect(later.initial()).toBe(refreshed);
});

it('a failed refresh keeps the cached list; a failed first read is failed, then recovers', async () => {
  const { transport: t, reads } = transport();
  const store = new SwitcherSourceStore(t, SPACE, opened());
  store.open();
  reads[0].reject(new Error('offline'));
  await settle();
  expect(store.getSnapshot()).toEqual({ state: 'failed' });
  store.close();
  store.open();
  reads[1].resolve(home(page(1, 'One')));
  await settle();
  const ready = store.getSnapshot();
  expect(ready.state).toBe('ready');
  store.close();
  store.open();
  reads[2].reject(new Error('offline'));
  await settle();
  expect(store.getSnapshot()).toBe(ready);
});

it('a result that arrives after a close or a reopen is dropped', async () => {
  const { transport: t, reads } = transport();
  const store = new SwitcherSourceStore(t, SPACE, opened());
  store.open();
  store.close();
  reads[0].resolve(home(page(1, 'Late')));
  await settle();
  expect(store.getSnapshot()).toEqual({ state: 'idle' });
  expect(store.initial()).toEqual({ state: 'loading' });
  store.open();
  store.close();
  store.open();
  reads[2].resolve(home(page(2, 'Current')));
  reads[1].resolve(home(page(3, 'Stale')));
  await settle();
  expect(store.getSnapshot()).toMatchObject({ state: 'ready', pages: [{ title: 'Current' }] });
});

it('keeps lists of different spaces and transports apart', async () => {
  const one = transport();
  const two = transport();
  const a = new SwitcherSourceStore(one.transport, SPACE, opened());
  a.open();
  one.reads[0].resolve(home(page(1, 'In A')));
  await settle();
  const sameTransportOtherSpace = new SwitcherSourceStore(one.transport, OTHER, opened());
  expect(sameTransportOtherSpace.initial()).toEqual({ state: 'loading' });
  expect(new SwitcherSourceStore(two.transport, SPACE, opened()).initial()).toEqual({
    state: 'loading',
  });
  expect(new SwitcherSourceStore(one.transport, SPACE, opened()).initial()).toMatchObject({
    state: 'ready',
  });
});

it('a failed read of the opened pages still shows the list; snapshots keep one identity while nothing changes', async () => {
  const { transport: t, reads } = transport();
  const store = new SwitcherSourceStore(t, SPACE, async () => {
    throw new Error('storage');
  });
  store.open();
  const loading = store.getSnapshot();
  expect(store.getSnapshot()).toBe(loading);
  reads[0].resolve(home(page(1, 'One')));
  await settle();
  expect(store.getSnapshot()).toMatchObject({ state: 'ready', opened: new Map() });
  expect(store.getSnapshot()).toBe(store.getSnapshot());
});

function Probe({ open, t, space }: { open: boolean; t: PageTransport; space?: string }) {
  const source = useSwitcherSource(open, t, space);
  return createElement(
    'output',
    null,
    source.state === 'ready' ? `ready:${source.pages.length}` : source.state,
  );
}
it('the hook reports idle while closed, loading without a cache and ready on the first render with one', async () => {
  const { transport: t, reads } = transport();
  const render = (open: boolean, space = SPACE) =>
    renderToString(createElement(Probe, { open, t, space }));
  expect(render(false)).toContain('idle');
  expect(render(true)).toContain('loading');
  expect(reads).toHaveLength(0);
  const store = new SwitcherSourceStore(t, SPACE, opened());
  store.open();
  reads[0].resolve(home(page(1, 'One'), page(2, 'Two')));
  await settle();
  expect(render(true)).toContain('ready:2');
  expect(render(true, OTHER)).toContain('loading');
  expect(render(false)).toContain('idle');
});

it('a list the space home already loaded is shown at once by a switcher that never read one', async () => {
  const { transport: t, reads } = transport();
  const loaded = [page(1, 'From the home'), page(2, 'Also')];
  rememberPages(t, SPACE, loaded);
  const store = new SwitcherSourceStore(t, SPACE, opened([[loaded[0].id, 7]]));
  expect(store.initial()).toEqual({ state: 'ready', pages: loaded, opened: new Map() });
  expect(renderToString(createElement(Probe, { open: true, t, space: SPACE }))).toContain(
    'ready:2',
  );
  expect(renderToString(createElement(Probe, { open: true, t, space: OTHER }))).toContain(
    'loading',
  );
  store.open();
  expect(store.getSnapshot()).toBe(store.initial());
  // It still refreshes in the background, and now knows the pages this device opened.
  reads[0].resolve(home(...loaded, page(3, 'New')));
  await settle();
  expect(store.getSnapshot()).toEqual({
    state: 'ready',
    pages: [...loaded, page(3, 'New')],
    opened: new Map([[loaded[0].id, 7]]),
  });
  // A later home load replaces the pages and keeps what a switcher read of the opened pages.
  rememberPages(t, SPACE, [page(9, 'Later')]);
  expect(new SwitcherSourceStore(t, SPACE, opened()).initial()).toEqual({
    state: 'ready',
    pages: [page(9, 'Later')],
    opened: new Map([[loaded[0].id, 7]]),
  });
});
