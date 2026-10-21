import { expect, test, type BrowserContext, type Page, type Route } from '@playwright/test';
import { text } from '../src/strings.js';

const mount = '/r/abcd/x/colab/';
const OLD = '/assets/index-OLDOLD12.js';
const NEW = '/assets/index-NEWNEW34.js';
const GUIDANCE = '/assets/recovery.js';
type Fulfill = (route: Route, options: Parameters<Route['fulfill']>[0]) => Promise<void>;

/** The mounted page as a built app serves it: a hashed entry module. The tab loads the
 * old build; what the server serves on later reads is `served`. Registration cannot
 * start (no Remote SDK), which is the failure path that asks whether the tab is stale. */
async function serve(
  context: BrowserContext,
  served: () => string,
  fulfill: Fulfill = (route, options) => route.fulfill(options),
) {
  const documents: boolean[] = [];
  await context.route(`**${mount}`, async (route) => {
    const navigation = route.request().isNavigationRequest();
    const response = await route.fetch();
    const html = await response.text();
    await fulfill(route, {
      response,
      body: html.replace(
        /\/src\/main\.tsx(?:\?[^"']*)?/,
        `${navigation && !loaded.value ? OLD : served()}?fixture=stale-tab`,
      ),
    });
    if (navigation) loaded.value = true;
    documents.push(navigation);
  });
  const loaded = { value: false };
  for (const entry of [OLD, NEW, GUIDANCE])
    // Vite may retain an HMR timestamp on the rewritten entry URL.
    await context.route(
      (url) => url.pathname === entry,
      (route) => route.fulfill({ contentType: 'text/javascript', body: "import '/src/main.tsx';" }),
    );
  await context.route('**/sdk/remote-v1.js', (route) => route.fulfill({ status: 404 }));
  return {
    navigations: () => documents.filter(Boolean).length,
    reads: () => documents.filter((n) => !n).length,
  };
}

const notice = (page: Page) => page.locator('[data-update-notice]');

test('a document read is ready only after its held response is fulfilled', async ({
  page,
  context,
}) => {
  let release!: () => void;
  const held = new Promise<void>((resolve) => (release = resolve));
  let entered!: () => void;
  const pending = new Promise<void>((resolve) => (entered = resolve));
  let delivery = Promise.resolve();
  const server = await serve(
    context,
    () => OLD,
    (route, options) => {
      if (route.request().isNavigationRequest()) return route.fulfill(options);
      delivery = held.then(() => route.fulfill(options));
      entered();
      return delivery;
    },
  );
  // Exercise the fixture directly, independently of application startup or build checks.
  await context.route(
    (url) => url.pathname === OLD,
    (route) => route.fulfill({ contentType: 'text/javascript', body: '' }),
  );
  let read: Promise<string> | undefined;
  try {
    await page.goto(mount);
    read = page.evaluate((url) => fetch(url).then((response) => response.text()), mount);
    await pending;
    expect(server.navigations()).toBe(1);
    expect(server.reads()).toBe(0);
    release();
    await expect.poll(() => server.reads()).toBe(1);
    expect(await read).toContain(OLD);
  } finally {
    // Drain the actual delivery even when the readiness assertion fails.
    release();
    await delivery;
    await read;
  }
});

test('a tab whose build was replaced says so, keeps its failure, and never reloads by itself', async ({
  page,
  context,
}, testInfo) => {
  const server = await serve(context, () => NEW);
  await page.goto(mount);
  await expect(notice(page)).toHaveText(new RegExp(`${text.updated}\\s*${text.reload}`));
  await expect(notice(page)).toHaveAttribute('role', 'status');
  // The failure it explains is still shown, and nothing navigated on its own.
  await expect(page.getByRole('heading', { name: text.error, level: 2 })).toBeVisible();
  await page.waitForTimeout(500);
  expect(server.navigations()).toBe(1);
  expect(server.reads()).toBe(1);
  for (const theme of ['light', 'dark']) {
    await page.evaluate((value) => (document.documentElement.dataset.theme = value), theme);
    for (const width of [1440, 390]) {
      await page.setViewportSize({ width, height: 844 });
      await page.screenshot({ path: testInfo.outputPath(`stale-tab-${theme}-${width}.png`) });
    }
  }
  // Reload is the reader's action: the new build loads and the notice is gone.
  await notice(page).getByRole('button', { name: text.reload }).click();
  await expect.poll(() => server.navigations()).toBe(2);
  await expect(page.getByRole('heading', { name: text.error, level: 2 })).toBeVisible();
  await expect(notice(page)).toHaveCount(0);
});

for (const [name, served] of [
  ['the same build', OLD],
  ['pairing guidance for an unpaired browser', GUIDANCE],
] as const)
  test(`no update notice when the server serves ${name}`, async ({ page, context }) => {
    const server = await serve(context, () => served);
    await page.goto(mount);
    await expect(page.getByRole('heading', { name: text.error, level: 2 })).toBeVisible();
    await expect.poll(() => server.reads()).toBe(1);
    await expect(notice(page)).toHaveCount(0);
  });

test('a serve older than the installed release says to restart it, with no Reload and no navigation', async ({
  page,
  context,
}, testInfo) => {
  const server = await serve(context, () => OLD);
  let asked = 0;
  await context.route('**/api/serve-release', (route) => {
    asked++;
    return route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({ running: '0.1.0-alpha.46', installed: '0.1.0-alpha.57' }),
    });
  });
  await page.goto(mount);
  await expect(notice(page)).toHaveText(
    'Colab 0.1.0-alpha.57 is installed, but 0.1.0-alpha.46 is still running. Restart tmt colab serve to update.',
  );
  await expect(notice(page).locator('code')).toHaveText(text.serveOlder.command);
  await expect(notice(page)).toHaveAttribute('role', 'status');
  await expect(notice(page).getByRole('button')).toHaveCount(0);
  await expect(page.getByRole('heading', { name: text.error, level: 2 })).toBeVisible();
  await page.waitForTimeout(500);
  expect(server.navigations()).toBe(1);
  expect(asked).toBe(1);
  for (const theme of ['light', 'dark']) {
    await page.evaluate((value) => (document.documentElement.dataset.theme = value), theme);
    for (const width of [1440, 390]) {
      await page.setViewportSize({ width, height: 844 });
      await expect(notice(page)).toBeVisible();
      await page.screenshot({ path: testInfo.outputPath(`serve-older-${theme}-${width}.png`) });
    }
  }
});

for (const [name, reply] of [
  ['the same release', { status: 200, body: { running: '0.1.0-alpha.57' } }],
  ['an older serve without the route', { status: 404, body: {} }],
  [
    'an unknown field',
    { status: 200, body: { running: '0.1.0-alpha.46', installed: '0.1.0-alpha.57', extra: 1 } },
  ],
  ['a malformed version', { status: 200, body: { running: 'x y', installed: '1' } }],
] as const)
  test(`no serve-older notice for ${name}`, async ({ page, context }) => {
    const server = await serve(context, () => OLD);
    await context.route('**/api/serve-release', (route) =>
      route.fulfill({
        status: reply.status,
        contentType: 'application/json',
        body: JSON.stringify(reply.body),
      }),
    );
    await page.goto(mount);
    await expect(page.getByRole('heading', { name: text.error, level: 2 })).toBeVisible();
    await expect.poll(() => server.reads()).toBe(1);
    await page.waitForTimeout(300);
    await expect(notice(page)).toHaveCount(0);
  });
