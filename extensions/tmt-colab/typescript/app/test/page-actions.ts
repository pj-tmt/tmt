import type { Locator, Page } from '@playwright/test';

/** Resolve page actions through their header menu; readers keep their direct Files control. */
export async function pageAction(host: Page | Locator, name: string): Promise<Locator> {
  if (name === 'Files')
    return host.getByRole('button', { name: /^Files(?: \(\d+\)| \d+)?$/, exact: true });
  const header = host.locator('.page-header-actions:visible');
  // A mounted opening header can precede the page header after reload.
  await header.waitFor();
  if (name === 'Agents') return header.getByRole('button', { name, exact: true });
  const menu =
    name === 'Chat' || name === 'Comments'
      ? /^Discussion \(/
      : name === 'Source' || name === 'Export page'
        ? 'Source and export'
        : 'More';
  const trigger = header.getByRole('button', { name: menu, exact: typeof menu === 'string' });
  if ((await trigger.getAttribute('aria-expanded')) !== 'true') await trigger.click();
  return header.getByRole(name.startsWith('Theme:') ? 'menuitemradio' : 'menuitem', {
    name: name === 'Comments' ? /^Comments \d+/ : name,
    exact: name !== 'Comments',
  });
}
