import { expect, test, chromium, type Browser, type Page, type Request } from '@playwright/test';
import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { AGENT_STATES } from '../src/ops-source.js';
import { mockDocument } from '../src/digest-mock.js';
import type { UsageRange } from '../src/digest-source.js';

/**
 * The one TMT web app as a host (#3937): conditions the door and the shell must keep, against a
 * real `tmt remote serve` and Chromium. Run it with the binary built by
 * `cargo build -p tmt-remote`, or point TMT_REMOTE_BINARY at one.
 *
 * Tests whose reason is a door or shell gap name it in a TODO and start expected-to-fail.
 */
const BINARY =
  process.env.TMT_REMOTE_BINARY ??
  fileURLToPath(new URL('../../../../../rust/target/debug/tmt-remote', import.meta.url));

let root: string;
let serve: ChildProcessWithoutNullStreams;
let origin: string;
let address: string;
let browser: Browser;

test.beforeAll(async () => {
  // Short root: Unix socket paths are limited to about 100 bytes. The door runs in this private
  // HOME with the fixture core; it never touches the owner's TMT data or tmux.
  root = await mkdtemp('/tmp/tmt-e2e-');
  await writeFile(
    join(root, 'core.mjs'),
    await readFile(new URL('./core-fixture.mjs', import.meta.url)),
    { mode: 0o700 },
  );
  serve = spawn(BINARY, ['serve', '--json'], {
    env: {
      PATH: process.env.PATH,
      HOME: root,
      TMT_AGENT: '1',
      BROWSER: 'true',
      TMT_EXECUTABLE: join(root, 'core.mjs'),
      TMT_FIXTURE_ROOT: root,
    },
  });
  const line = await new Promise<string>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('No descriptor within 30 s.')), 30000);
    createInterface({ input: serve.stdout }).once('line', (value) => {
      clearTimeout(timer);
      resolve(value);
    });
  });
  address = (JSON.parse(line) as { address: string }).address;
  origin = new URL(address).origin;
  browser = await chromium.launch();
});
test.afterAll(async () => {
  const failures: unknown[] = [];
  const step = async (release: () => unknown) => {
    try {
      await release();
    } catch (error) {
      failures.push(error);
    }
  };
  await step(() => browser?.close());
  await step(async () => {
    if (!serve || serve.exitCode !== null || serve.signalCode !== null) return;
    const exited = new Promise((resolve) => serve.once('exit', resolve));
    serve.kill('SIGTERM');
    await exited;
  });
  await step(() => root && rm(root, { recursive: true, force: true }));
  if (failures.length) throw new AggregateError(failures, 'SPA host teardown failed');
});

const get = (path: string, init: RequestInit = {}) =>
  fetch(`${origin}${path}`, { redirect: 'manual', ...init });

/** Routes the door answers with the one shell document. */
const APP_ROUTES = [
  '/',
  '/settings',
  '/settings/',
  '/remote',
  '/remote/',
  '/remote/devices',
  '/remote/connect',
  '/remote/machine',
  '/remote/settings',
  '/ops',
  '/ops/',
  '/digest',
  '/digest/',
  '/colab',
  '/colab/',
  '/colab/p/abc123',
  '/colab/a/b',
];
/** Near misses and neighbours the shell must not claim. */
const NOT_APP_ROUTES = [
  '/nope',
  '/remote/other',
  '/ops/x',
  '/digest/x',
  '/settings/x',
  '/colab/a.b',
  `/colab/${'a'.repeat(161)}`,
  '/read',
  '/read/x',
  '/x',
  '/x/colab/',
  '/pair/x',
];

test.describe('the door', () => {
  test('answers GET and HEAD of the explicit route list with the one shell', async () => {
    const shell = await (await get('/')).text();
    expect(shell).toContain('/sdk/shell.js');
    for (const path of APP_ROUTES) {
      const full = await get(path);
      expect(full.status, `GET ${path}`).toBe(200);
      expect(full.headers.get('content-type'), `GET ${path}`).toBe('text/html; charset=utf-8');
      expect(await full.text(), `GET ${path} is the same document`).toBe(shell);
      const head = await get(path, { method: 'HEAD' });
      expect(head.status, `HEAD ${path}`).toBe(200);
      for (const name of ['content-type', 'content-security-policy', 'cache-control']) {
        expect(head.headers.get(name), `HEAD ${path} ${name}`).toBe(full.headers.get(name));
      }
      expect(await head.text(), `HEAD ${path} has no body`).toBe('');
    }
    // A cross-origin load is refused.
    expect((await get('/', { headers: { Origin: 'http://evil.example' } })).status).toBe(403);
  });

  test('answers 404 for every path off the list and for any other method', async () => {
    for (const path of NOT_APP_ROUTES)
      for (const method of ['GET', 'HEAD'])
        expect((await get(path, { method })).status, `${method} ${path}`).toBe(404);
    for (const path of ['/', '/colab/p/abc123', '/settings', '/remote/settings'])
      for (const method of ['POST', 'PUT', 'DELETE', 'PATCH'])
        expect((await get(path, { method, body: '{}' })).status, `${method} ${path}`).toBe(404);
  });

  test('keeps /sdk, /x, /p, /read and /pair as they were', async () => {
    const shell = await (await get('/')).text();
    // The SDK and its assets are served by name, not by the fallback.
    for (const [path, type] of [
      ['/sdk/remote-v1.js', 'text/javascript; charset=utf-8'],
      ['/sdk/shell.js', 'text/javascript; charset=utf-8'],
      ['/sdk/pages.css', 'text/css; charset=utf-8'],
    ] as const) {
      const response = await get(path);
      expect(response.status, path).toBe(200);
      expect(response.headers.get('content-type'), path).toBe(type);
      expect(await response.text(), path).not.toBe(shell);
    }
    expect((await get('/sdk/nope')).status).toBe(404);
    // The legacy pairing link and the owner's short page link redirect, for GET and HEAD.
    for (const method of ['GET', 'HEAD']) {
      const pair = await get('/pair', { method });
      expect([pair.status, pair.headers.get('location')], `${method} /pair`).toEqual([
        308,
        '/remote/pair',
      ]);
      const short = await get('/p/abcd1234', { method });
      expect([short.status, short.headers.get('location')], `${method} /p`).toEqual([
        308,
        '/colab/p/abcd1234',
      ]);
    }
    expect((await get('/pair', { headers: { 'Sec-Fetch-Site': 'cross-site' } })).status).toBe(403);
    // The pairing page keeps its own document and its own script hash.
    const pairing = await get('/remote/pair');
    expect(pairing.status).toBe(200);
    expect(await pairing.text()).not.toBe(shell);
    expect(pairing.headers.get('content-security-policy')).toMatch(/script-src 'self' 'sha256-/);
    // /read and an unprefixed /x are not the app's; module mounts live under the machine prefix.
    for (const path of ['/read', '/read/x', '/x', '/x/colab/', '/x/-/shell'])
      expect((await get(path)).status, path).toBe(404);
  });

  test('keeps the strict request-target grammar in front of the fallback', async () => {
    // An escape, an empty inner segment or a query beyond plain name=value text is a malformed
    // target (400), never the shell; the fallback stays an explicit list.
    for (const path of [
      '/settings?section=%2F',
      '/colab/pages/abcd?read=a/b',
      '/colab/pages/abcd?tmt-session=x',
      '/colab/a%2Fb',
      '/colab//x',
    ])
      for (const method of ['GET', 'HEAD'])
        expect((await get(path, { method })).status, `${method} ${path}`).toBe(400);
    // The app writes its own plain keys onto its routes (`?open=`, `?read=`, `?anchor=`), so a
    // reload of such an address is the shell.
    for (const path of ['/colab/pages/abcd?read=x', '/colab/p/abcd?open=1&a=b'])
      expect((await get(path)).status, path).toBe(200);
  });

  test('redirects a short page link that carries a plain query', async () => {
    for (const method of ['GET', 'HEAD']) {
      const short = await get('/p/abcd1234?open=1&a=b', { method });
      expect([short.status, short.headers.get('location')], method).toEqual([
        308,
        '/colab/p/abcd1234?open=1&a=b',
      ]);
    }
    // Only a plain name=value query rides on the short link.
    for (const path of ['/p/abcd1234?a=%2e', '/p/abcd1234?a=/../x', '/p/abcd1234?a=b;c'])
      expect((await get(path)).status, path).toBe(400);
  });

  test('serves shell.html public and static', async () => {
    const plain = await get('/');
    const body = await plain.text();
    // The same bytes with or without a cookie or a device header, on every route and request.
    for (const init of <RequestInit[]>[
      {},
      { headers: { Cookie: 'tmt-session=a; tmt-device=b' } },
      { headers: { 'tmt-device-context': 'x' } },
    ]) {
      const again = await get('/colab/p/xyz', init);
      expect(await again.text()).toBe(body);
      expect(again.headers.get('set-cookie')).toBeNull();
      expect(again.headers.get('cache-control')).toBe('no-store');
    }
    expect(plain.headers.get('set-cookie')).toBeNull();
    // Nothing per machine or per request is embedded: no route prefix, no inline script.
    expect(body).not.toContain(new URL(address).pathname);
    expect(body).not.toMatch(/<script(?![^>]*\bsrc=)/);
    expect(body).not.toMatch(/\son\w+\s*=/);
    expect([...body.matchAll(/<script[^>]*\bsrc="([^"]+)"/g)].map((match) => match[1])).toEqual([
      '/sdk/shell.js',
    ]);
  });

  test('gives the shell a strict CSP', async () => {
    const policy = (await get('/')).headers.get('content-security-policy')!;
    const directives = parsePolicy(policy);
    expect(directives.get('default-src')).toEqual(["'none'"]);
    expect(directives.get('script-src')).toEqual(["'self'"]);
    for (const name of ['base-uri', 'form-action', 'object-src', 'frame-ancestors'])
      expect(directives.get(name), name).toEqual(["'none'"]);
    expect(policy).not.toMatch(/unsafe-|\*|https?:/);
  });

  test('pins the shell CSP: the page policy plus only what the Colab route needs', async () => {
    // Agreed with tmt-lead and remote-lead (#3937): the pairing page's policy plus exactly
    // img-src 'self', worker-src 'self', frame-src 'self' and object-src 'none'. Any widening fails.
    const page =
      "default-src 'none'; script-src 'self'; style-src 'self'; img-src data:; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";
    const shell =
      "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; worker-src 'self'; frame-src 'self'; base-uri 'none'; form-action 'none'; object-src 'none'; frame-ancestors 'none'";
    for (const path of ['/', '/remote/', '/settings', '/colab/', '/colab/pages/x'])
      expect((await get(path)).headers.get('content-security-policy'), path).toBe(shell);
    // Pairing keeps its own document and the page policy, with its bootstrap's hash only.
    const pairing = (await get('/remote/pair')).headers.get('content-security-policy')!;
    expect(pairing.replace(/ 'sha256-[^']+'/, '')).toBe(page);
    // No wider than Colab's app document policy (tmt-colab browser_policy.rs POLICY).
    const source = await readFile(
      fileURLToPath(
        new URL('../../../../tmt-colab/rust/tmt-colab/src/browser_policy.rs', import.meta.url),
      ),
      'utf8',
    );
    const colab = parsePolicy(/pub const POLICY: &str = "([^"]+)"/.exec(source)![1]!);
    for (const [name, sources] of parsePolicy(shell)) {
      const allowed = new Set(colab.get(name) ?? colab.get('default-src') ?? ["'none'"]);
      for (const value of sources) expect(allowed, `${name} ${value}`).toContain(value);
    }
  });
});

