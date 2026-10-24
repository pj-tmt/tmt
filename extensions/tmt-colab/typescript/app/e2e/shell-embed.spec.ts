import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';
import { text } from '../src/strings.js';

/**
 * A chapter's panel can host the page itself (#4192 slice 5): "Decide here" swaps the passage list
 * for the real page, which owns one live binding, closed when the panel leaves it. The page's own
 * drawers and menus stay at viewport level (modal, top layer), as they are on the page route.
 */
/** The look a capture run uses, `COLAB_LOOK=1440:dark`; the assertions hold in every look. */
const look = process.env.COLAB_LOOK ?? '1440:light';
const capture = async (page: Page, name: string) => {
  if (process.env.COLAB_CAPTURE_DIR) {
    // At rest: the pointer of the last click would otherwise leave its hover fill in the capture.
    await page.mouse.move(300, 600);
    await page.screenshot({
      path: capturePath(`shell-embed-${name}-${look.replace(':', '-')}.png`),
    });
  }
};

const PLAN = '10000000-0000-4000-8000-0000000000a1';
const t = text.shellPage;

async function openShell(page: Page, kind = 'named') {
  await page.emulateMedia({ colorScheme: look.endsWith('dark') ? 'dark' : 'light' });
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
  page.locator(`#shell-aside[data-page-id="${PLAN}"] button[data-opens-panel]`);
const bindings = (page: Page) =>
  page.evaluate(() => ({
    opened: window.bindings.opened.length,
    closed: window.bindings.closed.length,
    mostAtOnce: window.bindings.mostAtOnce,
  }));

async function decideHere(page: Page) {
  const [width, scheme] = look.split(':');
  await page.setViewportSize({ width: Number(width), height: 900 });
  await page.emulateMedia({ colorScheme: scheme === 'dark' ? 'dark' : 'light' });
  await openShell(page);
  await trigger(page).click();
  await panel(page).getByRole('button', { name: t.decideHere }).click();
}

test('Decide here hosts the page in the panel without touching the tab title', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await openShell(page);
  const title = await page.title();
  await capture(page, 'dashboard');
  // The selected page's first action lines up with its title, not indented from it.
  const side = page.locator('#shell-aside');
  const titleLeft = (await side.locator('.shell-aside-title').boundingBox())!.x;
  const openLeft = (await side.locator('.shell-open').first().boundingBox())!.x;
  expect(Math.abs(openLeft - titleLeft)).toBeLessThanOrEqual(1);
  await trigger(page).click();
  const decide = (await panel(page).getByRole('button', { name: t.decideHere }).boundingBox())!;
  const left = (await panel(page).locator('.tmt-ui-drawer-heading').first().boundingBox())!.x;
  expect(Math.abs(decide.x - left)).toBeLessThanOrEqual(1);
  await capture(page, 'summary');
  await panel(page).getByRole('button', { name: t.decideHere }).click();
  // The page's own document and header actions are in the panel; its brand and title are not (the
  // drawer already names the page), and the shell's own header is still the one on the page.
  await expect(panel(page).locator('section.page')).toBeVisible();
  await expect(panel(page).locator('iframe')).toBeVisible();
  await expect(panel(page).getByRole('navigation', { name: text.pageHeaderActions })).toBeVisible();
  await expect(panel(page).locator('.tmt-ui-brand')).toHaveCount(0);
  expect(await page.title()).toBe(title);
  await expect(page).toHaveURL(new RegExp(`\\?open=${PLAN}`));
  await expect(page).not.toHaveURL(/#t=/);
  // A viewport-fixed control of the page stays inside the panel, never over the shell beside it, and
  // the panel's actions line up with its content edge.
  const frame = (await panel(page).boundingBox())!;
  const floating = page.locator('.page-floating-actions');
  for (let index = 0; index < (await floating.count()); index += 1) {
    const box = await floating.nth(index).boundingBox();
    if (box) {
      expect(box.x).toBeGreaterThanOrEqual(frame.x);
      expect(box.x + box.width).toBeLessThanOrEqual(frame.x + frame.width);
    }
  }
  const edge = (await panel(page).locator('.tmt-ui-drawer-heading').first().boundingBox())!.x;
  const summary = (await panel(page).getByRole('button', { name: t.backToSummary }).boundingBox())!;
  expect(Math.abs(summary.x - edge)).toBeLessThanOrEqual(1);
  await capture(page, 'page');
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
  const more = panel(page).getByRole('button', { name: text.more, exact: true });
  await more.click();
  await page.getByRole('menuitem', { name: text.agentStatus, exact: true }).click();
  const drawer = page.getByRole('dialog', { name: text.agentStatus });
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
  await capture(page, 'drawer');
  await page.keyboard.press('Escape');
  await expect(drawer).toBeHidden();
  await expect(panel(page)).toBeVisible();
  await expect(page).toHaveURL(new RegExp(`\\?open=${PLAN}`));
  await expect(more).toBeFocused();
});

test('the More menu from the embedded bar anchors to its trigger and stays inside the viewport', async ({
  page,
}) => {
  await decideHere(page);
  const more = panel(page).getByRole('button', { name: text.more });
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
  await capture(page, 'menu');
  await page.keyboard.press('Escape');
  await expect(menu).toBeHidden();
  await expect(panel(page)).toBeVisible();
  await expect(more).toBeFocused();
});

test('below 900 px the full page route keeps its own drawer and menu, which look the same', async ({
  page,
}) => {
  const [, scheme] = look.split(':');
  await page.setViewportSize({ width: 390, height: 844 });
  await page.emulateMedia({ colorScheme: scheme === 'dark' ? 'dark' : 'light' });
  await openShell(page);
  await page.evaluate((id) => (location.hash = `/pages/${id}`), PLAN);
  const actions = page.getByRole('navigation', { name: text.pageHeaderActions });
  await expect(actions).toBeVisible();
  await actions.getByRole('button', { name: text.more, exact: true }).click();
  await page.getByRole('menuitem', { name: text.agentStatus, exact: true }).click();
  const drawer = page.getByRole('dialog', { name: text.agentStatus });
  await expect(drawer).toBeVisible();
  await capture(page, 'narrow-drawer');
  await page.keyboard.press('Escape');
  await expect(drawer).toBeHidden();
  await actions.getByRole('button', { name: text.more }).click();
  await expect(page.getByRole('menu')).toBeVisible();
  await capture(page, 'narrow-menu');
});

test('with the page in the panel, @ opens the embedded page’s Chat and Esc closes only that', async ({
  page,
}) => {
  await decideHere(page);
  await expect(panel(page).locator('iframe')).toBeVisible();
  // Typing in the panel, not in a field: the shell page behind it and the embedded page both listen
  // on the document, and only the page in the panel may act. The shell page cannot: its own panel is
  // an open overlay to it, and a dashboard has no document to select from (so C does nothing there).
  await panel(page).getByRole('link', { name: t.openFullPage }).focus();
  await page.keyboard.type('@');
  const chat = page.getByRole('dialog', { name: text.chat });
  await expect(chat).toBeVisible();
  await expect(chat).toHaveCount(1);
  // The Chat that opened is the embedded page's: it sits inside the panel, not beside it.
  await expect(panel(page).getByRole('dialog', { name: text.chat })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(chat).toBeHidden();
  await expect(panel(page)).toBeVisible();
  await expect(page).toHaveURL(new RegExp(`\\?open=${PLAN}`));
});
