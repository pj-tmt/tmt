import { expect, test, type Frame, type Page } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { showAllPages } from '../test/home-list.js';
import {
  binariesBuilt,
  remoteBinary,
  startIsolatedDoor,
  type IsolatedDoor,
} from './isolated-door.js';

/**
 * Conditions for Colab as a route of the one TMT web app at `/colab/...` (#3937; feat-lead's and
 * tmt-lead's conditions): every link form printed so far lands on `/colab/...` with its search
 * keys, a reload of a page route works, the app document's policy is no looser than today's,
 * the renderer stays a separate sandboxed opaque frame, and unmounting releases what Colab started.
 *
 * The router and module tests run over the local transports (test/spa-links-browser.tsx). The
 * door tests run against a private door: the real tmt-remote and tmt-colab binaries in a throwaway
 * HOME (e2e/isolated-door.ts; build them with `cargo build -p tmt-remote -p tmt-colab`, or set
 * TMT_REMOTE_BINARY and TMT_COLAB_BINARY). TMT_SPA_DOOR_ORIGIN instead points the unpaired checks at
 * a door that is already running. The paired tests pair through the door's own pairing page.
 */
const fixture = '/test/spa-links-browser.tsx';
const space = '10000000-0000-4000-8000-000000000001';
const page1 = '12345678-0000-4000-8000-000000000001';
const reading = '10000000-0000-4000-8000-000000000401';
const externalDoor = process.env.TMT_SPA_DOOR_ORIGIN;

const policySource = readFileSync(
  new URL('../../../rust/tmt-colab/src/browser_policy.rs', import.meta.url),
  'utf8',
);
const constant = (name: string) =>
  policySource.match(new RegExp(`pub const ${name}: &str = "([^"]+)";`))?.[1] ?? '';
/** Today's app document policy and the renderer's, from their one native owner. */
const APP_POLICY = constant('POLICY');
const RENDERER_POLICY = constant('RENDERER_POLICY');

/** Whether `actual` allows nothing that `baseline` does not: every baseline directive is present
 * and each of its sources is a source of the baseline, or the directive is `'none'`. Extra
 * directives only narrow what a document may do. */
function atLeastAsStrict(actual: string, baseline: string): string[] {
  const parse = (policy: string) =>
    new Map(
      policy
        .split(';')
        .map((directive) => directive.trim().split(/\s+/))
        .filter((parts) => parts[0])
        .map(([name, ...sources]) => [name!.toLowerCase(), new Set(sources)] as const),
    );
  const have = parse(actual);
  const problems: string[] = [];
  for (const [name, allowed] of parse(baseline)) {
    // A directive the page leaves out falls back to its default-src.
    const sources = have.get(name) ?? (name.endsWith('-src') ? have.get('default-src') : undefined);
    if (!sources) {
      problems.push(`${name} is missing`);
      continue;
    }
    if (sources.has("'none'") && sources.size === 1) continue;
    for (const source of sources)
      if (!allowed.has(source)) problems.push(`${name} allows ${source}`);
  }
  return problems;
}

// Renderer routing may still be fulfilling when a home-only assertion completes.
// Finish those callbacks before Playwright closes this fixture's page.
test.afterEach(async ({ page }) => {
  await page.unrouteAll({ behavior: 'wait' });
});

/** The door answers `/colab/renderer.html` with the renderer document and its policy; the dev
 * server serves the same file at `/renderer.html`. */
async function serveRendererLikeTheDoor(page: Page) {
  const origin = new URL(page.url()).origin;
  await page.route('**/colab/renderer.html', async (route) =>
    route.fulfill({ response: await route.fetch({ url: `${origin}/renderer.html` }) }),
  );
}

/** The renderer frame once it has navigated: the iframe element and its `src` exist before the frame's
 * own URL does, and until then the frame is still `about:blank`. */