/** `name -> sources`; a hash source stands for any per-page script hash. */
function parsePolicy(policy: string): Map<string, string[]> {
  return new Map(
    policy
      .split(';')
      .map((part) => part.trim().split(/\s+/))
      .filter((parts) => parts[0])
      .map(([name, ...sources]) => [
        name!,
        sources.map((source) => (source.startsWith("'sha256-") ? "'sha256-*'" : source)),
      ]),
  );
}

/** Open an app route in a fresh page and watch its requests. */
async function openApp(path: string, prepare?: (page: Page) => Promise<void>) {
  const page = await browser.newPage();
  const requests: Request[] = [];
  page.on('request', (request) => requests.push(request));
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await prepare?.(page);
  await page.goto(`${origin}${path}`, { waitUntil: 'load' });
  await page.locator('.tmt-ui-shell-main[data-route]').waitFor({ timeout: 10000 });
  expect(errors).toEqual([]);
  return { page, requests, url: (request: Request) => new URL(request.url()) };
}

/** The module's pages: nested in the rail on a wide screen, a tab strip on a phone. */
const modulePages = (page: Page) =>
  page.locator('.tmt-ui-rail-sub:visible, nav.tmt-ui-module-menu:visible');

/** Requests that are not the document or the door's own static SDK files. */
const beyondAssets = (requests: Request[]) =>
  requests
    .filter((request) => !request.isNavigationRequest())
    .map((request) => `${request.method()} ${new URL(request.url()).pathname}`)
    .filter((entry) => !/^GET \/sdk\/(pages\.css|shell\.js|remote-v1\.js)$/.test(entry));

