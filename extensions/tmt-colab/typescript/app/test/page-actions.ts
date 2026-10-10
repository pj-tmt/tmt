import type { Locator, Page } from '@playwright/test';

/** Resolve page actions through their header menu; readers keep their direct Files control. */
export async function pageAction(host: Page | Locator, name: string): Promise<Locator> {
  const directName = name === 'Files' ? /^Files(?: \(\d+\)| \d+)?$/ : name;
  const header = host.locator('.page-header-actions:visible');
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
  if ((await trigger.getAttribute('aria-expanded')) !== 'true') await trigger.click();
  return header.getByRole(name.startsWith('Theme:') ? 'menuitemradio' : 'menuitem', {
    name: name === 'Comments' ? /^Comments \d+/ : directName,
    exact: name !== 'Comments',
  });
}
