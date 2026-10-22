import { afterEach, expect, it, vi } from 'vite-plus/test';
import { fixture, input } from './export-input.js';

// An opted-in page's export carries its standalone copy with the runtime from the same base the
// renderer frame uses, wherever the app runs (#1693). Only the opt-in parse needs a browser
// (DOMParser), so the marker check is replaced; the base, the fetches, the request and the bundle
// are the real modules.
vi.mock('../src/page-runtime.js', async (original) => ({
  ...(await original<typeof import('../src/page-runtime.js')>()),
  wantsRuntime: () => true,
}));

const door = 'https://door.test';
const mount = `${door}/r/abcdefghijklmnop/x/colab/`;
const source: string = fixture.standalone.source;
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
  const { prepareExport } = await import('../src/export.js');
  const request = await standaloneRequest({ source });
  const bundle = await prepareExport({
    ...input(),
    source,
    ...(request === undefined ? {} : { standalone: request }),
  });
  const copy = bundle.files.some((file) => file.name === 'page.standalone.html')
    ? await bundle.blob('page.standalone.html').text()
    : undefined;
  return { request, fetched, base: entry.rendererBase(), bundle, copy };
}
afterEach(() => vi.unstubAllGlobals());

it('exports inside the TMT app with the runtime from /colab/, the base the renderer frame uses', async () => {
  const { fetched, base, bundle, copy } = await exportIn(
    `${door}/colab/p/12345678?x=1`,
    `${door}/colab/p/`,
    null,
  );
  expect(base).toBe(`${door}/colab/`);
  expect(fetched).toEqual([
    `${door}/colab/assets/colab-runtime.js`,
    `${door}/colab/assets/colab-runtime.css`,
  ]);
  expect(bundle.files.slice(0, 2).map(({ name }) => name)).toEqual([
    'page.html',
    'page.standalone.html',
  ]);
  expect(bundle.standalone).toEqual({ included: true });
  expect(copy).toContain(`<script>text of ${door}/colab/assets/colab-runtime.js</script>`);
  expect(copy).toContain(
    `<style type="text/tailwindcss">text of ${door}/colab/assets/colab-runtime.css</style>`,
  );
  expect(copy).toContain(`<body>${source}</body>`);
});

it('exports from the paired mount when the TMT app reads through one', async () => {
  const { fetched, base, bundle, copy } = await exportIn(
    `${door}/colab/p/12345678`,
    `${door}/colab/p/`,
    new URL(mount),
  );
  expect(base).toBe(mount);
  expect(fetched).toEqual([`${mount}assets/colab-runtime.js`, `${mount}assets/colab-runtime.css`]);
  expect(bundle.standalone).toEqual({ included: true });
  expect(copy).toContain(`<script>text of ${mount}assets/colab-runtime.js</script>`);
});

it('exports outside the TMT app from the document base', async () => {
  const { fetched, base, bundle } = await exportIn(`${door}/app/index.html`, `${door}/app/`);
  expect(base).toBe(`${door}/app/`);
  expect(fetched).toEqual([
    `${door}/app/assets/colab-runtime.js`,
    `${door}/app/assets/colab-runtime.css`,
  ]);
  expect(bundle.standalone).toEqual({ included: true });
});

it('still exports the page, and says why it has no copy, when the TMT app cannot load the runtime', async () => {
  // Another origin: the runtime a page already loaded is kept for the session, so a failure needs
  // a base nothing has loaded from.
  const down = 'https://down.test';
  const { request, bundle, copy } = await exportIn(
    `${down}/colab/p/12345678`,
    `${down}/colab/p/`,
    null,
    false,
  );
  expect(request).toEqual({ reason: 'runtime-unavailable' });
  expect(bundle.standalone).toEqual({ included: false, reason: 'runtime-unavailable' });
  expect(copy).toBeUndefined();
  expect(bundle.files.map(({ name }) => name)).toContain('page.html');
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
