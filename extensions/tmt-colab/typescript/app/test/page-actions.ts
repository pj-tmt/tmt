import { expect, type Locator, type Page } from '@playwright/test';
import { commentsRegion } from './comments-region.js';

/** The dock's Comments action: its name carries the open-thread count and, while a thread has news,
 * the word that says so (`Comments (2) · New`). */
export const COMMENTS_NAME = /^Comments(?: \(\d+\))?(?: · .*)?$/;
export const CHAT_NAME = /^Chat(?: · Unread)?$/;
export function commentsAction(host: Page | Locator) {
  return host.locator('.page-floating-actions').getByRole('button', { name: COMMENTS_NAME });
}
export function chatAction(host: Page | Locator) {
  return host.locator('.page-floating-actions').getByRole('button', { name: CHAT_NAME });
}

/** Start the browser fixture with the current All filter selected. */
export async function showResolvedOn(page: Page) {
  await page.addInitScript(() => localStorage.setItem('colab.comments.filter', 'all'));
}

/** The visible bar's buttons: the shared top bar where Colab is the shell's root, the module header
 * where a host's shell mounts Colab (the hosted mount keeps its own header). */
const BAR_BUTTONS = '.tmt-ui-topbar:visible button:visible, .tmt-ui-header:visible button:visible';

/** Resolve page actions: Comments and Chat are in the dock; page tools are in More. */
export async function pageAction(host: Page | Locator, name: string): Promise<Locator> {
  const directName =
    name === 'Files' ? /^Files(?: \(\d+\)| \d+)?$/ : name === 'Agents' ? 'Agent status' : name;
  const page = 'page' in host ? host.page() : host;
  const header = host
    .locator('.page-header-actions:visible, .tmt-ui-header:visible:has(+ main .reader-status-row)')
    .first();
  // Every action starts from nothing open: a modal drawer is dismissed from its covered header, and
  // the non-modal Comments panel (which leaves the header live) through its own Close.
  const drawer = page.locator('.tmt-ui-drawer[open]');
  const comments = commentsRegion(page);
  const chat = page.locator('.tmt-ui-panel[data-panel=chat]:not([hidden])');
  const dismissDrawer = async () => {
    if (await comments.isVisible()) {
      await comments.getByRole('button', { name: 'Close (Esc)' }).click();
      await comments.waitFor({ state: 'hidden' });
      return;
    }
    if (await chat.isVisible()) {
      await chat.getByRole('button', { name: 'Close Chat' }).click();
      await chat.waitFor({ state: 'hidden' });
      return;
    }
    // The first covered-header press dismisses only; the action needs a second press.
    const button = await page.locator(BAR_BUTTONS).first().boundingBox();
    if (!button) throw new Error('Open drawer has no visible header return target');
    await page.mouse.click(button.x + button.width / 2, button.y + button.height / 2);
    await drawer.waitFor({ state: 'hidden' });
  };
  const anythingOpen = async () =>
    (await drawer.isVisible()) || (await comments.isVisible()) || (await chat.isVisible());
  if (await anythingOpen()) await dismissDrawer();
  if (name.startsWith('Theme:')) {
    // Theme is the shared top bar's menu, not a page action.
    await page
      .locator('.tmt-ui-topbar-common:visible')
      .getByRole('button', { name: /^Theme:/ })
      .click();
    return page.getByRole('menuitemradio', { name: name.slice('Theme: '.length), exact: true });
  }
  if (name === 'Chat' || name === 'Comments') {
    const action = name === 'Chat' ? chatAction(host) : commentsAction(host);
    await action.waitFor();
    return action;
  }
  // A mounted opening header can precede the page header after reload.
  await header.waitFor();
  const menu = 'More';
  // A shared modal menu aria-hides the background, including its own trigger.
  // The header is still physically visible; inspect its disclosure state without
  // forcing activation or treating the hidden background as an actionable target.
  const trigger = header.getByRole('button', {
    name: menu,
    exact: typeof menu === 'string',
    includeHidden: true,
  });
  if ((await trigger.getAttribute('aria-expanded')) !== 'true') {
    const previous = page
      .locator('.tmt-ui-menu-surface[open], [data-slot="dropdown-menu-content"]:visible')
      .first();
    if (await previous.isVisible()) {
      const button = await trigger.boundingBox();
      if (!button) throw new Error('Menu switch has no visible trigger');
      await page.mouse.click(button.x + button.width / 2, button.y + button.height / 2);
      await previous.waitFor({ state: 'hidden' });
    }
    try {
      await trigger.click({ timeout: 5_000 });
    } catch (error) {
      // A deep link such as `#t=` opens its drawer once the page has loaded, which can be after the
      // check above; dismiss that drawer and press again.
      if (!(await anythingOpen())) throw error;
      await dismissDrawer();
      await trigger.click();
    }
  }
  return page.getByRole('menuitem', {
    name: name === 'Share…' ? /^Share…/ : directName,
    exact: name !== 'Share…',
  });
}

/** Open a shared reader's More menu and select a page action. */
export async function readerAction(page: Page, name: string) {
  await page.locator('.tmt-ui-header').getByRole('button', { name: 'More', exact: true }).click();
  return page.getByRole('menuitem', { name, exact: true });
}

/** Select through the shipped filter and arrows; no synthetic row, state injection or hidden UI. */
export async function chooseThreadFilter(page: Page, name: 'Open' | 'Needs you' | 'All') {
  const panel = commentsRegion(page);
  await panel.getByRole('button', { name: 'Thread filter', exact: true }).click();
  await page.getByRole('menuitemradio', { name, exact: true }).click();
}
async function walkDiscussion(page: Page, matches: () => Promise<boolean>) {
  await chooseThreadFilter(page, 'All');
  const panel = commentsRegion(page);
  const position = panel.locator('.discussion-position');
  await expect(position).toBeVisible();
  const count = Number((await position.innerText()).split('/')[1].trim());
  if (!Number.isSafeInteger(count) || count < 1) throw new Error('Missing thread count');
  const previous = panel.getByRole('button', { name: 'Previous', exact: true });
  for (let index = 0; index < count && !(await previous.isDisabled()); index++)
    await previous.click();
  for (let index = 0; index < count; index++) {
    if (await matches()) return;
    const next = panel.getByRole('button', { name: 'Next', exact: true });
    if (await next.isDisabled()) break;
    await next.click();
  }
  throw new Error('Thread was not reachable through the discussion arrows');
}
export async function selectThread(page: Page, id: string | null) {
  if (!id) throw new Error('Missing thread ID');
  await walkDiscussion(
    page,
    async () =>
      (await commentsRegion(page).locator('.comments-panel').getAttribute('data-thread-id')) === id,
  );
}
export async function selectDiscussion(page: Page, content: string | RegExp) {
  await walkDiscussion(page, async () => {
    const body = await commentsRegion(page).locator('.comments-panel').innerText();
    return typeof content === 'string' ? body.includes(content) : content.test(body);
  });
}