test.describe('the app in a browser', () => {
  test('makes no session reads from an unpaired /remote', async () => {
    const { page, requests } = await openApp('/remote');
    await expect(page.locator('.tmt-ui-shell-main[data-route="remote"]')).toContainText(
      'Not paired',
    );
    await page.waitForTimeout(1500);
    // The landing reads nothing from the door until its own check connects.
    expect(beyondAssets(requests)).toEqual([]);
    await page.close();
  });

  test('loads no Colab chunk on the Colab route unpaired', async () => {
    for (const path of ['/colab', '/colab/p/abc123']) {
      const { page, requests } = await openApp(path);
      await expect(page.locator('.tmt-ui-shell-main[data-route="colab"]')).toContainText(
        'Pair this browser with Remote to open Colab',
      );
      await page.waitForTimeout(500);
      // Only the page's own mount and the composed-shell read may be asked; no module under a
      // mount, and no Colab asset, is fetched.
      for (const entry of beyondAssets(requests))
        expect(entry, path).toMatch(/^(POST \/sdk\/mount|GET \/r\/[a-z0-9]+\/x\/-\/shell)$/);
      expect(requests.map((request) => request.url()).join('\n')).not.toMatch(
        /\/x\/colab\/|\/colab\/assets\//,
      );
      await page.close();
    }
  });

  test('every focusable on every route shows one inset frame (#3998 tab walk)', async () => {
    const { page } = await openApp('/');
    await page.setViewportSize({ width: 1440, height: 900 });
    const problems: string[] = [];
    for (const scheme of ['light', 'dark'] as const) {
      await page.emulateMedia({ colorScheme: scheme });
      for (const path of ['/', '/remote/', '/remote/settings', '/settings', '/ops', '/digest']) {
        await page.goto(`${origin}${path}`);
        // The route's own markup is in place and no first read is still painting.
        await expect(page.locator('.tmt-ui-shell-main > :not([hidden])').first()).toBeVisible();
        await expect(page.locator('.tmt-page-loading')).toHaveCount(0);
        await page.locator('body').focus();
        const seen = new Set<string>();
        let first = '';
        for (let step = 0; step < 60; step++) {
          await page.keyboard.press('Tab');
          const focus = await page.evaluate(() => {
            const element = document.activeElement as HTMLElement | null;
            if (!element || element === document.body) return undefined;
            const style = getComputedStyle(element);
            const width = Number.parseFloat(style.outlineWidth);
            const outline = style.outlineStyle !== 'none' && width > 0;
            const wrong: string[] = [];
            if (style.outlineStyle === 'auto') wrong.push('a browser default ring');
            if (outline && Number.parseFloat(style.outlineOffset) > -width)
              wrong.push(`an outline outside its border (offset ${style.outlineOffset})`);
            if (
              style.boxShadow !== 'none' &&
              style.boxShadow.split(/,(?![^(]*\))/).some((shadow) => !/inset/.test(shadow))
            )
              wrong.push('an outer glow');
            const name =
              element.getAttribute('aria-label') ?? element.textContent?.trim().slice(0, 24) ?? '';
            const key = `${element.tagName.toLowerCase()}.${element.className} ${name}`;
            return { key, wrong };
          });
          if (!focus) continue;
          // One lap: stop when focus comes back to the first control.
          if (focus.key === first) break;
          first ||= focus.key;
          if (seen.has(focus.key)) continue;
          seen.add(focus.key);
          if (focus.wrong.length)
            problems.push(`${scheme} ${path} ${focus.key}: ${focus.wrong.join(', ')}`);
        }
        expect(seen.size, `${scheme} ${path} reached focusables`).toBeGreaterThan(3);
      }
    }
    expect(problems).toEqual([]);
    await page.close();
  });

  /** Remote's rows: the path, the menu row, the page heading, a heading that shows and one that
   * does not (the views share one document, so a hidden one is in the page but not shown). */
  const REMOTE_VIEWS = [
    {
      path: '/remote/',
      id: 'status',
      h1: 'Remote connection',
      shown: '#heading',
      hidden: '#connect-heading',
    },
    {
      path: '/remote/devices',
      id: 'devices',
      h1: 'Devices',
      shown: '#devices-title',
      hidden: '#opening-title',
    },
    {
      path: '/remote/connect',
      id: 'connect',
      h1: 'Connect a device',
      shown: '#connect-heading',
      hidden: '#heading',
    },
    {
      path: '/remote/machine',
      id: 'machine',
      h1: 'This machine’s settings',
      shown: '#opening-title',
      hidden: '#devices-title',
    },
    {
      path: '/remote/settings',
      id: 'settings',
      h1: 'Remote settings',
      shown: '#firestore-title',
      hidden: '#opening-title',
    },
  ] as const;
  const litRow = (page: Page) =>
    modulePages(page)
      .locator('[aria-current="page"]')
      .evaluateAll((items) => items.map((item) => item.getAttribute('data-id')));
  const expectView = async (page: Page, view: (typeof REMOTE_VIEWS)[number]) => {
    expect(new URL(page.url()).pathname, view.id).toBe(view.path);
    expect(new URL(page.url()).hash, view.id).toBe('');
    await expect.poll(() => litRow(page), view.id).toEqual([view.id]);
    await expect(page.locator('.tmt-ui-shell-bar h1'), view.id).toHaveText(view.h1);
    await expect(page.locator(view.shown), view.id).toBeVisible();
    await expect(page.locator(view.hidden), view.id).toBeHidden();
    await expect(page, view.id).toHaveTitle(`${view.h1} · TMT`);
  };

  test('each Remote row is its own route and selection follows the URL', async () => {
    for (const view of REMOTE_VIEWS) {
      // A direct load, and a reload, land on the row the path names.
      const { page } = await openApp(view.path);
      await expectView(page, view);
      await page.reload({ waitUntil: 'load' });
      await expectView(page, view);
      await page.close();
    }
  });

  test('switching between Remote rows only swaps the content: no remount, no storage', async () => {
    const { page } = await openApp('/remote/');
    // Marks on what must survive a switch: the document, the shell's header slot and content slot,
    // the rail and the module menu element. Mutations of the slots' own children are counted.
    await page.evaluate(() => {
      const w = window as unknown as { __kept: unknown; __swaps: number };
      const bar = document.querySelector('.tmt-ui-shell-bar')!;
      const main = document.querySelector('.tmt-ui-shell-main')!;
      w.__kept = {
        bar: bar.firstElementChild,
        main: [...main.children],
        rail: document.querySelector('nav.tmt-ui-rail'),
        mark: 1,
      };
      w.__swaps = 0;
      const watch = new MutationObserver((records) => {
        for (const record of records) if (record.type === 'childList') w.__swaps++;
      });
      watch.observe(bar, { childList: true });
      watch.observe(main, { childList: true });
    });
    const menu = modulePages(page);
    for (const view of REMOTE_VIEWS.slice(1).concat(REMOTE_VIEWS[0])) {
      await menu.locator(`a[data-id="${view.id}"]`).click();
      await expectView(page, view);
    }
    const kept = await page.evaluate(() => {
      const w = window as unknown as {
        __kept: { bar: Element; main: Element[]; rail: Element; mark: number };
        __swaps: number;
      };
      const bar = document.querySelector('.tmt-ui-shell-bar')!;
      const main = document.querySelector('.tmt-ui-shell-main')!;
      return {
        sameDocument: w.__kept.mark === 1,
        bar: bar.firstElementChild === w.__kept.bar,
        main:
          main.children.length === w.__kept.main.length &&
          [...main.children].every((child, index) => child === w.__kept.main[index]),
        rail: document.querySelector('nav.tmt-ui-rail') === w.__kept.rail,
        swaps: w.__swaps,
        stored: Object.keys(sessionStorage).filter((key) => key.startsWith('tmt.remote.menu')),
      };
    });
    expect(kept).toEqual({
      sameDocument: true,
      bar: true,
      main: true,
      rail: true,
      swaps: 0,
      stored: [],
    });
    await page.close();
  });

  test('Back and forward land on the same row, selected', async () => {
    const { page } = await openApp('/remote/');
    const menu = modulePages(page);
    const order = [REMOTE_VIEWS[1]!, REMOTE_VIEWS[3]!, REMOTE_VIEWS[4]!, REMOTE_VIEWS[2]!];
    for (const view of order) {
      await menu.locator(`a[data-id="${view.id}"]`).click();
      await expectView(page, view);
    }
    for (const view of [...order.slice(0, -1).reverse(), REMOTE_VIEWS[0]!]) {
      await page.goBack();
      await expectView(page, view);
    }
    for (const view of order) {
      await page.goForward();
      await expectView(page, view);
    }
    await page.close();
  });

  test('captures every Remote row at 1440 and 390, light and dark', async () => {
    // Evidence for review, written only when a directory is named (TMT_REMOTE_CAPTURE_DIR).
    const directory = process.env.TMT_REMOTE_CAPTURE_DIR;
    test.skip(!directory, 'TMT_REMOTE_CAPTURE_DIR is not set');
    await mkdir(directory!, { recursive: true });
    for (const colorScheme of ['light', 'dark'] as const)
      for (const width of [1440, 390]) {
        const context = await browser.newContext({ colorScheme, viewport: { width, height: 900 } });
        const page = await context.newPage();
        await page.goto(`${origin}/remote/`, { waitUntil: 'load' });
        await page.locator('.tmt-ui-shell-main[data-route="remote"]').waitFor();
        for (const view of REMOTE_VIEWS) {
          // Narrow screens keep the menu behind its button; the path is what is being shown.
          await page.evaluate((path) => window.history.pushState(null, '', path), view.path);
          await page.evaluate(() => window.dispatchEvent(new PopStateEvent('popstate')));
          await expectView(page, view);
          await page.screenshot({
            path: join(directory!, `remote-${view.id}-${width}-${colorScheme}.png`),
          });
        }
        await context.close();
      }
  });

  test('the rail returns to Status and a menu click on the shown row stays put', async () => {
    const { page } = await openApp('/remote/machine');
    await page.locator('nav.tmt-ui-rail a[data-id="ops"]').click();
    await expect(page.locator('.tmt-ui-shell-main')).toHaveAttribute('data-route', 'ops');
    await page.locator('nav.tmt-ui-rail a[data-id="remote"]').click();
    await expectView(page, REMOTE_VIEWS[0]!);
    await modulePages(page).locator('a[data-id="status"]').click();
    await expectView(page, REMOTE_VIEWS[0]!);
    // The menu has no Remotes heading; This machine's group keeps its own.
    await expect(modulePages(page).locator('.tmt-ui-rail-sub-group')).not.toContainText([
      'Remotes',
    ]);
    await expect(modulePages(page)).toContainText('This machine');
    await expect(modulePages(page).locator('a[data-id="status"]')).toContainText('Status');
    await page.close();
  });

  test("loads a page's own stylesheet with its route and drops it on leaving", async () => {
    const { page } = await openApp('/remote/settings');
    const sheet = page.locator('head link[rel="stylesheet"][href="/sdk/settings.css"]');
    await expect(sheet).toHaveCount(1);
    // Its rules apply before the markup shows: the page's choice control keeps its arrow room.
    await expect(page.locator('.tmt-ui-shell-main')).toHaveAttribute('data-route', 'remote');
    await page.locator('nav.tmt-ui-rail a[data-id="ops"]').click();
    await expect(page.locator('.tmt-ui-shell-main')).toHaveAttribute('data-route', 'ops');
    await expect(sheet).toHaveCount(0);
    await page.goBack();
    await expect(page.locator('.tmt-ui-shell-main')).toHaveAttribute('data-route', 'remote');
    await expect(sheet).toHaveCount(1);
    await page.close();
  });

  test('shows the identical rail on every route with one document load', async () => {
    const { page, requests } = await openApp('/');
    await page.evaluate(() => ((window as unknown as { __spa: number }).__spa = 1));
    const rail = () =>
      page.locator('nav.tmt-ui-rail a.tmt-ui-rail-item').evaluateAll((items) =>
        items.map((item) => ({
          id: item.getAttribute('data-id'),
          label: item.textContent?.trim(),
          href: item.getAttribute('href'),
          disabled: item.getAttribute('aria-disabled'),
          current: item.getAttribute('aria-current'),
        })),
      );
    const first = await rail();
    expect(first.map((item) => item.id)).toEqual([
      'home',
      'colab',
      'remote',
      'ops',
      'digest',
      'settings',
    ]);
    const stable = (items: typeof first) => items.map(({ current: _current, ...item }) => item);
    expect(first.filter((item) => item.current).map((item) => item.id)).toEqual(['home']);
    // Follow the rail through every route this app serves itself, then walk back.
    for (const id of ['remote', 'ops', 'digest', 'settings', 'home']) {
      await page.locator(`nav.tmt-ui-rail a[data-id="${id}"]`).click();
      const route = id === 'settings' ? 'settings' : id;
      await expect(page.locator('.tmt-ui-shell-main')).toHaveAttribute('data-route', route);
      const items = await rail();
      expect(stable(items), id).toEqual(stable(first));
      expect(
        items.filter((item) => item.current).map((item) => item.id),
        id,
      ).toEqual([id]);
    }
    await page.goBack();
    await expect(page.locator('.tmt-ui-shell-main')).toHaveAttribute('data-route', 'settings');
    expect(stable(await rail())).toEqual(stable(first));
    // One document for all of it: the marker survived, and only the first request navigated.
    expect(await page.evaluate(() => (window as unknown as { __spa: number }).__spa)).toBe(1);
    expect(requests.filter((request) => request.isNavigationRequest()).length).toBe(1);
    await page.close();
  });

  test('puts the TMT mark in the rail head and the page title alone in the bar; the rail toggle has its own row', async () => {
    const { page } = await openApp('/ops');
    await page.setViewportSize({ width: 1440, height: 900 });
    const rail = page.locator('nav.tmt-ui-rail');
    const brand = rail.locator('> .tmt-ui-shell-brand');
    const header = page.locator('.tmt-ui-shell-bar > .tmt-ui-header');
    // The brand sits once, at the head of the rail; the host's header (the page title) starts the
    // content column beside it.
    await expect(brand).toContainText('tmt');
    await expect(header).toContainText('Agents');
    const [railBox, brandBox, headerBox] = await Promise.all(
      [rail, brand, header].map(async (node) => (await node.boundingBox())!),
    );
    expect(Math.round(brandBox!.x)).toBeGreaterThanOrEqual(Math.round(railBox!.x));
    expect(Math.round(brandBox!.y + brandBox!.height)).toBeLessThanOrEqual(
      Math.round(railBox!.y + railBox!.height),
    );
    expect(Math.round(headerBox!.x)).toBeGreaterThanOrEqual(
      Math.round(railBox!.x + railBox!.width),
    );
    await expect(header.locator('.tmt-ui-title')).toHaveCSS('font-size', '16px');
    // The bar draws no module crumb and no second TMT: the title is the page's name.
    await expect(header.locator('.tmt-ui-wordmark, .tmt-ui-brand')).toHaveCount(0);
    await expect(header.locator('.tmt-ui-title')).toHaveText('Agents');
    await expect(page.locator('.tmt-ui-module-menu-title')).toHaveCount(0);
    // The toggle is the rail's foot row, ruled off above TMT settings.
    const toggle = page.locator('nav.tmt-ui-rail > .tmt-ui-rail-top > .tmt-ui-rail-toggle');
    await expect(toggle).toHaveAttribute('aria-label', 'Collapse sidebar');
    const home = page.locator('nav.tmt-ui-rail a[data-id="home"]');
    const settings = page.locator('nav.tmt-ui-rail').getByRole('link', {
      name: 'TMT settings',
      exact: true,
    });
    expect((await toggle.boundingBox())!.y).toBeGreaterThan((await home.boundingBox())!.y);
    expect((await toggle.boundingBox())!.y).toBeLessThan((await settings.boundingBox())!.y);
    await expect(page.locator('.tmt-ui-rail-top')).toHaveCSS('border-top-style', 'solid');
    // Expanded, a label needs no tooltip.
    await home.hover();
    await expect(page.locator('.tmt-ui-rail-tip')).toBeHidden();
    const icon = await home.locator('svg').boundingBox();
    // Collapsing slides the rail (a width transition) and keeps each icon where it was.
    await expect(page.locator('nav.tmt-ui-rail')).toHaveCSS('transition-property', 'width');
    await toggle.click();
    await expect(toggle).toHaveAttribute('aria-label', 'Expand sidebar');
    await expect(page.locator('nav.tmt-ui-rail')).toHaveCSS('width', '56px');
    // The brand keeps its mark and drops its words.
    await expect(brand.locator('.tmt-ui-shell-brand-text')).toBeHidden();
    await expect(brand.locator('svg').first()).toBeVisible();
    expect((await home.locator('svg').boundingBox())!.x).toBe(icon!.x);
    await expect(home.locator('.tmt-ui-rail-label')).toBeHidden();
    // Collapsed, hovering or focusing an item shows its label beside the rail.
    await home.hover();
    const tip = page.locator('.tmt-ui-rail-tip');
    await expect(tip).toBeVisible();
    await expect(tip).toHaveText('Home');
    const [tipBox, homeBox] = await Promise.all(
      [tip, home].map(async (node) => (await node.boundingBox())!),
    );
    expect(tipBox!.x).toBeGreaterThanOrEqual(homeBox!.x + homeBox!.width);
    await page.mouse.move(800, 600);
    await expect(tip).toBeHidden();
    await page.locator('nav.tmt-ui-rail a[data-id="digest"]').focus();
    await expect(tip).toHaveText('Digest');
    await toggle.click();
    await expect(page.locator('nav.tmt-ui-rail')).toHaveCSS('width', '220px');
    await page.close();
  });
});

