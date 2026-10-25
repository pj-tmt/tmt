import { expect, it, vi } from 'vite-plus/test';
import {
  createSearchRegistry,
  searchSnapshotProblem,
  type SearchItem,
  type SearchProvider,
  type SearchSnapshot,
} from '../src/search.js';

const item = (key: string, extra: Partial<SearchItem> = {}): SearchItem => ({
  key,
  label: key.toUpperCase(),
  href: `/m/${key}`,
  ...extra,
});
const ready = (...keys: string[]): SearchSnapshot => ({
  state: 'ready',
  items: keys.map((key) => item(key)),
});

/** A provider whose snapshot the test changes, counting the reads and the live subscriptions. */
function fixture(key: string, first: SearchSnapshot = ready()) {
  let snapshot = first;
  const listeners = new Set<() => void>();
  const reads = vi.fn(() => snapshot);
  const provider: SearchProvider = {
    key,
    label: key.toUpperCase(),
    getSnapshot: reads,
    subscribe(listener) {
      listeners.add(listener);
      return () => void listeners.delete(listener);
    },
  };
  return {
    provider,
    reads,
    listeners,
    set(next: SearchSnapshot) {
      snapshot = next;
      for (const listener of Array.from(listeners)) listener();
    },
  };
}

it('lists the groups in registration order and tells unavailable from a confirmed empty', () => {
  const registry = createSearchRegistry();
  registry.register(fixture('nav', ready('home', 'colab')).provider);
  registry.register(fixture('ops', { state: 'unavailable' }).provider);
  registry.register(fixture('remote', ready()).provider);
  expect(registry.groups().map((group) => [group.key, group.label, group.state])).toEqual([
    ['nav', 'NAV', 'ready'],
    ['ops', 'OPS', 'unavailable'],
    ['remote', 'REMOTE', 'ready'],
  ]);
  expect(registry.groups().map((group) => group.items.map((entry) => entry.key))).toEqual([
    ['home', 'colab'],
    [],
    [],
  ]);
});

it('follows a provider, reads each change once and keeps the array between changes', () => {
  const registry = createSearchRegistry();
  const colab = fixture('colab', ready('a'));
  registry.register(colab.provider);
  const listener = vi.fn();
  registry.subscribe(listener);
  const before = registry.groups();
  expect(registry.groups()).toBe(before);
  expect(colab.reads).toHaveBeenCalledTimes(2);
  colab.set(ready('a', 'b'));
  expect(listener).toHaveBeenCalledTimes(1);
  const after = registry.groups();
  expect(after).not.toBe(before);
  expect(registry.groups()).toBe(after);
  expect(after[0]!.items.map((entry) => entry.key)).toEqual(['a', 'b']);
  // One read admitted the registration and one built each array: two arrays, three reads.
  expect(colab.reads).toHaveBeenCalledTimes(3);
});

it('withdraws a provider on its disposal and ignores it afterwards', () => {
  const registry = createSearchRegistry();
  const ops = fixture('ops', ready('agent'));
  const withdraw = registry.register(ops.provider);
  const listener = vi.fn();
  registry.subscribe(listener);
  expect(registry.find('ops', 'agent')?.label).toBe('AGENT');
  withdraw();
  withdraw();
  expect(listener).toHaveBeenCalledTimes(1);
  expect(registry.groups()).toEqual([]);
  expect(registry.find('ops', 'agent')).toBeUndefined();
  expect(ops.listeners.size).toBe(0);
  // A key freed by a withdrawal can be registered again, and the old withdrawal stays stale.
  const next = fixture('ops', ready('squad'));
  registry.register(next.provider);
  withdraw();
  expect(registry.groups().map((group) => group.items.map((entry) => entry.key))).toEqual([
    ['squad'],
  ]);
});

it('finds only the item the latest snapshot still has', () => {
  const registry = createSearchRegistry();
  const colab = fixture('colab', ready('a', 'b'));
  registry.register(colab.provider);
  expect(registry.find('colab', 'b')?.href).toBe('/m/b');
  colab.set({ state: 'ready', items: [item('a', { href: '/m/a2' })] });
  expect(registry.find('colab', 'b')).toBeUndefined();
  expect(registry.find('colab', 'a')?.href).toBe('/m/a2');
  expect(registry.find('nope', 'a')).toBeUndefined();
  colab.set({ state: 'unavailable' });
  expect(registry.find('colab', 'a')).toBeUndefined();
});

