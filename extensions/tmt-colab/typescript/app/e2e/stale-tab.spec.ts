import { expect, test, type BrowserContext, type Page } from '@playwright/test';
import { text } from '../src/strings.js';

const mount = '/r/abcd/x/colab/';
const OLD = '/assets/index-OLDOLD12.js';
const NEW = '/assets/index-NEWNEW34.js';
const GUIDANCE = '/assets/recovery.js';

/** The mounted page as a built app serves it: a hashed entry module. The tab loads the
 * old build; what the server serves on later reads is `served`. Registration cannot
 * start (no Remote SDK), which is the failure path that asks whether the tab is stale. */
async function serve(context: BrowserContext, served: () => string) {
  const documents: boolean[] = [];
  await context.route(`**${mount}`, async (route) => {
    const navigation = route.request().isNavigationRequest();
    documents.push(navigation);
    const response = await route.fetch();
    const html = await response.text();
    await route.fulfill({
      response,
      body: html.replace('/src/main.tsx', navigation && !loaded.value ? OLD : served()),
    });
    if (navigation) loaded.value = true;
  });
  const loaded = { value: false };
  for (const entry of [OLD, NEW, GUIDANCE])
    await context.route(`**${entry}`, (route) =>
      route.fulfill({ contentType: 'text/javascript', body: "import '/src/main.tsx';" }),
    );
  await context.route('**/sdk/remote-v1.js', (route) => route.fulfill({ status: 404 }));
  return {
    navigations: () => documents.filter(Boolean).length,
    reads: () => documents.filter((n) => !n).length,
  };
}

const notice = (page: Page) => page.locator('[data-update-notice]');

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
  await expect(notice(page)).toHaveText(text.serveOlder('0.1.0-alpha.46', '0.1.0-alpha.57'));
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