/** A composed shell answer whose Colab module carries `entry`, as a paired door would send it. */
function composed(rail: string, entry?: string) {
  return {
    version: 1,
    modules: [
      {
        extension: 'colab',
        rail: { label: 'Colab', icon: 'file-text', path: rail, order: 1 },
        menu: [],
        settings: [],
        ...(entry === undefined ? {} : { entry }),
      },
    ],
  };
}
const PREFIX = '/r/abcdefghijklmnop';
const MOUNT = `${PREFIX}/x/colab/`;

/** Open /colab/ as a paired browser would, with the door's mount and shell answers faked. */
async function openColab(answer: unknown) {
  const stub = `export function mount(slot) { slot.main.textContent = 'colab stub mounted'; }`;
  return openApp('/colab/', async (page) => {
    await page.route('**/sdk/mount', (route) =>
      route.fulfill({ json: { address: `${origin}${PREFIX}` } }),
    );
    await page.route(`**${PREFIX}/x/-/shell`, (route) => route.fulfill({ json: answer }));
    // Anything a module entry may point at answers with the stub, so a wrongly accepted entry
    // shows up as a request and as a mounted stub.
    await page.route(/\.js$/, (route) => {
      const path = new URL(route.request().url()).pathname;
      if (path.startsWith('/sdk/')) return route.fallback();
      return route.fulfill({ body: stub, contentType: 'text/javascript' });
    });
  });
}
const loadedEntry = (requests: Request[]) =>
  requests.find(
    (request) =>
      !request.url().startsWith(`${origin}/sdk/`) &&
      /\.js(\?|$)/.test(new URL(request.url()).pathname),
  );

