import type { Locator, Page } from '@playwright/test';

/** Resolve page actions through their header menu; readers keep their direct Files control. */
export async function pageAction(host: Page | Locator, name: string): Promise<Locator> {
  const directName = name === 'Files' ? /^Files(?: \(\d+\)| \d+)?$/ : name;
  const page = 'page' in host ? host.page() : host;
  const header = host.locator('.page-header-actions:visible');
  const drawer = page.locator('.tmt-ui-drawer[open]');
  if (await drawer.isVisible()) {
    // The first covered-header press dismisses only; the action needs a second press.
    const button = await page
      .locator('.tmt-ui-header:visible button:visible')
      .first()
      .boundingBox();
    if (!button) throw new Error('Open drawer has no visible header return target');
    await page.mouse.click(button.x + button.width / 2, button.y + button.height / 2);
    await drawer.waitFor({ state: 'hidden' });
  }
  if (name === 'Files' || name === 'Agents') {
    const direct = host.getByRole('button', { name: directName, exact: true });
    await header.or(direct).first().waitFor();
    if (await direct.isVisible()) return direct;
  }
  // A mounted opening header can precede the page header after reload.
  await header.waitFor();
  const sourceMenu = header.getByRole('button', { name: 'Source and export', exact: true });
  const menu =
    name === 'Chat' || name === 'Comments'
      ? /^Discussion \(/
      : (name === 'Source' || name === 'Export page') && (await sourceMenu.isVisible())
        ? 'Source and export'
        : 'More';
  const trigger = header.getByRole('button', { name: menu, exact: typeof menu === 'string' });
  if ((await trigger.getAttribute('aria-expanded')) !== 'true') {
    const previous = page.locator('.tmt-ui-menu-surface[open]');
    if (await previous.isVisible()) {
      const button = await trigger.boundingBox();
      if (!button) throw new Error('Menu switch has no visible trigger');
      await page.mouse.click(button.x + button.width / 2, button.y + button.height / 2);
      await previous.waitFor({ state: 'hidden' });
    }
    await trigger.click();
  }
  return page.getByRole(name.startsWith('Theme:') ? 'menuitemradio' : 'menuitem', {
    name: name === 'Comments' ? /^Comments \d+/ : directName,
    exact: name !== 'Comments',
  });
}
