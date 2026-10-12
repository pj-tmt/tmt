import { afterEach, beforeEach, expect, it, vi } from 'vite-plus/test';
import {
  RECENTLY_OPENED,
  newestOpened,
  noteOpened,
  readRecentlyOpened,
} from '../src/recently-opened.js';

const records = new Map<string, unknown>();
let failure: 'read' | 'write' | undefined;
vi.mock('../src/storage.js', () => ({
  deleteRecord: async (key: string) => {
    records.delete(key);
  },
  record: async (key: string, ...values: unknown[]) => {
    if (failure === (values.length ? 'write' : 'read')) throw new Error('Device storage failed');
    // A real read or write takes a turn, which is what lets two tabs interleave.
    await new Promise((resolve) => setTimeout(resolve, 0));
    if (values.length) records.set(key, structuredClone(values[0]));
    else return records.get(key);
  },
}));
const SPACE = 'a'.repeat(32);
const KEY = `recent-pages:${SPACE}`;
const id = (n: number) => `00000000-0000-4000-8000-${String(n).padStart(12, '0')}`;

/** A Web Lock that queues, as the browser's does. */
function queueingLocks() {
  const queues = new Map<string, Promise<unknown>>();
  return {
    request: (name: string, action: () => Promise<unknown>) => {
      const next = (queues.get(name) ?? Promise.resolve()).then(action, action);
      queues.set(name, next);
      return next;
    },
  };
}
beforeEach(() => {
  vi.stubGlobal('navigator', { locks: queueingLocks() });
});
afterEach(() => {
  records.clear();
  failure = undefined;
  vi.unstubAllGlobals();
});

it('notes an open and reads it back, keyed by space and never beyond this device', async () => {
  expect(await readRecentlyOpened(SPACE)).toEqual(new Map());
  await noteOpened(SPACE, id(1), 1000);
  await noteOpened(SPACE, id(2), 2000);
  await noteOpened(SPACE, id(1), 3000);
  expect(await readRecentlyOpened(SPACE)).toEqual(
    new Map([
      [id(1), 3000],
      [id(2), 2000],
    ]),
  );
  expect([...records.keys()]).toEqual([KEY]);
  expect(await readRecentlyOpened('b'.repeat(32))).toEqual(new Map());
});

it('keeps only the newest hundred, equal times keeping the smaller ID', async () => {
  for (let n = 1; n <= RECENTLY_OPENED + 20; n++) await noteOpened(SPACE, id(n), n * 10);
  const kept = await readRecentlyOpened(SPACE);
  expect(kept.size).toBe(RECENTLY_OPENED);
  expect(kept.has(id(20))).toBe(false);
  expect(kept.has(id(21))).toBe(true);
  expect(kept.get(id(120))).toBe(1200);
  const tied = newestOpened(
    [
      [id(3), 5],
      [id(1), 5],
      [id(2), 5],
    ],
    2,
  );
  expect([...tied.keys()]).toEqual([id(1), id(2)]);
});

it('two tabs noting at once lose neither open', async () => {
  await Promise.all([noteOpened(SPACE, id(1), 10), noteOpened(SPACE, id(2), 20)]);
  expect(await readRecentlyOpened(SPACE)).toEqual(
    new Map([
      [id(2), 20],
      [id(1), 10],
    ]),
  );
  // Without Web Locks it still works, and the lock is only an aid.
  vi.stubGlobal('navigator', {});
  await noteOpened(SPACE, id(3), 30);
  expect((await readRecentlyOpened(SPACE)).get(id(3))).toBe(30);
});

it('ignores anything in the record that is not a page ID with a whole time', async () => {
  records.set(KEY, {
    [id(1)]: 5,
    [id(2)]: -1,
    [id(3)]: 1.5,
    [id(4)]: '7',
    [id(5)]: null,
    'not-a-page-id': 9,
    [id(6)]: Number.MAX_SAFE_INTEGER + 1,
  });
  expect(await readRecentlyOpened(SPACE)).toEqual(new Map([[id(1), 5]]));
  for (const value of [null, 'text', 7, [id(1)], undefined]) {
    records.set(KEY, value);
    expect(await readRecentlyOpened(SPACE)).toEqual(new Map());
  }
  // A damaged record is repaired by the next note, not kept.
  records.set(KEY, { junk: 1, [id(1)]: 5 });
  await noteOpened(SPACE, id(2), 6);
  expect(records.get(KEY)).toEqual({ [id(2)]: 6, [id(1)]: 5 });
});

it('never throws: unavailable storage, an unknown space or a bad page are empty or ignored', async () => {
  failure = 'read';
  expect(await readRecentlyOpened(SPACE)).toEqual(new Map());
  await expect(noteOpened(SPACE, id(1), 1)).resolves.toBeUndefined();
  failure = 'write';
  await expect(noteOpened(SPACE, id(1), 1)).resolves.toBeUndefined();
  failure = undefined;
  expect(records.size).toBe(0);
  for (const [space, page, now] of [
    [undefined, id(1), 1],
    ['short', id(1), 1],
    [SPACE, 'not-a-page', 1],
    [SPACE, id(1), -5],
    [SPACE, id(1), Number.NaN],
  ] as const) {
    await expect(noteOpened(space, page, now)).resolves.toBeUndefined();
    expect(await readRecentlyOpened(space)).toEqual(new Map());
  }
  expect(records.size).toBe(0);
});