test.describe('a module entry', () => {
  for (const [name, rail, entry] of [
    ['the default under its mount', MOUNT, undefined],
    ['a named file under its mount', MOUNT, 'assets/other.js'],
  ] as const)
    test(`loads ${name}`, async () => {
      const { page, requests } = await openColab(composed(rail, entry));
      await expect(page.locator('.tmt-ui-shell-main')).toContainText('colab stub mounted');
      expect(new URL(loadedEntry(requests)!.url()).pathname.startsWith(MOUNT)).toBe(true);
      await page.close();
    });

  for (const [name, rail, entry] of [
    ['a cross-origin URL', MOUNT, 'https://evil.example/colab.js'],
    ['a protocol-relative URL', MOUNT, '//evil.example/colab.js'],
    ['a same-origin path outside the prefix', MOUNT, '/evil/colab.js'],
    ['a relative path that climbs out of its mount', MOUNT, '../../colab.js'],
    ['another module under the same prefix', MOUNT, `${PREFIX}/x/other/colab.js`],
    ['the same module under another machine prefix', MOUNT, '/r/zzzzzzzzzzzzzzzz/x/colab/colab.js'],
    ['an encoded slash', MOUNT, 'assets%2f..%2fcolab.js'],
    ['a dot segment', MOUNT, 'assets/../colab.js'],
  ] as const)
    test(`refuses ${name} and loads nothing`, async () => {
      const { page, requests } = await openColab(composed(rail, entry));
      await expect(page.locator('.tmt-ui-shell-main')).toContainText('Colab');
      await page.waitForTimeout(500);
      expect(requests.map((request) => request.url()).join('\n')).not.toMatch(
        /evil|colab\.js|\/zzzzzzzzzzzzzzzz\//,
      );
      await expect(page.locator('.tmt-ui-shell-main')).not.toContainText('colab stub mounted');
      await page.close();
    });

  test('shows the busy mark while the module loads, then the module in its place', async () => {
    let release!: () => void;
    const held = new Promise<void>((resolve) => (release = resolve));
    const { page } = await openApp('/colab/', async (page) => {
      await page.route('**/sdk/mount', (route) =>
        route.fulfill({ json: { address: `${origin}${PREFIX}` } }),
      );
      await page.route(`**${PREFIX}/x/-/shell`, (route) =>
        route.fulfill({ json: composed(MOUNT) }),
      );
      await page.route(/\.js$/, async (route) => {
        if (new URL(route.request().url()).pathname.startsWith('/sdk/')) return route.fallback();
        await held;
        return route.fulfill({
          // Like Colab: it clears the main area for an empty root, filled a moment later.
          body: `export function mount(slot) {
            const root = document.createElement('div');
            slot.main.replaceChildren(root);
            window.fillStub = () => (root.innerHTML = '<p>colab stub mounted</p>');
          }`,
          contentType: 'text/javascript',
        });
      });
    });
    const loading = page.locator('.tmt-ui-shell-main + .tmt-ui-shell-loading');
    await expect(loading).toBeVisible();
    await expect(loading).toHaveAttribute('role', 'status');
    await expect(loading.locator('.tmt-ui-busy-mark svg')).toBeVisible();
    release();
    await page.waitForFunction(() => 'fillStub' in window);
    // Mounted but still empty: the mark stays until the module paints.
    await expect(loading).toBeVisible();
    await page.evaluate(() => (window as unknown as { fillStub: () => void }).fillStub());
    await expect(page.locator('.tmt-ui-shell-main')).toContainText('colab stub mounted');
    await expect(loading).toBeHidden();
    await page.close();
  });

  test('refuses a sibling that only shares the mount name as a prefix', async () => {
    const { page, requests } = await openColab(
      composed(`${PREFIX}/x/colab`, `${PREFIX}/x/colab-evil/colab.js`),
    );
    await page.waitForTimeout(500);
    expect(loadedEntry(requests)).toBeUndefined();
    await page.close();
  });

  test('opens an older Colab, which names no entry and has no module, by its own link', async () => {
    const { page } = await openApp('/colab/', async (page) => {
      await page.route('**/sdk/mount', (route) =>
        route.fulfill({ json: { address: `${origin}${PREFIX}` } }),
      );
      await page.route(`**${PREFIX}/x/-/shell`, (route) =>
        route.fulfill({ json: composed(MOUNT, undefined) }),
      );
      await page.route(`**${MOUNT}assets/colab-module.js`, (route) =>
        route.fulfill({ status: 404 }),
      );
    });
    await expect(page.locator('.tmt-ui-shell-main')).toContainText('Colab could not start here');
    await expect(page.locator(`.tmt-ui-shell-main a[href$="${MOUNT}"]`)).toHaveText('Open Colab');
    await page.close();
  });

  test('falls back to the module link when its entry is refused', async () => {
    const { page } = await openColab(composed(MOUNT, 'https://evil.example/colab.js'));
    await expect(page.locator(`.tmt-ui-shell-main a[href$="${MOUNT}"]`)).toBeVisible();
    await page.close();
  });
});

/**
 * The Ops route on the owner browser read `x/-/ops` (#4381). The door's read needs a live owner
 * session, so these stub the mount answer and the read: the page's own fetch, timer and states are
 * what is under test (the door's read has its own Rust tests and vectors).
 */
