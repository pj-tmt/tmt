import { afterEach, expect, it, vi } from 'vite-plus/test';

// The export's standalone copy loads the same runtime the renderer frame does, from the same base
// (#1693). Only the opt-in parse needs a browser (DOMParser), so the marker check is replaced; the
// base, the fetches and the outcome are the real modules.
vi.mock('../src/page-runtime.js', async (original) => ({
  ...(await original<typeof import('../src/page-runtime.js')>()),
  wantsRuntime: () => true,
}));

const door = 'https://door.test';
const mount = `${door}/r/abcdefghijklmnop/x/colab/`;
/** A fresh copy of the modules (the shell host and the runtime cache are module state) in a page
 * at `href` whose document base is `baseURI`; `shell` is what the TMT app host would register. */
async function exportIn(href: string, baseURI: string, shell?: URL | null, ok = true) {
  vi.resetModules();
  const url = new URL(href);
  const fetched: string[] = [];
  vi.stubGlobal('location', { href, origin: url.origin, pathname: url.pathname });
  vi.stubGlobal('document', { baseURI, querySelector: () => null });
  vi.stubGlobal('fetch', async (target: string) => {
    fetched.push(target);
    return new Response(ok ? `text of ${target}` : 'gone', { status: ok ? 200 : 404 });
  });
  const entry = await import('../src/entry.js');
  if (shell !== undefined) entry.hostInShell(shell);
  const { standaloneRequest } = await import('../src/export-runtime.js');
  const request = await standaloneRequest({ source: '<p>opts in</p>' });
  return { request, fetched, base: entry.rendererBase() };
}
afterEach(() => vi.unstubAllGlobals());

it('exports inside the TMT app with the runtime from /colab/, the base the renderer frame uses', async () => {
  const { request, fetched, base } = await exportIn(
    `${door}/colab/p/12345678?x=1`,
    `${door}/colab/p/`,
    null,
  );
  expect(base).toBe(`${door}/colab/`);
  expect(fetched).toEqual([
    `${door}/colab/assets/colab-runtime.js`,
    `${door}/colab/assets/colab-runtime.css`,
  ]);
  expect(request).toEqual({
    runtime: {
      script: `text of ${door}/colab/assets/colab-runtime.js`,
      style: `text of ${door}/colab/assets/colab-runtime.css`,
    },
  });
});

it('exports from the paired mount when the TMT app reads through one', async () => {
  const { request, fetched, base } = await exportIn(
    `${door}/colab/p/12345678`,
    `${door}/colab/p/`,
    new URL(mount),
  );
  expect(base).toBe(mount);
  expect(fetched).toEqual([`${mount}assets/colab-runtime.js`, `${mount}assets/colab-runtime.css`]);
  expect(request).toHaveProperty('runtime');
});

it('exports outside the TMT app from the document base', async () => {
  const { request, fetched, base } = await exportIn(`${door}/app/index.html`, `${door}/app/`);
  expect(base).toBe(`${door}/app/`);
  expect(fetched).toEqual([
    `${door}/app/assets/colab-runtime.js`,
    `${door}/app/assets/colab-runtime.css`,
  ]);
  expect(request).toHaveProperty('runtime');
});

it('reports the runtime as unavailable when the TMT app cannot load it', async () => {
  // Another origin: the runtime a page already loaded is kept for the session, so a failure needs
  // a base nothing has loaded from.
  const down = 'https://down.test';
  const { request } = await exportIn(`${down}/colab/p/12345678`, `${down}/colab/p/`, null, false);
  expect(request).toEqual({ reason: 'runtime-unavailable' });
});

it('hands the renderer frame the same base the export uses', async () => {
  vi.resetModules();
  vi.stubGlobal('location', {
    href: `${door}/colab/p/12345678`,
    origin: door,
    pathname: '/colab/p/12345678',
  });
  vi.stubGlobal('document', { baseURI: `${door}/colab/p/`, querySelector: () => null });
  const entry = await import('../src/entry.js');
  entry.hostInShell(null);
  const { rendererUrl } = await import('../src/renderer.js');
  expect(rendererUrl(entry.rendererBase())).toBe(`${door}/colab/renderer.html`);
});