async function rendererFrame(page: Page): Promise<Frame> {
  const find = () =>
    page.frames().find((candidate) => candidate.url().endsWith('/colab/renderer.html'));
  await expect.poll(find).toBeDefined();
  return find()!;
}

async function open(page: Page, url: string) {
  await page.goto('/');
  await serveRendererLikeTheDoor(page);
  await page.evaluate(async ({ fixture, url }) => (await import(fixture)).mountAt(url), {
    fixture,
    url,
  });
}
const here = (page: Page) => page.evaluate(() => location.pathname + location.search);

const owner = `#space=${space}&path=${encodeURIComponent(`/pages/${page1}`)}`;
const forms: [string, string, string][] = [
  ['the printed short link', '/colab/p/12345678', `/colab/pages/${page1}`],
  [
    'a short link with a reading',
    `/colab/p/12345678#read=${reading}`,
    `/colab/pages/${page1}?read=${reading}`,
  ],
  [
    'a short link with an anchor',
    '/colab/p/12345678#anchor=digest-matrix',
    `/colab/pages/${page1}?anchor=digest-matrix`,
  ],
  ['a short link with a thread', `/colab/p/12345678#t=${reading}`, `/colab/pages/${page1}`],
  ['an owner link on the moved mount', `/colab/${owner}`, `/colab/pages/${page1}`],
  [
    'an owner link with a reading',
    `/colab/${owner}&read=${reading}`,
    `/colab/pages/${page1}?read=${reading}`,
  ],
  [
    'an owner link with an anchor',
    `/colab/${owner}&anchor=digest-matrix`,
    `/colab/pages/${page1}?anchor=digest-matrix`,
  ],
  [
    'an owner link that kept its query',
    `/colab/?read=${reading}${owner}`,
    `/colab/pages/${page1}?read=${reading}`,
  ],
  ['the preview hash path', `/colab/#/pages/${page1}`, `/colab/pages/${page1}`],
  ['the page route itself', `/colab/pages/${page1}`, `/colab/pages/${page1}`],
  [
    'a page route with a reading',
    `/colab/pages/${page1}?read=${reading}`,
    `/colab/pages/${page1}?read=${reading}`,
  ],
];
for (const [name, start, landing] of forms) {
  test(`${name} lands on ${landing}`, async ({ page }) => {
    await open(page, start);
    await expect.poll(() => here(page)).toBe(landing);
    await expect(
      page.frameLocator('iframe').getByRole('heading', { name: 'Original content' }),
    ).toBeVisible();
  });
}

test('a page link in a body opens in the one document, not as a new load', async ({ page }) => {
  await page.goto('/');
  await serveRendererLikeTheDoor(page);
  await page.evaluate(
    async ({ fixture, url }) =>
      (await import(fixture)).mountAt(url, 'See [the proposal](/p/12345678#digest-matrix) now'),
    {
      fixture,
      url: '/colab/',
    },
  );
  const link = page.locator('#spa-body').getByRole('link', { name: 'the proposal' });
  // Inside the app the printed link is the app's own short location.
  await expect(link).toHaveAttribute('href', '/colab/p/12345678#digest-matrix');
  await page.evaluate(() => {
    (window as unknown as { __document: string }).__document = 'same';
  });
  await link.click();
  await expect.poll(() => here(page)).toBe(`/colab/pages/${page1}?anchor=digest-matrix`);
  // The same document: its own state survived, and no navigation was recorded after the first.
  expect(await page.evaluate(() => (window as unknown as { __document?: string }).__document)).toBe(
    'same',
  );
  expect(await page.evaluate(() => performance.getEntriesByType('navigation').length)).toBe(1);
});

test('a short link with a thread keeps the thread fragment on the page route', async ({ page }) => {
  await open(page, `/colab/p/12345678#t=${reading}`);
  await expect.poll(() => here(page)).toBe(`/colab/pages/${page1}`);
  // The page route carries the fragment the deep-thread handler reads.
  expect(await page.evaluate(() => location.hash)).toBe(`#t=${reading}`);
});

