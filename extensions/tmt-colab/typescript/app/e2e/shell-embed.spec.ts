import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';
import { text } from '../src/strings.js';

/**
 * A chapter's panel can host the page itself (#4192 slice 5): "Decide here" swaps the passage list
 * for the real page, which owns one live binding, closed when the panel leaves it. The page's own
 * drawers and menus stay at viewport level (modal, top layer), as they are on the page route.
 */
const PLAN = '10000000-0000-4000-8000-0000000000a1';
const t = text.shellPage;

async function openShell(page: Page, kind = 'named') {
  await page.route('**/src/main.tsx', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `import { mount } from "/test/shell-browser.tsx"; mount(${JSON.stringify(kind)});`,
    }),
  );
  await page.goto('/');
}
const panel = (page: Page) => page.getByRole('dialog', { name: 'Release plan' });
const trigger = (page: Page) =>
  page.locator(`.shell-chapter[data-page-id="${PLAN}"] button[data-opens-panel]`);
const bindings = (page: Page) =>
  page.evaluate(() => ({
    opened: window.bindings.opened.length,
    closed: window.bindings.closed.length,
    mostAtOnce: window.bindings.mostAtOnce,
  }));

async function decideHere(page: Page) {
  await page.setViewportSize({ width: 1440, height: 900 });
  await openShell(page);
  await trigger(page).click();
  await panel(page).getByRole('button', { name: t.decideHere }).click();
}

test('Decide here hosts the page in the panel without touching the tab title', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await openShell(page);
  const title = await page.title();
  await trigger(page).click();
  await panel(page).getByRole('button', { name: t.decideHere }).click();
  // The page's own document and header actions are in the panel; its brand and title are not (the
  // drawer already names the page), and the shell's own header is still the one on the page.
  await expect(panel(page).locator('section.page')).toBeVisible();
  await expect(panel(page).locator('iframe')).toBeVisible();
  await expect(panel(page).getByRole('button', { name: /Comments/ })).toBeVisible();
  await expect(panel(page).locator('.tmt-ui-brand')).toHaveCount(0);
  expect(await page.title()).toBe(title);
  await expect(page).toHaveURL(new RegExp(`\\?open=${PLAN}`));
  await expect(page).not.toHaveURL(/#t=/);
  // Back to the summary shows the passage list again and lets the page go.
  await panel(page).getByRole('button', { name: t.backToSummary }).click();
  await expect(panel(page).locator('.shell-panel-needs li').first()).toBeVisible();
  await expect(panel(page).locator('section.page')).toHaveCount(0);
});

test('the panel owns one live binding and gives it up before the full page opens its own', async ({
  page,
}) => {
  await decideHere(page);
  await expect(panel(page).locator('iframe')).toBeVisible();
  expect(await bindings(page)).toMatchObject({ opened: 1, closed: 0, mostAtOnce: 1 });
  await panel(page).getByRole('link', { name: t.openFullPage }).click();
  await expect(page).toHaveURL(new RegExp(`/pages/${PLAN}`));
  await expect(page.locator('section.page iframe')).toBeVisible();
  // The route opened its own binding only after the panel's was closed: never two at once.
  await expect.poll(async () => (await bindings(page)).closed).toBe(1);
  expect(await bindings(page)).toMatchObject({ opened: 2, mostAtOnce: 1 });
});

test('Esc, ← Summary and Back each let the panel’s binding go, and a reopened panel opens one', async ({
  page,
}) => {
  await decideHere(page);
  await expect(panel(page).locator('iframe')).toBeVisible();
  await panel(page).getByRole('button', { name: t.backToSummary }).click();
  await expect.poll(async () => (await bindings(page)).closed).toBe(1);
  await panel(page).getByRole('button', { name: t.decideHere }).click();
  await expect(panel(page).locator('iframe')).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(panel(page)).toHaveCount(0);
  await expect.poll(async () => (await bindings(page)).closed).toBe(2);
  expect(await bindings(page)).toMatchObject({ opened: 2, mostAtOnce: 1 });
});

test('a page drawer opened from the embedded page sits at the right edge, closes alone on Esc and returns focus', async ({
  page,
}) => {
  await decideHere(page);
  const comments = panel(page).getByRole('button', { name: /Comments/ });
  await comments.click();
  const drawer = page.getByRole('dialog', { name: 'Comments' });
  await expect(drawer).toBeVisible();
  const box = (await drawer.boundingBox())!;
  const viewport = page.viewportSize()!;
  // Its normal width and the viewport's right edge, not clipped to the panel.
  expect(box.width).toBeGreaterThanOrEqual(430);
  expect(Math.round(box.x + box.width)).toBeGreaterThan(viewport.width - 24);
  // The panel is inert behind the scrim: nothing in it takes a pointer.
  await expect(
    panel(page).getByRole('link', { name: t.openFullPage }).click({ trial: true, timeout: 1000 }),
  ).rejects.toThrow();
  await page.keyboard.press('Escape');
  await expect(drawer).toBeHidden();
  await expect(panel(page)).toBeVisible();
  await expect(page).toHaveURL(new RegExp(`\\?open=${PLAN}`));
  await expect(comments).toBeFocused();
  await page.screenshot({ path: capturePath('shell-embed-drawer-1440.png') });
});

test('the More menu from the embedded bar anchors to its trigger and stays inside the viewport', async ({
  page,
}) => {
  await decideHere(page);
  const more = panel(page).getByRole('button', { name: /More/ });
  await more.click();
  const menu = page.getByRole('menu');
  await expect(menu).toBeVisible();
  const trigger = (await more.boundingBox())!;
  const shown = (await menu.boundingBox())!;
  const viewport = page.viewportSize()!;
  expect(shown.x).toBeGreaterThanOrEqual(0);
  expect(shown.x + shown.width).toBeLessThanOrEqual(viewport.width);
  expect(shown.y + shown.height).toBeLessThanOrEqual(viewport.height);
  // Beside or under the trigger, never far from it.
  expect(Math.abs(shown.y - (trigger.y + trigger.height))).toBeLessThan(80);
  await page.screenshot({ path: capturePath('shell-embed-menu-1440.png') });
  await page.keyboard.press('Escape');
  await expect(menu).toBeHidden();
  await expect(panel(page)).toBeVisible();
  await expect(more).toBeFocused();
});
