import { expect, test, chromium, type Browser, type Page, type Request } from '@playwright/test';
import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';

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
    // A query, an escape or an empty inner segment is a malformed target (400),
    // never the shell: the app's routes carry no query and the fallback stays an explicit list.
    for (const path of ['/settings?section=modules', '/colab/a%2Fb', '/colab//x'])
      for (const method of ['GET', 'HEAD'])
        expect((await get(path, { method })).status, `${method} ${path}`).toBe(400);
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

  test("loads a page's own stylesheet with its route and drops it on leaving", async () => {
    const { page } = await openApp('/remote/settings');
    const sheet = page.locator('head link[rel="stylesheet"][href="/sdk/settings.css"]');
    await expect(sheet).toHaveCount(1);
    // Its rules apply before the markup shows: the page's choice control keeps its arrow room.
    await expect(page.locator('.tmt-ui-shell-main')).toHaveAttribute(
      'data-route',
      'remote-settings',
    );
    await page.locator('nav.tmt-ui-rail a[data-id="ops"]').click();
    await expect(page.locator('.tmt-ui-shell-main')).toHaveAttribute('data-route', 'ops');
    await expect(sheet).toHaveCount(0);
    await page.goBack();
    await expect(page.locator('.tmt-ui-shell-main')).toHaveAttribute(
      'data-route',
      'remote-settings',
    );
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
      'search',
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
