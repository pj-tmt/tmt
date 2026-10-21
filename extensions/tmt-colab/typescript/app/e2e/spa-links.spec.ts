import { expect, test, type Page } from '@playwright/test';
import { readFileSync } from 'node:fs';

/**
 * Conditions for Colab as a route of the one TMT web app at `/colab/...` (#3937; feat-lead's and
 * tmt-lead's conditions): every link form printed so far lands on `/colab/...` with its search
 * keys, a reload of a page route works, the app document's policy is no looser than today's,
 * the renderer stays a separate sandboxed opaque frame, and unmounting releases what Colab started.
 *
 * The router and module tests run over the local transports (test/spa-links-browser.tsx). The
 * door tests need the real door serving the TMT app; until ux-4's door and build work lands they
 * run as expected failures against the dev server. To flip them, point TMT_SPA_DOOR_ORIGIN at a
 * running door that serves the built app: they then run normally and must pass.
 */
const fixture = '/test/spa-links-browser.tsx';
const space = '10000000-0000-4000-8000-000000000001';
const page1 = '12345678-0000-4000-8000-000000000001';
const reading = '10000000-0000-4000-8000-000000000401';
const doorOrigin = process.env.TMT_SPA_DOOR_ORIGIN;
const needsDoor = (title: string) =>
  `TODO(ux-4 door/build): ${title}. Set TMT_SPA_DOOR_ORIGIN to a door serving the built TMT app and remove this marker.`;

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

/** The door answers `/colab/renderer.html` with the renderer document and its policy; the dev
 * server serves the same file at `/renderer.html`. */
async function serveRendererLikeTheDoor(page: Page) {
  const origin = new URL(page.url()).origin;
  await page.route('**/colab/renderer.html', async (route) =>
    route.fulfill({ response: await route.fetch({ url: `${origin}/renderer.html` }) }),
  );
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
  const inside = page
    .frames()
    .find((candidate) => candidate.url().endsWith('/colab/renderer.html'))!;
  expect(await inside.evaluate(() => window.origin)).toBe('null');
  expect(await frame.evaluate((node) => (node as HTMLIFrameElement).contentDocument)).toBe(null);
  expect(
    await page.evaluate(() => (document.querySelector('iframe') as HTMLIFrameElement).srcdoc),
  ).toBe('');
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
  const origin = (baseURL: string | undefined) => doorOrigin ?? baseURL!;
  test.beforeEach(() => {
    test.fail(!doorOrigin, needsDoor('the door serves /colab/... as a route of the TMT app'));
  });

  test('the app document of a short link has a policy no looser than today’s app document', async ({
    baseURL,
    request,
  }) => {
    const response = await request.get(`${origin(baseURL)}/colab/p/12345678`);
    expect(response.ok()).toBe(true);
    const policy = response.headers()['content-security-policy'];
    expect(policy, 'the document carries a Content-Security-Policy').toBeTruthy();
    expect(atLeastAsStrict(policy!, APP_POLICY)).toEqual([]);
  });

  test('the renderer document keeps the renderer policy and is not the app document', async ({
    baseURL,
    request,
  }) => {
    const response = await request.get(`${origin(baseURL)}/colab/renderer.html`);
    expect(response.ok()).toBe(true);
    expect(response.headers()['content-security-policy']).toBe(RENDERER_POLICY);
    expect(await response.text()).toContain('Colab page renderer');
  });

  test('a reload of a page route opens the same page in the TMT app', async ({ page, baseURL }) => {
    await page.goto(`${origin(baseURL)}/colab/pages/${page1}`);
    await expect(page.getByRole('navigation', { name: 'TMT', exact: true })).toBeVisible();
    await page.reload();
    await expect(page.getByRole('navigation', { name: 'TMT', exact: true })).toBeVisible();
    await expect(page.locator('.colab-route')).toBeVisible();
    expect(new URL(page.url()).pathname).toBe(`/colab/pages/${page1}`);
  });

  test('the moved mount and the old short entry redirect into /colab/ and keep their query', async ({
    baseURL,
    request,
  }) => {
    const base = origin(baseURL);
    const moved = await request.get(`${base}/r/abcdefghijklmnop/x/colab/?read=${reading}`, {
      maxRedirects: 0,
    });
    expect(moved.status()).toBe(308);
    expect(moved.headers().location).toBe(`/colab/?read=${reading}`);
    const short = await request.get(`${base}/p/12345678`, { maxRedirects: 0 });
    expect([301, 302, 307, 308]).toContain(short.status());
    expect(short.headers().location).toBe('/colab/p/12345678');
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

test('a paired Colab closes its Remote session and page bindings on unmount', async ({ page }) => {
  test.fail(!doorOrigin, needsDoor('a paired Colab needs the door’s mount API and a real session'));
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
  await page.goto(`${doorOrigin ?? ''}/colab/pages/${page1}`);
  const live = () =>
    page.evaluate(
      () =>
        (window as unknown as { __opened: WebSocket[] }).__opened.filter(
          (socket) => socket.readyState < WebSocket.CLOSING,
        ).length,
    );
  // Open page: Colab holds its session. Leaving the route through the TMT rail must close it.
  await expect.poll(live).toBeGreaterThan(0);
  await page
    .getByRole('navigation', { name: 'TMT', exact: true })
    .getByRole('link', { name: 'Home' })
    .click();
  await expect.poll(live).toBe(0);
});