test('the printed home link and a bare mount land on the Colab home', async ({ page }) => {
  for (const start of ['/colab/', `/colab/#space=${space}`]) {
    await open(page, start);
    await expect.poll(() => here(page)).toBe('/colab/');
    await expect(page.getByText('Original proposal')).toBeVisible();
  }
});

test('an owner link to the old list lands on the home and keeps no stale route', async ({
  page,
}) => {
  await open(page, `/colab/#space=${space}&path=%2Fall`);
  await expect.poll(() => here(page)).toBe('/colab/');
});

test('an open key on a link survives to the page route', async ({ page }) => {
  await open(page, `/colab/p/12345678#open=${page1}`);
  await expect.poll(() => here(page)).toBe(`/colab/pages/${page1}?open=${page1}`);
});

test('the renderer is still a separate sandboxed frame with an opaque origin', async ({ page }) => {
  await open(page, `/colab/pages/${page1}`);
  const frame = page.locator('iframe');
  await expect(frame).toHaveCount(1);
  // Scripts run, and nothing else: never the app's origin, forms, popups or navigation.
  await expect(frame).toHaveAttribute('sandbox', 'allow-scripts');
  await expect(frame).toHaveAttribute('referrerpolicy', 'no-referrer');
  const src = new URL((await frame.getAttribute('src'))!, page.url());
  expect(src.pathname).toBe('/colab/renderer.html');
  // The frame is its own document, not the app's, and its origin is opaque.
  const inside = await rendererFrame(page);
  expect(await inside.evaluate(() => window.origin)).toBe('null');
  expect(await frame.evaluate((node) => (node as HTMLIFrameElement).contentDocument)).toBe(null);
  expect(
    await page.evaluate(() => (document.querySelector('iframe') as HTMLIFrameElement).srcdoc),
  ).toBe('');
});

test('a page that opts in to the runtime renders in the shell with the runtime from /colab/assets/', async ({
  page,
}) => {
  const fetched: string[] = [];
  await page.route('**/assets/colab-runtime.*', (route) => {
    const url = new URL(route.request().url());
    fetched.push(url.pathname);
    return url.pathname.endsWith('.css')
      ? route.fulfill({ contentType: 'text/css', body: '@theme { --color-stub: #123456; }' })
      : route.fulfill({
          contentType: 'text/javascript',
          body: 'window.tmt = Object.freeze({ stub: true });',
        });
  });
  await page.goto('/');
  await serveRendererLikeTheDoor(page);
  await page.evaluate(
    async ({ fixture, url }) =>
      (await import(fixture)).mountAt(
        url,
        undefined,
        '<meta name="tmt-colab-runtime" content="react tailwind"><p id="opted">Marker</p>' +
          '<script>document.getElementById("opted").dataset.tmt = String(window.tmt && window.tmt.stub)</script>',
      ),
    { fixture, url: `/colab/pages/${page1}` },
  );
  // The renderer and the runtime it loads share one base: beside /colab/, not the page's route.
  const frame = page.locator('iframe');
  await expect(frame).toHaveAttribute('src', /\/colab\/renderer\.html/);
  const inside = await rendererFrame(page);
  await expect(inside.locator('#opted')).toHaveAttribute('data-tmt', 'true');
  expect(fetched.sort()).toEqual([
    '/colab/assets/colab-runtime.css',
    '/colab/assets/colab-runtime.js',
  ]);
});

