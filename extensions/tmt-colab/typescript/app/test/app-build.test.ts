import { expect, it } from 'vite-plus/test';
import { createBuildWatch, entryOf, isNewerBuild } from '../src/app-build.js';

const base = new URL('https://door.example/r/abc/x/colab/');
const page = (src: string) =>
  `<!doctype html><html><head><script type="module" crossorigin src="${src}"></script></head></html>`;

it('reads the hashed entry module of a served page', () => {
  expect(entryOf(page('./assets/index-AAA.js'), base)).toBe('/r/abc/x/colab/assets/index-AAA.js');
  expect(entryOf('<p>no script</p>', base)).toBeNull();
  expect(entryOf('<script src="x.js"></script>', base)).toBeNull();
});

function watch(initial: { loaded: string | null; served: string | null | Error }) {
  const state = { ...initial, now: 0, reads: 0 };
  const instance = createBuildWatch({
    loaded: () => state.loaded,
    served: async () => {
      state.reads++;
      if (state.served instanceof Error) throw state.served;
      return state.served;
    },
    now: () => state.now,
  });
  let notified = 0;
  instance.subscribe(() => notified++);
  return { state, instance, notified: () => notified };
}

it('is stale only when two known builds differ, and tells subscribers once', async () => {
  const w = watch({ loaded: '/assets/index-OLDOLD12.js', served: '/assets/index-NEWNEW34.js' });
  expect(await w.instance.check()).toBe(true);
  expect(w.instance.stale()).toBe(true);
  expect(w.notified()).toBe(1);
  // Once stale it stays stale without another read.
  expect(await w.instance.check()).toBe(true);
  expect(w.state.reads).toBe(1);
});

it('tells an upgrade from a different kind of page', () => {
  expect(isNewerBuild('/a/index-AAAAAAAA.js', '/a/index-BBBBBBBB.js')).toBe(true);
  expect(isNewerBuild('/a/index-AAAAAAAA.js', '/a/index-AAAAAAAA.js')).toBe(false);
  // Pairing guidance (unhashed recovery.js) or the reader entry is not a newer app build.
  expect(isNewerBuild('/a/index-AAAAAAAA.js', '/a/assets/recovery.js')).toBe(false);
  expect(isNewerBuild('/a/index-AAAAAAAA.js', '/a/assets/reader.js')).toBe(false);
  expect(isNewerBuild('/src/main.tsx', '/src/main.tsx')).toBe(false);
  expect(isNewerBuild('/a/index-AAAAAAAA.js', '/a/other-BBBBBBBB.js')).toBe(false);
});

it.each([
  ['the same build', { loaded: '/a/index-XXXXXXXX.js', served: '/a/index-XXXXXXXX.js' }],
  ['an unreadable served page', { loaded: '/a/index-XXXXXXXX.js', served: null }],
  ['an offline server', { loaded: '/a/index-XXXXXXXX.js', served: new Error('offline') }],
  [
    'pairing guidance served to an unpaired browser',
    { loaded: '/a/index-XXXXXXXX.js', served: '/a/assets/recovery.js' },
  ],
  ['a tab without a module entry', { loaded: null, served: '/a/index-YYYYYYYY.js' }],
])('is not stale for %s', async (_name, sources) => {
  const w = watch(sources);
  expect(await w.instance.check()).toBe(false);
  expect(w.notified()).toBe(0);
});

it('reads the served build at most once a minute and retries after an offline blip', async () => {
  const w = watch({ loaded: '/a/index-XXXXXXXX.js', served: new Error('offline') });
  await w.instance.check();
  await w.instance.check();
  expect(w.state.reads).toBe(1);
  w.state.now = 61_000;
  w.state.served = '/a/index-YYYYYYYY.js';
  expect(await w.instance.check()).toBe(true);
  expect(w.state.reads).toBe(2);
});
