/** The built Settings harness and the page route every Settings spec opens it through. */
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { build } from 'vite';
import type { Page } from '@playwright/test';

const assets = new URL('../../../rust/tmt-remote/assets/', import.meta.url);

export interface SettingsHost {
  directory: string;
  harness: string;
  html: string;
  css: string;
}

/** Builds one harness entry with the real Settings page and reads the page shell and its styles. */
export async function buildSettingsHost(entry: string, prefix: string): Promise<SettingsHost> {
  const directory = await mkdtemp(join(tmpdir(), prefix));
  await build({
    root: new URL('..', import.meta.url).pathname,
    logLevel: 'silent',
    build: {
      outDir: directory,
      emptyOutDir: true,
      minify: false,
      lib: {
        entry: new URL(entry, import.meta.url).pathname,
        formats: ['es'],
        fileName: () => 'harness.js',
      },
      rollupOptions: { output: { inlineDynamicImports: true } },
    },
  });
  // The app root normally supplies the common bar. This direct-page harness mounts the shared
  // shell itself, so give it the same empty slot without reintroducing a page title.
  const html = (await readFile(new URL('tmt-settings.html', assets), 'utf8'))
    .replace(
      '<div class="tmt-ui-shell-content">',
      '<div class="tmt-ui-shell-content"><div class="tmt-ui-shell-bar"></div>',
    )
    .replace(
      '<script type="module" src="/sdk/shell.js"></script>',
      '<script type="module" src="/harness.js"></script>',
    );
  const css =
    (await readFile(
      new URL('../../../../../design/browser-ui/generated/static.css', import.meta.url),
      'utf8',
    )) +
    (await readFile(new URL('pages.css', assets), 'utf8')) +
    (await readFile(new URL('tmt-settings.css', assets), 'utf8'));
  return { directory, harness: await readFile(join(directory, 'harness.js'), 'utf8'), html, css };
}

export async function removeSettingsHost(host: SettingsHost | undefined) {
  if (host?.directory) await rm(host.directory, { recursive: true, force: true });
}

/** Opens one Settings section of the harness at a width and theme. */
export async function openSettings(
  page: Page,
  host: SettingsHost,
  origin: string,
  options: { section: string; state: string; width?: number; theme?: string },
) {
  const { section, state, width = 1440, theme = 'light' } = options;
  await page.setViewportSize({ width, height: 1050 });
  await page.addInitScript((theme) => localStorage.setItem('tmt-ui-theme', theme), theme);
  await page.route(`${origin}/**`, (route) => {
    const path = new URL(route.request().url()).pathname;
    return route.fulfill({
      contentType:
        path === '/harness.js'
          ? 'text/javascript'
          : path === '/sdk/pages.css'
            ? 'text/css'
            : 'text/html',
      body:
        path === '/harness.js' ? host.harness : path === '/sdk/pages.css' ? host.css : host.html,
    });
  });
  await page.goto(`${origin}/settings/?state=${state}#${section}`);
}
