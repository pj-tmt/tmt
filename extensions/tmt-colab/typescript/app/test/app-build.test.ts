import { afterEach, expect, it, vi } from 'vite-plus/test';
import { createBuildWatch, parseServeRelease, type ServeRelease } from '../src/app-build.js';

const page = (src: string) =>
  `<!doctype html><html><head><script type="module" crossorigin src="${src}"></script></head></html>`;

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

it('compares the build a source names with the served one, and says nothing without one', async () => {
  // A source may answer later (a promise); the in-app one is the module's own stamp.
  let served = '/r/abc/x/colab/assets/index-OLDOLD12.js';
  const mounted = Promise.resolve(served);
  const stale = createBuildWatch({
    loaded: () => mounted,
    served: async () => served,
    now: () => 0,
  });
  expect(await stale.check()).toBe(false);
  const later = createBuildWatch({
    loaded: () => mounted,
    served: async () => (served = '/r/abc/x/colab/assets/index-NEWNEW34.js'),
    now: () => 0,
  });
  expect(await later.check()).toBe(true);
  // Unpaired or unreadable at mount: no running build, so no row and no served read.
  const read = vi.fn(async () => served);
  const quiet = createBuildWatch({ loaded: async () => null, served: read, now: () => 0 });
  expect(await quiet.check()).toBe(false);
  expect(read).not.toHaveBeenCalled();
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

it('reads only a strict serve release', () => {
  expect(parseServeRelease({ running: '0.1.0-alpha.46' })).toEqual({ running: '0.1.0-alpha.46' });
  expect(parseServeRelease({ running: '0.1.0-alpha.46', installed: '0.1.0-alpha.57+b1' })).toEqual({
    running: '0.1.0-alpha.46',
    installed: '0.1.0-alpha.57+b1',
  });
  expect(parseServeRelease({ running: '1', installed: '2', restartFailed: true })).toEqual({
    running: '1',
    installed: '2',
    restartFailed: true,
  });
  for (const bad of [
    null,
    [],
    'x',
    {},
    { running: 1 },
    { running: '' },
    { running: 'a b' },
    { running: 'a\nb' },
    { running: '9'.repeat(65) },
    { running: '1', installed: undefined },
    { running: '1', installed: 2 },
    { running: '1', extra: true },
    { running: '1', restartFailed: true },
    { running: '1', installed: '2', restartFailed: false },
    { running: '1', installed: '2', restartFailed: 'true' },
  ])
    expect(parseServeRelease(bad)).toBeNull();
});

function releaseWatch(
  release: () => Promise<ServeRelease | null>,
  served = '/a/index-XXXXXXXX.js',
) {
  const state = { now: 0, release, reads: 0 };
  const instance = createBuildWatch({
    loaded: () => '/a/index-XXXXXXXX.js',
    served: async () => served,
    release: () => {
      state.reads++;
      return state.release();
    },
    now: () => state.now,
  });
  let notified = 0;
  instance.subscribe(() => notified++);
  return { state, instance, notified: () => notified };
}

it('reports a serve older than the installed release once and stops asking', async () => {
  const w = releaseWatch(async () => ({ running: '46', installed: '57' }));
  expect(await w.instance.check()).toBe(false);
  expect(w.instance.serveOlder()).toEqual({ running: '46', installed: '57' });
  expect(w.notified()).toBe(1);
  w.state.now = 120_000;
  await w.instance.check();
  expect(w.state.reads).toBe(1);
});

it.each([
  ['the same release', async () => ({ running: '57' })],
  ['equal versions', async () => ({ running: '57', installed: '57' })],
  ['no route', async () => null],
  [
    'an unreachable route',
    async () => {
      throw new Error('offline');
    },
  ],
])('is not older for %s, and asks again after a minute', async (_name, read) => {
  const w = releaseWatch(read);
  await w.instance.check();
  expect(w.instance.serveOlder()).toBeNull();
  expect(w.notified()).toBe(0);
  await w.instance.check();
  expect(w.state.reads).toBe(1);
  w.state.now = 61_000;
  w.state.release = async () => ({ running: '46', installed: '57' });
  await w.instance.check();
  expect(w.instance.serveOlder()).toEqual({ running: '46', installed: '57' });
});

it('reports a stale tab and an older serve independently of a failing read', async () => {
  const w = releaseWatch(async () => ({ running: '46', installed: '57' }), '/a/index-YYYYYYYY.js');
  expect(await w.instance.check()).toBe(true);
  expect(w.instance.serveOlder()).not.toBeNull();
  expect(w.notified()).toBe(1);
});

it.each(['/', '/colab', '/pair', '/read/page', '/p/page'])(
  'never probes the native serve release from hosted entry %s',
  async (path) => {
    const url = new URL(path, 'https://demo-colab-reader.web.app');
    const requests: string[] = [];
    vi.stubGlobal('location', url);
    vi.stubGlobal('document', {
      baseURI: url.href,
      querySelectorAll: () => [{ src: new URL('/assets/index-ABCDEFGH.js', url).href }],
    });
    vi.stubGlobal('fetch', async (input: RequestInfo | URL) => {
      const request = new URL(String(input));
      requests.push(request.pathname);
      return new Response(
        request.pathname.endsWith('/api/serve-release')
          ? JSON.stringify({ running: 'old', installed: 'new' })
          : page('/assets/index-ABCDEFGH.js'),
      );
    });
    try {
      const watch = createBuildWatch();
      expect(await watch.check()).toBe(false);
      expect(watch.serveOlder()).toBeNull();
      expect(requests).toEqual([path]);
    } finally {
      vi.unstubAllGlobals();
    }
  },
);

it('still checks the native release for a mounted owner page', async () => {
  const url = new URL('https://door.example/r/abc/x/colab/');
  const requests: string[] = [];
  vi.stubGlobal('location', url);
  vi.stubGlobal('document', {
    baseURI: url.href,
    querySelectorAll: () => [{ src: new URL('assets/index-ABCDEFGH.js', url).href }],
  });
  vi.stubGlobal('fetch', async (input: RequestInfo | URL) => {
    const request = new URL(String(input));
    requests.push(request.pathname);
    return new Response(
      request.pathname.endsWith('/api/serve-release')
        ? JSON.stringify({ running: 'old', installed: 'new' })
        : page('assets/index-ABCDEFGH.js'),
    );
  });
  try {
    const watch = createBuildWatch();
    expect(await watch.check()).toBe(false);
    expect(watch.serveOlder()).toEqual({ running: 'old', installed: 'new' });
    expect(requests).toEqual(['/r/abc/x/colab/', '/r/abc/x/colab/api/serve-release']);
  } finally {
    vi.unstubAllGlobals();
  }
});

/** The Colab module inside the TMT app at `href`: a fresh copy of the modules (the shell host is
 * module state) whose build stamp is `built`, over a door whose mount serves `served`. */
async function moduleIn(options: {
  mount?: URL | null;
  built?: string;
  served: string;
  href?: string;
}) {
  vi.resetModules();
  const href = options.href ?? 'https://door.example/colab/pages/x';
  const requests: string[] = [];
  vi.stubGlobal('location', new URL(href));
  vi.stubGlobal('document', { baseURI: href, querySelectorAll: () => [] });
  if (options.built !== undefined) vi.stubGlobal('__COLAB_MODULE_BUILD__', options.built);
  vi.stubGlobal('fetch', async (input: RequestInfo | URL) => {
    const request = new URL(String(input));
    requests.push(request.pathname);
    return request.pathname.endsWith('/api/serve-release')
      ? new Response('', { status: 404 })
      : new Response(page(`./assets/index-${options.served}.js`));
  });
  const entry = await import('../src/entry.js');
  if (options.mount !== undefined) entry.hostInShell(options.mount);
  const { createBuildWatch } = await import('../src/app-build.js');
  return { watch: createBuildWatch(), requests };
}
const mount = new URL('https://door.example/r/abcdefghijklmnop/x/colab/');
const mountEntry = '/r/abcdefghijklmnop/x/colab/';
afterEach(() => vi.unstubAllGlobals());

it('a paired tab in the TMT app runs the build its module was made with, however late it mounts', async () => {
  // An upgrade landed between the module's import and its mount: every read now names NEWNEW34,
  // and only the module's own stamp says it is OLDOLD12. The check is the only read there is.
  const { watch, requests } = await moduleIn({
    mount,
    built: '/assets/index-OLDOLD12.js',
    served: 'NEWNEW34',
  });
  expect(await watch.check()).toBe(true);
  expect(watch.stale()).toBe(true);
  expect(requests).toEqual([mountEntry, `${mountEntry}api/serve-release`]);
});

it('a paired tab on the served build is not stale', async () => {
  const { watch } = await moduleIn({
    mount,
    built: '/assets/index-SAME1234.js',
    served: 'SAME1234',
  });
  expect(await watch.check()).toBe(false);
  expect(watch.stale()).toBe(false);
});

it('the unpaired preview in the TMT app has no running build and reads nothing', async () => {
  // Its served read would be the shell's own page, which must never be taken for Colab's build.
  const { watch, requests } = await moduleIn({
    mount: null,
    built: '/assets/index-OLDOLD12.js',
    served: 'NEWNEW34',
  });
  expect(await watch.check()).toBe(false);
  expect(requests).toEqual([]);
});

it('a module without a stamp (the dev server) stays quiet', async () => {
  const { watch, requests } = await moduleIn({ mount, served: 'NEWNEW34' });
  expect(await watch.check()).toBe(false);
  expect(requests).not.toContain(mountEntry);
});