it('refuses a second provider for a key and an invalid first snapshot, changing nothing', () => {
  const registry = createSearchRegistry();
  const first = fixture('colab', ready('a'));
  registry.register(first.provider);
  const listener = vi.fn();
  registry.subscribe(listener);
  expect(() => registry.register(fixture('colab', ready('z')).provider)).toThrow(
    'already registered for colab',
  );
  const bad = fixture('ops', {
    state: 'ready',
    items: [item('a', { href: 'https://example.com' })],
  });
  expect(() => registry.register(bad.provider)).toThrow('Search provider ops: item a has no route');
  expect(bad.listeners.size).toBe(0);
  expect(listener).not.toHaveBeenCalled();
  expect(registry.groups().map((group) => group.key)).toEqual(['colab']);
  expect(() => registry.register({ ...first.provider, key: '' })).toThrow('needs a key');
});

it('never shows rows the provider did not admit', () => {
  const registry = createSearchRegistry();
  const colab = fixture('colab', ready('a'));
  registry.register(colab.provider);
  colab.set({ state: 'ready', items: [item('a'), item('a')] });
  expect(registry.groups()[0]).toMatchObject({ state: 'unavailable', items: [] });
  colab.set(ready('a'));
  expect(registry.groups()[0]).toMatchObject({ state: 'ready' });
  colab.reads.mockImplementationOnce(() => {
    throw new Error('unreadable');
  });
  colab.set(ready('b'));
  expect(registry.groups()[0]).toMatchObject({ state: 'unavailable', items: [] });
});

it('admits only routes of this origin and well-formed items', () => {
  const route = (href: string) =>
    searchSnapshotProblem({ state: 'ready', items: [item('a', { href })] });
  for (const href of ['/', '/colab/p/abc', '/ops/#agents', '/settings/?section=modules#modules'])
    expect(route(href), href).toBeUndefined();
  for (const href of [
    '',
    'colab',
    'https://example.com/',
    'javascript:alert(1)',
    '//example.com',
    '/\\example.com',
    '/a b',
    '/a\nb',
    '/a\u0000b',
  ])
    expect(route(href), JSON.stringify(href)).toContain('item a has no route');
  expect(searchSnapshotProblem({ state: 'unavailable' })).toBeUndefined();
  expect(searchSnapshotProblem({ state: 'ready', items: [] })).toBeUndefined();
  const full = item('a', { detail: 'Page · design', words: ['alpha'], icon: 'file-text' });
  expect(searchSnapshotProblem({ state: 'ready', items: [full] })).toBeUndefined();
  for (const bad of [
    null,
    { state: 'loading' },
    { state: 'ready' },
    { state: 'ready', items: [null] },
    { state: 'ready', items: [{ ...item('a'), key: '' }] },
    { state: 'ready', items: [{ ...item('a'), label: '' }] },
    { state: 'ready', items: [{ ...item('a'), detail: 3 }] },
    { state: 'ready', items: [{ ...item('a'), words: ['x', 3] }] },
    { state: 'ready', items: [{ ...item('a'), icon: 3 }] },
    { state: 'ready', items: [item('a'), item('a')] },
  ])
    expect(searchSnapshotProblem(bad), JSON.stringify(bad)).toBeTypeOf('string');
});

it('is inert after disposal and releases every provider', () => {
  const registry = createSearchRegistry();
  const colab = fixture('colab', ready('a'));
  registry.register(colab.provider);
  const listener = vi.fn();
  registry.subscribe(listener);
  registry.dispose();
  registry.dispose();
  expect(colab.listeners.size).toBe(0);
  expect(registry.groups()).toEqual([]);
  colab.set(ready('a', 'b'));
  const late = fixture('ops');
  registry.register(late.provider)();
  registry.subscribe(listener)();
  expect(late.listeners.size).toBe(0);
  expect(registry.groups()).toEqual([]);
  expect(listener).not.toHaveBeenCalled();
});

it('replaces a session by withdrawing its providers and registering the next ones', () => {
  const registry = createSearchRegistry();
  const withdrawals = [
    registry.register(fixture('colab', ready('old')).provider),
    registry.register(fixture('ops', ready('old')).provider),
  ];
  for (const withdraw of withdrawals) withdraw();
  registry.register(fixture('colab', ready('new')).provider);
  expect(registry.groups().map((group) => [group.key, group.items[0]?.key])).toEqual([
    ['colab', 'new'],
  ]);
  expect(registry.find('ops', 'old')).toBeUndefined();
});
