import { expect, it } from 'vite-plus/test';
import { ShortcutStore } from '../src/shortcut-store.js';
function memory() {
  const data = new Map<string, string>();
  return {
    data,
    getItem: (key: string) => data.get(key) ?? null,
    setItem: (key: string, value: string) => {
      data.set(key, value);
    },
  };
}
it('mutations read the latest per-space durable record', () => {
  const storage = memory(),
    a = new ShortcutStore('one', storage),
    b = new ShortcutStore('one', storage);
  const first = a.save({ name: 'Design', search: { project: ['design'] } });
  const second = b.save({ name: 'Needs me', search: { needs: true } });
  expect(a.read().items.map((x) => x.id)).toEqual([first.id, second.id]);
  a.save({ ...first, name: 'Review' });
  b.move(second.id, -1);
  expect(a.read().items.map((x) => x.name)).toEqual(['Needs me', 'Review']);
  b.delete(first.id);
  expect(a.read().items).toHaveLength(1);
  expect(new ShortcutStore('other', storage).read().items).toEqual([]);
});
it('migrates actual explicit pins once in order without changing legacy pins', () => {
  const storage = memory(),
    store = new ShortcutStore('one', storage);
  store.migrate(undefined);
  expect(store.read().migrated).toBe(false);
  const pins = { automatic: false, names: ['design', 'review'] };
  store.migrate(pins);
  store.migrate(pins);
  expect(store.read().items.map((x) => x.search.project)).toEqual([['design'], ['review']]);
  expect(pins).toEqual({ automatic: false, names: ['design', 'review'] });
  const auto = new ShortcutStore('automatic', storage);
  auto.migrate({ automatic: true, names: ['recent'] });
  expect(auto.read().items).toEqual([]);
});
it('preserves corrupt/newer data and reports failed writes without an in-memory save', () => {
  for (const raw of ['broken', JSON.stringify({ version: 2, items: [] })]) {
    const storage = memory(),
      store = new ShortcutStore('one', storage);
    storage.data.set(store.key, raw);
    expect(() => store.save({ name: 'New', search: {} })).toThrow();
    expect(storage.data.get(store.key)).toBe(raw);
  }
  const storage = memory(),
    store = new ShortcutStore('one', storage);
  storage.setItem = () => {
    throw new Error('quota');
  };
  expect(() => store.save({ name: 'New', search: {} })).toThrow('quota');
  expect(store.read().items).toEqual([]);
});

it('never commits unreadable filters or a migration marker after failed persistence', () => {
  const storage = memory(),
    store = new ShortcutStore('retry', storage);
  const write = storage.setItem;
  storage.setItem = () => {
    throw new Error('quota');
  };
  expect(() => store.migrate({ automatic: false, names: ['design'] })).toThrow();
  expect(store.read().migrated).toBe(false);
  storage.setItem = write;
  store.migrate({ automatic: false, names: ['design'] });
  expect(store.read().items).toHaveLength(1);
  expect(() =>
    store.save({
      name: 'Too many',
      search: { scope: 'scope', project: ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h'] },
    }),
  ).toThrow();
  expect(store.read().items).toHaveLength(1);
});

it('moves a shortcut to the target row index in either direction using the latest record', () => {
  const storage = memory();
  const first = new ShortcutStore('one', storage);
  const second = new ShortcutStore('one', storage);
  const a = first.save({ name: 'A', search: {} });
  const b = first.save({ name: 'B', search: {} });
  const c = first.save({ name: 'C', search: {} });
  second.moveTo(a.id, c.id);
  expect(first.read().items.map((item) => item.name)).toEqual(['B', 'C', 'A']);
  first.moveTo(a.id, b.id);
  expect(second.read().items.map((item) => item.name)).toEqual(['A', 'B', 'C']);
  second.delete(b.id);
  first.moveTo(a.id, b.id);
  first.moveTo('unknown', c.id);
  expect(first.read().items.map((item) => item.name)).toEqual(['A', 'C']);
});
