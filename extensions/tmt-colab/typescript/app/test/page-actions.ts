import type { Locator, Page } from '@playwright/test';
import { commentsRegion } from './comments-region.js';

/** Resolve page actions through their header menu; readers keep their direct Files control.
 * Chat is in Discussion; page tools, including Agent status, are in More. */
export async function pageAction(host: Page | Locator, name: string): Promise<Locator> {
  const directName =
    name === 'Files' ? /^Files(?: \(\d+\)| \d+)?$/ : name === 'Agents' ? 'Agent status' : name;
  const page = 'page' in host ? host.page() : host;
  const header = host.locator('.page-header-actions:visible');
  // Every action starts from nothing open: a modal drawer is dismissed from its covered header, and
  // the non-modal Comments panel (which leaves the header live) through its own Close.
  const drawer = page.locator('.tmt-ui-drawer[open]');
  const comments = commentsRegion(page);
  const dismissDrawer = async () => {
    if (await comments.isVisible()) {
      await comments.getByRole('button', { name: /^Close (Comments|Discussion)$/ }).click();
      await comments.waitFor({ state: 'hidden' });
      return;
    }
    // The first covered-header press dismisses only; the action needs a second press.
    const button = await page
      .locator('.tmt-ui-header:visible button:visible')
      .first()
      .boundingBox();
    if (!button) throw new Error('Open drawer has no visible header return target');
    await page.mouse.click(button.x + button.width / 2, button.y + button.height / 2);
    await drawer.waitFor({ state: 'hidden' });
  };
  const anythingOpen = async () => (await drawer.isVisible()) || (await comments.isVisible());
  if (await anythingOpen()) await dismissDrawer();
  if (name === 'Files') {
    const direct = host.getByRole('button', { name: directName, exact: true });
    await header.or(direct).first().waitFor();
    if (await direct.isVisible()) return direct;
  }
  // A mounted opening header can precede the page header after reload.
  await header.waitFor();
  const menu = name === 'Chat' || name === 'Comments' ? /^Discussion(?: [(·]|$)/ : 'More';
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
  return page.getByRole(name.startsWith('Theme:') ? 'menuitemradio' : 'menuitem', {
    name:
      name === 'Comments'
        ? /^Comments(?: \d+)?(?: ·.*)?$/
        : name === 'Chat'
          ? /^Chat(?: · Unread)?$/
          : name === 'Share…'
            ? /^Share…/
            : directName,
    exact: name !== 'Comments' && name !== 'Chat' && name !== 'Share…',
  });
}