test.describe('the Ops route on the owner read', () => {
  const vectors = JSON.parse(
    readFileSync(new URL('../../../../../contracts/owner-reads-v1.json', import.meta.url), 'utf8'),
  ) as { cases: { target: string; expect: Record<string, unknown> }[] };
  const board = vectors.cases.find((vector) => vector.target === 'ops')!.expect;
  async function stub(page: Page, answers: Record<string, unknown>[]) {
    const reads: number[] = [];
    await page.route(`${origin}/sdk/mount`, (route) =>
      route.fulfill({ json: { address: `${origin}/r/owner` } }),
    );
    await page.route(`${origin}/r/owner/x/-/ops`, (route) => {
      reads.push(Date.now());
      const next = answers.length > 1 ? answers.shift()! : answers[0]!;
      return route.fulfill({ json: { ...next, observedAtMs: Date.now() } });
    });
    return reads;
  }

  test('shows the board, refreshes it while visible and stops on leaving the route', async () => {
    let reads: number[] = [];
    const changed = structuredClone(board) as { agents: { task?: string }[] };
    changed.agents[0]!.task = 'merge';
    const { page } = await openApp('/ops', async (page) => {
      reads = await stub(page, [board, changed]);
    });
    const rows = page.locator('#ops-section tbody tr:not(.tmt-ui-table-group)');
    await expect(page.locator('#ops-section .tmt-ui-page-title')).toHaveText('Your team, in sync.');
    await expect(rows).toHaveCount(2);
    // Lead first: the name with its mark, the state as a chip, what it works on, the model.
    await expect(rows.nth(0).locator('.tmt-ui-table-select')).toHaveText('tmt-lead');
    await expect(rows.nth(0).locator('.tmt-ui-chip')).toHaveText('Working');
    await expect(rows.nth(0)).toContainText('review');
    await expect(rows.nth(1).locator('.tmt-ui-state-mark').first()).toHaveAttribute(
      'data-shape',
      'ring',
    );
    await expect(rows.nth(1).locator('.tmt-ui-chip').first()).toContainText('Offline');
    // Every mark is drawn in one box, whatever the state.
    const boxes = await page
      .locator('#ops-section .tmt-ui-state-mark')
      .evaluateAll((marks) =>
        marks.map((mark) => [
          mark.getBoundingClientRect().width,
          mark.getBoundingClientRect().height,
        ]),
      );
    expect(new Set(boxes.map((box) => box.join('x')))).toEqual(new Set(['12x12']));
    // Each state's own mark, and the timer's clock, keep that box too.
    const shapes = [...Object.values(AGENT_STATES).map((state) => state.mark), 'clock'];
    const drawn = await page.evaluate((names) => {
      const host = document.querySelector('#ops-section')!;
      const sizes = names.map((shape) => {
        const mark = document.createElement('span');
        mark.className = 'tmt-ui-state-mark';
        mark.dataset.shape = shape;
        host.append(mark);
        const box = mark.getBoundingClientRect();
        mark.remove();
        return `${box.width}x${box.height}`;
      });
      return sizes;
    }, shapes);
    expect(new Set(drawn)).toEqual(new Set(['12x12']));
    // One strip: all agents, then each state (idle and idle alert count as Idle); a count at 0
    // stays. The vector's offline agent waits on the person, and offline wins.
    const stats = page.locator('#ops-section .tmt-ui-stat');
    await expect(stats.locator('.tmt-ui-stat-value')).toHaveText([
      '2 agents',
      '1 working',
      '0 needs you',
      '0 blocked',
      '0 idle',
      '1 offline',
      '0 unknown',
    ]);
    await expect(stats.nth(1)).toHaveAttribute('data-tone', 'working');
    await expect(stats.nth(1).locator('.tmt-ui-stat-number')).toHaveCSS(
      'font-family',
      /Bricolage Grotesque/,
    );
    // Nobody needs the person, so there is no attention banner.
    await expect(page.locator('#ops-section .tmt-ui-banner')).toHaveCount(0);
    await expect(page.locator('#ops-section .tmt-ui-table-group-toggle')).toContainText('tmt-core');
    // The next read, five seconds on, repaints only what changed.
    await expect(rows.nth(0)).toContainText('merge', { timeout: 8000 });
    await page.goto(`${origin}/ops#schedules`);
    await expect(page.locator('#ops-section .tmt-ui-page-title')).toHaveText(
      'Keep the rhythm visible.',
    );
    const timer = page.locator('#ops-section tbody tr');
    await expect(timer.locator('td').nth(0)).toContainText('Hourly pass');
    await expect(timer.locator('td').nth(1)).toHaveText('tmt-core');
    // The schedule is shown as recorded, in the mono face.
    await expect(timer.locator('td').nth(2)).toHaveText('cron 35 * * * *');
    await expect(timer.locator('td').nth(2)).toHaveCSS('font-family', /mono/i);
    await expect(timer.locator('td').nth(3)).toHaveText('tmt-core-pm');
    await expect(timer.locator('td').nth(4)).toHaveText('c1');
    await expect(page.locator('#ops-section .tmt-ui-card-head .tmt-ui-chip')).toHaveText(
      'Read only',
    );
    await expect(page.locator('#ops-section')).toContainText('Schedules are shown as recorded.');
    // How to manage them: the one command, with a Copy that says what happened.
    await expect(page.locator('#ops-section')).toContainText(
      'Use the Ops workspace to manage your timers. This web view does not change schedules.',
    );
    await expect(page.locator('#ops-section .tmt-ui-command-text')).toHaveText('tmt ops ui');
    await page.context().grantPermissions(['clipboard-write'], { origin });
    await page.getByRole('button', { name: 'Copy tmt ops ui' }).click();
    await expect(page.locator('#ops-section .tmt-ui-command-feedback')).toHaveText('Copied.');
    await page.goto(`${origin}/ops#settings`);
    await expect(page.locator('#ops-section')).toContainText(
      'The Ops settings page is not in the browser yet.',
    );
    await page.locator('nav.tmt-ui-rail a[data-id="digest"]').click();
    await expect(page.locator('.tmt-ui-shell-main')).toHaveAttribute('data-route', 'digest');
    const left = reads.length;
    await page.waitForTimeout(6000);
    expect(reads).toHaveLength(left);
    await page.close();
  });

  /** A board of two squads: core has an agent that needs the person, ux has an idle lead. */
  const squadBoard = () => {
    const agent = (name: string, squad: string, extra: Record<string, unknown> = {}) => ({
      id: `id-${name}`,
      name,
      squad,
      role: name.endsWith('lead') ? 'lead' : 'member',
      presence: 'active',
      waitingOnYou: 0,
      model: 'sonnet',
      ...extra,
    });
    const squads = [
      {
        name: 'tmt-core',
        layout: 'team',
        lead: { id: 'id-tmt-lead', name: 'tmt-lead' },
        idleAlertMs: 1800000,
      },
      { name: 'tmt-ux', layout: 'team' },
    ];
    const agents = [
      agent('core-1', 'tmt-core', { pending: 'approve the plan', waitingOnYou: 1, task: '#12' }),
      agent('tmt-lead', 'tmt-core', { stateTone: 'working', task: 'review' }),
      agent('core-3', 'tmt-core'),
      agent('ux-lead', 'tmt-ux', { stateTone: 'dim' }),
      agent('ux-1', 'tmt-ux', { presence: 'offline' }),
    ];
    return { version: 1, state: 'ok', squads, agents, timers: [], truncated: false };
  };

  test('filters, searches, groups and selects agents, and keeps them across a refresh', async () => {
    const first = squadBoard();
    const changed = structuredClone(first);
    Object.assign(changed.agents[1]!, { task: 'merge' });
    const { page } = await openApp('/ops', async (page) => {
      await stub(page, [first, changed]);
    });
    const section = page.locator('#ops-section');
    const rows = section.locator('tbody tr:not(.tmt-ui-table-group)');
    await expect(rows).toHaveCount(5);
    // Squads in board order, each lead first.
    await expect(rows.locator('.tmt-ui-table-select')).toHaveText([
      'tmt-lead',
      'core-1',
      'core-3',
      'ux-lead',
      'ux-1',
    ]);
    // Idle and unknown have their own counts.
    await expect(section.locator('.tmt-ui-stat-value')).toHaveText([
      '5 agents',
      '1 working',
      '1 needs you',
      '0 blocked',
      '1 idle',
      '1 offline',
      '1 unknown',
    ]);
    // What needs the person is above the table; View all filters to it.
    const banner = section.locator('.tmt-ui-banner');
    await expect(banner).toContainText('1 agent needs your attention');
    await expect(banner.locator('.tmt-ui-banner-row')).toContainText(['core-1']);
    await banner.getByRole('button', { name: 'View all' }).click();
    await expect(section.getByRole('tab', { name: 'Needs you' })).toHaveAttribute(
      'aria-selected',
      'true',
    );
    await expect(rows.locator('.tmt-ui-table-select')).toHaveText(['core-1']);
    await section.getByRole('tab', { name: 'Idle' }).click();
    await expect(rows.locator('.tmt-ui-table-select')).toHaveText(['ux-lead']);
    await section.getByRole('tab', { name: 'Unknown' }).click();
    await expect(rows.locator('.tmt-ui-table-select')).toHaveText(['core-3']);
    await section.getByRole('tab', { name: 'All' }).click();
    // Search matches the name, squad or work.
    const search = section.getByRole('searchbox', { name: 'Filter agents' });
    await search.fill('ux');
    await expect(rows.locator('.tmt-ui-table-select')).toHaveText(['ux-lead', 'ux-1']);
    await search.fill('nobody');
    await expect(section).toContainText('No agents match.');
    await search.fill('');
    // The first agent that needs the person is selected on load, so the panel is never empty.
    const detail = page.locator('#ops-detail');
    await expect(detail.locator('dd').first()).toHaveText('core-1');
    await expect(section.locator('.tmt-ui-table-select', { hasText: 'core-1' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
    // A squad collapses to its header; the pressed agent fills the detail panel.
    const toggle = section.locator('.tmt-ui-table-group-toggle', { hasText: 'tmt-ux' });
    await toggle.click();
    await expect(toggle).toHaveAttribute('aria-expanded', 'false');
    await expect(rows).toHaveCount(3);
    await section.getByRole('button', { name: 'tmt-lead' }).click();
    await expect(detail.locator('dd').first()).toHaveText('tmt-lead');
    await expect(detail).toContainText('review');
    // The search, the collapsed squad and the selection survive the next read, five seconds on.
    await search.fill('tmt-lead');
    await expect(detail).toContainText('merge', { timeout: 8000 });
    await expect(search).toHaveValue('tmt-lead');
    await expect(rows.locator('.tmt-ui-table-select')).toHaveText(['tmt-lead']);
    await expect(rows.first()).toHaveAttribute('data-selected', '');
    await search.fill('');
    await expect(toggle).toHaveAttribute('aria-expanded', 'false');
    await expect(rows).toHaveCount(3);
    // Pressing the selected agent again keeps it selected.
    await section.getByRole('button', { name: 'tmt-lead' }).click();
    await expect(detail.locator('dd').first()).toHaveText('tmt-lead');
    await expect(rows.first()).toHaveAttribute('data-selected', '');
    await page.close();
  });

  test('shows each squad as a card with its people, and opens an agent in a drawer', async () => {
    const { page } = await openApp('/ops#squads', async (page) => {
      await stub(page, [squadBoard()]);
    });
    const section = page.locator('#ops-section');
    await expect(section.locator('.tmt-ui-page-title')).toHaveText('Squads, clearly connected.');
    await expect(section.locator('.tmt-ui-page-subtitle')).toHaveText(
      'Every team, its lead and the people moving work forward.',
    );
    // The same count strip as the agents page.
    await expect(section.locator('.tmt-ui-stat-value')).toHaveText([
      '5 agents',
      '1 working',
      '1 needs you',
      '0 blocked',
      '1 idle',
      '1 offline',
      '1 unknown',
    ]);
    const cards = section.locator('.ops-squad-card');
    await expect(cards).toHaveCount(2);
    // Side by side, equal columns.
    const boxes = await cards.evaluateAll((nodes) => nodes.map((n) => n.getBoundingClientRect()));
    expect(boxes[0]!.top).toBe(boxes[1]!.top);
    expect(boxes[0]!.width).toBe(boxes[1]!.width);
    await expect(cards.nth(0).locator('.tmt-ui-card-title')).toHaveText('tmt-core');
    await expect(cards.nth(0).locator('.ops-squad-meta')).toHaveText('Lead tmt-lead · 3 agents');
    await expect(cards.nth(1).locator('.ops-squad-meta')).toHaveText('2 agents');
    // The people as an avatar stack, the lead first.
    await expect(cards.nth(0).locator('.tmt-ui-avatar')).toHaveText(['TL', 'C1', 'C3']);
    // One row per agent: mark, name, state chip.
    const members = cards.nth(0).locator('.ops-squad-member');
    await expect(members.locator('strong')).toHaveText(['tmt-lead', 'core-1', 'core-3']);
    await expect(members.locator('.tmt-ui-chip')).toHaveText(['Working', 'Needs you', 'Unknown']);
    await expect(members.first().locator('.tmt-ui-state-mark').first()).toHaveAttribute(
      'data-shape',
      'dot',
    );
    // The footer: the idle alert and a Read only badge.
    await expect(cards.nth(0).locator('.ops-squad-bottom')).toContainText(
      'Idle alert after 30 minutes',
    );
    await expect(cards.nth(1).locator('.ops-squad-bottom')).toContainText(
      'Idle alert not configured',
    );
    await expect(cards.nth(0).locator('.ops-squad-bottom .tmt-ui-chip')).toHaveText('Read only');
    // An agent opens in a read-only drawer; Escape or Close returns to the row that opened it.
    const drawer = page.locator('dialog.tmt-ui-drawer');
    await members.first().click();
    await expect(drawer).toBeVisible();
    await expect(drawer.locator('h2')).toHaveText('tmt-lead');
    await expect(drawer.locator('dd')).toHaveText(['tmt-core', 'Lead', 'sonnet']);
    await expect(drawer).toContainText('review');
    await expect(drawer).toContainText('This panel does not control an agent.');
    await page.keyboard.press('Escape');
    await expect(drawer).toBeHidden();
    await expect(members.first()).toBeFocused();
    await members.nth(1).click();
    await expect(drawer).toContainText('approve the plan');
    await drawer.getByRole('button', { name: 'Close core-1' }).click();
    await expect(drawer).toBeHidden();
    await page.close();
  });

  test('refreshes an open squad drawer and returns focus to the current member row', async () => {
    let current = squadBoard();
    const { page } = await openApp('/ops#squads', async (page) => {
      await stub(page, [current]);
      await page.route(`${origin}/r/owner/x/-/ops`, (route) =>
        route.fulfill({ json: { ...current, observedAtMs: Date.now() } }),
      );
    });
    const row = page.getByRole('button', { name: 'core-1, Needs you', exact: true });
    await row.click();
    const drawer = page.getByRole('dialog', { name: 'core-1', exact: true });
    await expect(drawer).toContainText('approve the plan');
    current = structuredClone(current);
    Object.assign(current.agents[0]!, {
      task: 'The refreshed task',
      next: 'The refreshed next step',
      pending: '',
      waitingOnYou: 0,
      stateTone: 'working',
    });
    await expect(drawer).toContainText('The refreshed task', { timeout: 8000 });
    await expect(drawer).toContainText('The refreshed next step');
    await expect(drawer).not.toContainText('approve the plan');
    await expect(drawer.locator('.ops-drawer-top .tmt-ui-chip').first()).toHaveText('Working');
    await page.keyboard.press('Escape');
    await expect(drawer).toBeHidden();
    await expect(page.getByRole('button', { name: 'core-1, Working', exact: true })).toBeFocused();
    await page.close();
  });

  test('reports a removed drawer agent, focuses the page fallback and closes on section departure', async () => {
    let current = squadBoard();
    const { page } = await openApp('/ops#squads', async (page) => {
      await stub(page, [current]);
      await page.route(`${origin}/r/owner/x/-/ops`, (route) =>
        route.fulfill({ json: { ...current, observedAtMs: Date.now() } }),
      );
    });
    await page.getByRole('button', { name: 'core-1, Needs you', exact: true }).click();
    const drawer = page.getByRole('dialog', { name: 'core-1', exact: true });
    await expect(drawer).toContainText('approve the plan');
    current = { ...current, agents: current.agents.filter((agent) => agent.name !== 'core-1') };
    await expect(drawer).toContainText('This agent is no longer on the board.', { timeout: 8000 });
    await expect(drawer).not.toContainText('approve the plan');
    await expect(drawer.locator('dd')).toHaveCount(0);
    await page.keyboard.press('Escape');
    await expect(page.getByRole('heading', { name: 'Squads, clearly connected.' })).toBeFocused();

    await page.getByRole('button', { name: 'tmt-lead, Working', exact: true }).click();
    await expect(page.getByRole('dialog', { name: 'tmt-lead', exact: true })).toBeVisible();
    // History can change while a modal is open. A detached native dialog must not retain `open`.
    await page.evaluate(() => {
      location.hash = 'schedules';
    });
    await expect(page.getByRole('heading', { name: 'Keep the rhythm visible.' })).toBeVisible();
    await page.evaluate(() => {
      location.hash = 'squads';
    });
    await expect(page.locator('dialog.tmt-ui-drawer')).not.toHaveAttribute('open');
    await page.getByRole('button', { name: 'tmt-lead, Working', exact: true }).click();
    await expect(page.getByRole('dialog', { name: 'tmt-lead', exact: true })).toBeVisible();
    await page
      .locator('nav.tmt-ui-rail a[data-id="digest"]')
      .evaluate((node: HTMLAnchorElement) => node.click());
    await expect(page.locator('.tmt-ui-shell-main')).toHaveAttribute('data-route', 'digest');
    await expect(page.locator('dialog.tmt-ui-drawer')).toHaveCount(0);
    await page.close();
  });

  test('a door that stops answering mid-route is retried, never a reason to pair again', async () => {
    let down = false;
    const { page } = await openApp('/ops', async (page) => {
      await stub(page, [board]);
      // The door stops: the read fails on the network, as a restart or a blip does.
      await page.route(`${origin}/r/owner/x/-/ops`, (route) =>
        down ? route.abort('connectionrefused') : route.fallback(),
      );
    });
    const rows = page.locator('#ops-section tbody tr:not(.tmt-ui-table-group)');
    await expect(rows).toHaveCount(2);
    down = true;
    await expect(page.locator('#ops-note')).toHaveText(
      'Your machine did not answer. This page tries again shortly.',
      { timeout: 8000 },
    );
    await expect(page.locator('#ops-note')).not.toContainText('Pair');
    // Back up: the next read (after the doubled wait) shows the board again and clears the note.
    down = false;
    await expect(rows).toHaveCount(2, { timeout: 15_000 });
    await expect(page.locator('#ops-note')).toBeHidden();
    await page.close();
  });

  test('says why there is no board, never an empty list', async () => {
    const { page } = await openApp('/ops', async (page) => {
      await stub(page, [{ version: 1, state: 'unavailable', reason: 'not_installed' }]);
    });
    await expect(page.locator('#ops-note')).toHaveText(
      'Ops is not installed on your machine. Run tmt extension install ops, then reload this page.',
    );
    await expect(page.locator('#ops-section .tmt-ui-list-row')).toHaveCount(0);
    await page.close();
  });

  test('without an owner session it asks to pair', async () => {
    const { page } = await openApp('/ops');
    await expect(page.locator('#ops-note')).toContainText('Pair it from Remote');
    await expect(page.locator('#ops-section .tmt-ui-list-row')).toHaveCount(0);
    await page.close();
  });
});

/**
 * The Digest route on the owner reads `x/-/digest/<range>` and `x/-/limits`: the page's pieces are
 * the leaf's (card, status tile, segments, meter, badge, table), not page-local copies.
 */
test.describe('the Digest route on the owner reads', () => {
  async function stub(page: Page) {
    await page.route(`${origin}/sdk/mount`, (route) =>
      route.fulfill({ json: { address: `${origin}/r/owner` } }),
    );
    await page.route(new RegExp(`${origin}/r/owner/x/-/digest/(24h|7d|30d)$`), (route) => {
      const range = route.request().url().split('/').pop() as UsageRange;
      return route.fulfill({
        json: {
          version: 1,
          state: 'ok',
          since: range,
          report: mockDocument(range),
          observedAtMs: Date.now(),
        },
      });
    });
    await page.route(`${origin}/r/owner/x/-/limits`, (route) =>
      route.fulfill({
        json: { version: 1, state: 'unavailable', reason: 'not_sampled', observedAtMs: Date.now() },
      }),
    );
  }

  for (const width of [1440, 390])
    test(`draws the website allowance rows, charts, tabs, meters and coverage from the leaf pieces at ${width}`, async () => {
      const { page } = await openApp('/digest', async (page) => {
        await page.setViewportSize({ width, height: 900 });
        await stub(page);
      });
      const cards = page.locator('#digest-sections .tmt-ui-card');
      await expect(cards.first()).toBeVisible();
      expect(await cards.count()).toBe(5);
      await expect(cards.first()).toHaveCSS('border-radius', '8px');
      const provider = page.locator('.digest-provider').first();
      const details = provider.locator('details');
      await expect(provider.locator('.digest-provider-label')).toContainText('% left');
      await expect(details).not.toHaveAttribute('open');
      await details.locator('summary').click();
      await expect(provider.locator('.tmt-ui-status-tile-label')).toHaveText([
        'Elapsed',
        'Pace',
        'Resets in',
      ]);
      await expect(provider.locator('.tmt-ui-meter-fill')).toHaveCSS(
        'background-color',
        /rgb\(\d+, \d+, \d+\)/,
      );
      const left = await provider
        .locator('.tmt-ui-meter-fill')
        .evaluate((node) => (node as HTMLElement).style.getPropertyValue('--tmt-ui-meter-fill'));
      expect(Number(left)).toBeGreaterThan(0);
      // The range is the leaf's boxed tabs, one selected; choosing 7d reads that range.
      const ranges = page.locator('#usage-ranges .tmt-ui-tab');
      await expect(ranges).toHaveText(['24h', '7d', '30d']);
      await expect(ranges.first()).toHaveAttribute('aria-selected', 'true');
      await ranges.nth(1).click();
      await expect(ranges.nth(1)).toHaveAttribute('aria-selected', 'true');
      await expect(page.locator('#digest-ranking-rows')).toBeVisible();
      expect((await page.locator('#usage-ranges').boundingBox())!.width).toBeGreaterThan(80);
      // The token scale is the same tabs for the chart below it.
      const scale = page.locator('#digest-tokens .tmt-ui-tab');
      await expect(scale).toHaveText(['Absolute', 'Share']);
      await expect(scale.first()).toHaveAttribute('aria-selected', 'true');
      expect(
        (await page.locator('#digest-tokens .tmt-ui-tab-list').boundingBox())!.width,
      ).toBeGreaterThan(80);
      await expect(page.locator('#digest-missing .digest-gap dd')).toHaveText(
        Array(4).fill('Not connected'),
      );
      await expect(page.locator('.digest-provider details').first()).toHaveAttribute('open', '');
      const heading = page.locator('#digest-ranking-rows .tmt-ui-table[data-numeric] th').nth(1);
      await expect(heading).toHaveText('Tokens');
      expect((await heading.boundingBox())!.height).toBeLessThan(40);
      // Nothing is a page-local copy any more.
      await expect(
        page.locator(
          '.digest-badge, .digest-segment, .digest-segments, .digest-stat, .digest-meter, .tmt-ui-segments, .tmt-ui-badge',
        ),
      ).toHaveCount(0);
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        width,
      );
      await page.close();
    });
});
