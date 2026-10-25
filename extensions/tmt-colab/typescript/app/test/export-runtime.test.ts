import { afterEach, expect, it, vi } from 'vite-plus/test';
import { fixture, input } from './export-input.js';

// An opted-in page's export carries its standalone copy with the runtime from the same base the
// renderer frame uses, wherever the app runs (#1693); a page that uses the design system carries the
// design system's two files from the same base (#5057). Only the opt-in parse needs a browser
// (DOMParser), so the marker check is replaced by a text test of the marker name; the base, the
// fetches, the request and the bundle are the real modules.
vi.mock('../src/page-runtime.js', async (original) => ({
  ...(await original<typeof import('../src/page-runtime.js')>()),
  wantsRuntime: (text: string) => text.includes('tmt-colab-runtime'),
}));

const door = 'https://door.test';
const mount = `${door}/r/abcdefghijklmnop/x/colab/`;
const source: string = fixture.standalone.source;
/** A fresh copy of the modules (the shell host and the runtime cache are module state) in a page
 * at `href` whose document base is `baseURI`; `shell` is what the TMT app host would register. */
async function exportIn(
  href: string,
  baseURI: string,
  shell?: URL | null,
  ok: boolean | ((target: string) => boolean) = true,
  pageSource: string = source,
) {
  vi.resetModules();
  const url = new URL(href);
  const fetched: string[] = [];
  vi.stubGlobal('location', { href, origin: url.origin, pathname: url.pathname });
  vi.stubGlobal('document', { baseURI, querySelector: () => null });
  vi.stubGlobal('fetch', async (target: string) => {
    fetched.push(target);
    const up = typeof ok === 'function' ? ok(target) : ok;
    return new Response(up ? `text of ${target}` : 'gone', { status: up ? 200 : 404 });
  });
  const entry = await import('../src/entry.js');
  if (shell !== undefined) entry.hostInShell(shell);
  const { standaloneRequest } = await import('../src/export-runtime.js');
  const { prepareExport } = await import('../src/export.js');
  const request = await standaloneRequest({ source: pageSource });
  const bundle = await prepareExport({
    ...input(),
    source: pageSource,
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

const design: string = fixture.standaloneDesign.source;
it('exports a design-system page with the design files from the base the renderer frame uses, and no runtime', async () => {
  // Its own origin: what a page loaded is kept for the session, so each case needs a base nothing has loaded from.
  const own = 'https://design.test';
  const { request, fetched, bundle, copy } = await exportIn(
    `${own}/colab/p/12345678?x=1`,
    `${own}/colab/p/`,
    null,
    true,
    design,
  );
  expect(fetched).toEqual([
    `${own}/colab/assets/colab-page-ui.js`,
    `${own}/colab/assets/colab-page-ui.css`,
  ]);
  expect(request).toEqual({
    design: {
      script: `text of ${own}/colab/assets/colab-page-ui.js`,
      style: `text of ${own}/colab/assets/colab-page-ui.css`,
    },
  });
  expect(bundle.standalone).toEqual({ included: true });
  expect(copy).toContain(`<style>text of ${own}/colab/assets/colab-page-ui.css</style>`);
  expect(copy).toContain(`<script>text of ${own}/colab/assets/colab-page-ui.js</script>`);
  expect(copy).toContain('<html data-colab-design>');
  expect(copy).not.toContain('colab-runtime');
});

it('loads the runtime and the design files for a page that asks for both, and for neither when it asks for neither', async () => {
  const own = 'https://both.test';
  const both = `${source}${design}`;
  const first = await exportIn(`${own}/colab/p/12345678`, `${own}/colab/p/`, null, true, both);
  expect([...first.fetched].sort()).toEqual(
    [
      `${own}/colab/assets/colab-page-ui.css`,
      `${own}/colab/assets/colab-page-ui.js`,
      `${own}/colab/assets/colab-runtime.css`,
      `${own}/colab/assets/colab-runtime.js`,
    ].sort(),
  );
  expect(first.bundle.standalone).toEqual({ included: true });
  const plain = await exportIn(
    `${own}/colab/p/12345678`,
    `${own}/colab/p/`,
    null,
    true,
    '<p>plain</p>',
  );
  expect(plain.request).toBeUndefined();
  expect(plain.fetched).toEqual([]);
  expect(plain.copy).toBeUndefined();
});

it('says the design system could not be loaded, whatever else loaded', async () => {
  const down = 'https://down-design.test';
  const { request, bundle, copy } = await exportIn(
    `${down}/colab/p/12345678`,
    `${down}/colab/p/`,
    null,
    (target) => !target.includes('colab-page-ui'),
    design,
  );
  expect(request).toEqual({ reason: 'design-unavailable' });
  expect(bundle.standalone).toEqual({ included: false, reason: 'design-unavailable' });
  expect(copy).toBeUndefined();
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