test('the policy comparison accepts a stricter document and names every loosening', () => {
  expect(APP_POLICY).toContain("default-src 'none'");
  expect(atLeastAsStrict(APP_POLICY, APP_POLICY)).toEqual([]);
  expect(atLeastAsStrict(`${APP_POLICY}; require-trusted-types-for 'script'`, APP_POLICY)).toEqual(
    [],
  );
  expect(
    atLeastAsStrict(APP_POLICY.replace("script-src 'self'", "script-src 'none'"), APP_POLICY),
  ).toEqual([]);
  expect(
    atLeastAsStrict(
      APP_POLICY.replace("script-src 'self'", "script-src 'self' 'unsafe-inline'"),
      APP_POLICY,
    ),
  ).toEqual(["script-src allows 'unsafe-inline'"]);
  expect(
    atLeastAsStrict(APP_POLICY.replace("connect-src 'self'", 'connect-src *'), APP_POLICY),
  ).toEqual(['connect-src allows *']);
  expect(atLeastAsStrict(APP_POLICY.replace("frame-ancestors 'none'", ''), APP_POLICY)).toEqual([
    'frame-ancestors is missing',
  ]);
});

test.describe('the door serving the TMT app', () => {
  let door: IsolatedDoor | undefined;
  let origin: string;
  test.beforeAll(async () => {
    // TMT_SPA_DOOR_ORIGIN points the unpaired checks at a door that is already running; it cannot
    // pair a browser, so the paired checks need the isolated door below.
    if (externalDoor) {
      origin = externalDoor;
      return;
    }
    if (!binariesBuilt()) return;
    door = await startIsolatedDoor();
    origin = door.origin;
  });
  test.afterAll(async () => {
    await door?.stop();
  });
  test.beforeEach(() => {
    test.skip(
      !externalDoor && !door,
      'Needs the native binaries: cargo build -p tmt-remote -p tmt-colab (or TMT_REMOTE_BINARY and TMT_COLAB_BINARY)',
    );
  });

  test('the app document of a short link has a policy no looser than today’s app document', async ({
    request,
  }) => {
    const response = await request.get(`${origin}/colab/p/12345678`);
    expect(response.ok()).toBe(true);
    const policy = response.headers()['content-security-policy'];
    expect(policy, 'the document carries a Content-Security-Policy').toBeTruthy();
    expect(atLeastAsStrict(policy!, APP_POLICY)).toEqual([]);
  });

  test('every Colab route serves that same app document, unpaired', async ({ request }) => {
    const policies = new Set<string>();
    for (const path of [
      '/colab/',
      '/colab/p/12345678',
      `/colab/pages/${page1}`,
      '/colab/settings',
    ]) {
      const response = await request.get(`${origin}${path}`);
      expect(response.status(), path).toBe(200);
      policies.add(response.headers()['content-security-policy'] ?? '');
    }
    expect(policies.size).toBe(1);
  });

  test('the renderer is served by the Colab mount with its own policy, not by the app', async ({
    request,
  }) => {
    test.skip(!door, 'The mount path needs the isolated door’s machine address');
    const renderer = await request.get(`${door!.mount}renderer.html`);
    expect(renderer.ok()).toBe(true);
    expect(renderer.headers()['content-security-policy']).toBe(RENDERER_POLICY);
    expect(await renderer.text()).toContain('Colab page renderer');
    // The app origin has no renderer of its own: the frame never loads from the app document's path.
    expect((await request.get(`${origin}/colab/renderer.html`)).status()).toBe(404);
  });

  test('the old short entry redirects into /colab/ and keeps its path', async ({ request }) => {
    const short = await request.get(`${origin}/p/12345678`, { maxRedirects: 0 });
    expect(short.status()).toBe(308);
    expect(short.headers().location).toBe('/colab/p/12345678');
  });

  test('a reload of a page route serves the TMT app, unpaired', async ({ page }) => {
    await page.goto(`${origin}/colab/pages/${page1}`);
    const rail = page.getByRole('navigation', { name: 'TMT', exact: true });
    await expect(rail).toBeVisible();
    await page.reload();
    await expect(rail).toBeVisible();
    expect(new URL(page.url()).pathname).toBe(`/colab/pages/${page1}`);
  });

  test.describe('with a paired browser', () => {
    test('an old link to the Colab mount opens its page in /colab/ and keeps its keys', async ({
      page,
    }) => {
      // The door refuses a query on mount paths (unchanged), so old owner links carry their keys
      // in the fragment; the mount's document moves them into the app before reading anything.
      const id = await door!.createPage('Moved page', '<h1>Moved content</h1>');
      await door!.pair(page);
      await page.goto(`${door!.mount}#path=${encodeURIComponent(`/pages/${id}`)}&read=${reading}`);
      await expect.poll(() => here(page)).toBe(`/colab/pages/${id}?read=${reading}`);
      await expect(page.locator('.colab-route')).toBeVisible();
    });

    test('an owner link to another space lands on the blocked view', async ({ page }) => {
      const id = await door!.createPage('Foreign page', '<h1>Foreign content</h1>');
      await door!.pair(page);
      const other = 'a'.repeat(32);
      await page.goto(
        `${door!.mount}#space=${other}&path=${encodeURIComponent(`/pages/${id}`)}&read=${reading}`,
      );
      await expect.poll(() => here(page)).toBe('/colab/blocked');
      await expect(page.frameLocator('iframe').getByText('Foreign content')).toHaveCount(0);
    });

    test('a reload of a page route opens the same page in the TMT app', async ({ page }) => {
      const id = await door!.createPage('Reload page', '<h1>Reload content</h1>');
      await door!.pair(page);
      await page.goto(`${door!.origin}/colab/pages/${id}`);
      await expect(page.locator('.colab-route')).toBeVisible();
      await page.reload();
      await expect(page.locator('.colab-route')).toBeVisible();
      expect(new URL(page.url()).pathname).toBe(`/colab/pages/${id}`);
      await expect(
        page.frameLocator('iframe').getByRole('heading', { name: 'Reload content' }),
      ).toBeVisible();
    });

    test('a paired browser opens Colab again after the door restarts', async ({ page }) => {
      const id = await door!.createPage('Restart page', '<h1>Restart content</h1>');
      await door!.pair(page);
      await door!.restartRemote();
      // The door's sessions ended; the route reopens this browser's session from its pairing.
      await page.goto(`${door!.origin}/colab/pages/${id}`);
      await expect(page.locator('.colab-route')).toBeVisible();
      await expect(
        page.frameLocator('iframe').getByRole('heading', { name: 'Restart content' }),
      ).toBeVisible();
      await expect(
        page
          .getByRole('navigation', { name: 'TMT', exact: true })
          .getByRole('link', { name: 'Colab', exact: true }),
      ).not.toHaveAttribute('aria-disabled', 'true');
    });

    test('a tab whose Colab was upgraded before it mounted still shows the update row in the shell bar', async ({
      page,
    }) => {
      const id = await door!.createPage('Upgrade page', '<h1>Upgrade content</h1>');
      await door!.pair(page);
      // The upgrade landed after the Colab module loaded and before anything read the mount's
      // entry, so every read of it names an upgraded build and only the module can say which
      // build it is. Only fetches are rewritten, never a navigation.
      let reads = 0;
      await page.route(
        (url) => url.pathname === new URL(door!.mount).pathname,
        async (route) => {
          if (route.request().isNavigationRequest()) return route.continue();
          reads++;
          const response = await route.fetch();
          const html = (await response.text()).replace(
            /index-[A-Za-z0-9_-]{8}\.js/,
            'index-NEWNEW34.js',
          );
          await route.fulfill({ response, body: html });
        },
      );
      await page.goto(`${door!.origin}/colab/pages/${id}`);
      await expect(page.locator('.colab-route')).toBeVisible();
      const row = page.locator('.tmt-ui-shell-bar > [data-update-notice]');
      await expect(row).toBeVisible();
      await expect(row).toHaveAttribute('role', 'status');
      expect(reads).toBeGreaterThanOrEqual(1);
    });

    test('a tab that runs the build the door serves shows no update row', async ({ page }) => {
      const id = await door!.createPage('Current page', '<h1>Current content</h1>');
      await door!.pair(page);
      const mount = new URL(door!.mount).pathname;
      const read = page.waitForResponse(
        (response) =>
          new URL(response.url()).pathname === mount && !response.request().isNavigationRequest(),
      );
      await page.goto(`${door!.origin}/colab/pages/${id}`);
      await expect(page.locator('.colab-route')).toBeVisible();
      // The check has read the served build; give its result two frames to reach the shell.
      await (await read).finished();
      await page.evaluate(
        () =>
          new Promise<void>((resolve) =>
            requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
          ),
      );
      await expect(page.locator('[data-update-notice]')).toHaveCount(0);
    });

    test('the list tab survives reload, Back and Forward in the TMT app', async ({ page }) => {
      const id = await door!.createPage('Tab page', '<h1>Tab content</h1>');
      await door!.pair(page);
      await page.goto(`${door!.origin}/colab/`);
      // A fresh visit to the bare list opens the default tab, All.
      await expect(page.getByRole('tab', { name: /^All, \d+\+?$/ })).toHaveAttribute(
        'aria-selected',
        'true',
      );
      await expect(page.locator(`[data-page-id="${id}"] a`)).toBeVisible();
      // Another tab is what the entry keeps; the page may not have reported its stage yet, so the
      // list leaves for it by its address.
      const discussion = page.getByRole('tab', { name: /^Discussion/ });
      await discussion.click();
      await page.reload();
      await expect(discussion).toHaveAttribute('aria-selected', 'true');
      await page.goto(`${door!.origin}/colab/pages/${id}`);
      await expect.poll(() => here(page)).toBe(`/colab/pages/${id}`);
      await page.goBack();
      await expect(discussion).toHaveAttribute('aria-selected', 'true');
      await page.goForward();
      await expect.poll(() => here(page)).toBe(`/colab/pages/${id}`);
      await expect(page.locator('.colab-route')).toBeVisible();
    });

    test('a Recent card on Home opens its page in the same document', async ({ page }) => {
      const id = await door!.createPage('Recent page', '<h1>Recent content</h1>');
      await door!.pair(page);
      const card = page.locator('#home-recent').getByRole('link', { name: 'Recent page' });
      // Colab records titles in the background after a page-list read; Home lists them then.
      await page.goto(`${door!.origin}/colab/`);
      await showAllPages(page);
      await expect(page.getByText('Recent page').first()).toBeVisible();
      await expect(async () => {
        await page.goto(`${door!.origin}/`);
        await expect(card).toBeVisible({ timeout: 1000 });
      }).toPass({ timeout: 20000 });
      const rail = page.getByRole('navigation', { name: 'TMT', exact: true });
      // Module links stay stable; the active module adds its own nested pages.
      const moduleLinks = rail.locator(':scope > .tmt-ui-rail-items > li > a');
      const items = await moduleLinks.allTextContents();
      await page.evaluate(() => ((window as unknown as { __same: boolean }).__same = true));
      await card.click();
      await expect.poll(() => here(page)).toBe(`/colab/pages/${id}`);
      await expect(page.locator('.colab-route')).toBeVisible();
      expect(await page.evaluate(() => (window as unknown as { __same?: boolean }).__same)).toBe(
        true,
      );
      expect(await moduleLinks.allTextContents()).toEqual(items);
      await expect(rail.getByRole('link', { name: /^All pages(?: · .+)?$/ })).toBeVisible();
    });

    test('moving between routes never runs out of this device’s sessions', async ({ page }) => {
      const limit = spawnSync(remoteBinary, ['settings', 'sessions-per-device', '2'], {
        env: door!.env,
        cwd: door!.env.HOME,
      });
      expect(limit.status, String(limit.stderr)).toBe(0);
      await door!.createPage('Session page', '<h1>Session content</h1>');
      await door!.pair(page);
      await page.goto(`${door!.origin}/remote/my-devices`);
      // In-document moves, as the rail and in-page links make them.
      const go = (path: string) =>
        page.evaluate((path) => {
          history.pushState(null, '', path);
          dispatchEvent(new PopStateEvent('popstate'));
        }, path);
      const thisDevice = page.locator('.device-summary').filter({ hasText: 'This device' });
      const live = async () =>
        Number(/(\d+) live session/.exec((await thisDevice.textContent()) ?? '')?.[1]);
      for (let round = 0; round < 5; round++) {
        await expect(thisDevice, `round ${round}`).toBeVisible();
        await expect(page.locator('#access')).not.toContainText('connection limit');
        expect(await live(), `round ${round}`).toBeLessThanOrEqual(2);
        await go('/ops');
        await expect(page.locator('.tmt-ui-shell-main[data-route="ops"]')).toBeVisible();
        await go('/remote/my-devices');
        await expect(thisDevice).toBeVisible();
        await expect(page.locator('#access')).not.toContainText('connection limit');
        await go('/colab/');
        await expect(page.locator('.colab-route')).toBeVisible();
        await expect(page.getByRole('tab', { name: /^Discussion/ })).toBeVisible();
        await go('/remote/my-devices');
      }
      await expect(thisDevice).toBeVisible();
      expect(await live()).toBeLessThanOrEqual(2);
    });

    test('a paired Colab closes its Remote session and page bindings on unmount', async ({
      page,
    }) => {
      await page.addInitScript(() => {
        const opened: WebSocket[] = [];
        const Native = window.WebSocket;
        window.WebSocket = class extends Native {
          constructor(...args: ConstructorParameters<typeof WebSocket>) {
            super(...args);
            opened.push(this);
          }
        };
        (window as unknown as { __opened: WebSocket[] }).__opened = opened;
      });
      const id = await door!.createPage('Unmount page', '<h1>Unmount content</h1>');
      await door!.pair(page);
      await page.goto(`${door!.origin}/colab/pages/${id}`);
      const live = () =>
        page.evaluate(
          () =>
            (window as unknown as { __opened: WebSocket[] }).__opened.filter(
              (socket) => socket.readyState < WebSocket.CLOSING,
            ).length,
        );
      // An open page holds its binding; leaving the route through the TMT rail must close it.
      await expect.poll(live).toBeGreaterThan(0);
      const styles = page.locator('link[href$="/colab-module.css"]');
      await expect(styles).toHaveCount(1);
      await page
        .getByRole('navigation', { name: 'TMT', exact: true })
        .getByRole('link', { name: 'Home' })
        .click();
      await expect.poll(live).toBe(0);
      // Colab's stylesheet goes with its route.
      await expect(styles).toHaveCount(0);
    });
  });
});

test('unmounting Colab closes what it started', async ({ page }) => {
  await page.goto('/');
  await serveRendererLikeTheDoor(page);
  await page.evaluate(async (fixture) => (await import(fixture)).trackStarted(), fixture);
  await page.evaluate(async ({ fixture, url }) => (await import(fixture)).mountModule(url), {
    fixture,
    url: `/colab/pages/${page1}`,
  });
  // The local preview has no pages of its own: the route is the Colab home inside the slot.
  await expect(page.locator('#spa-slot .colab-route > *').first()).toBeVisible();
  await page.evaluate(async (fixture) => (await import(fixture)).unmountModule(), fixture);
  // The route is gone, and nothing it opened is still running.
  await expect(page.locator('#spa-slot .colab-route > *')).toHaveCount(0);
  await expect
    .poll(() => page.evaluate(async (fixture) => (await import(fixture)).leaks(), fixture))
    .toMatchObject({
      openSockets: 0,
      intervals: 0,
      workers: 0,
      channels: 0,
      mountedChildren: 0,
    });
});
