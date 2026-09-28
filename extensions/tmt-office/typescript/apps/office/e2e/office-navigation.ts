import { expect, type Page } from '@playwright/test';

export async function openOfficeMenu(page: Page): Promise<void> {
  const menu = page.getByRole('button', { name: 'Office menu', exact: true });
  if ((await menu.getAttribute('aria-expanded')) !== 'true') await menu.click();
}

export async function openMeetingRooms(page: Page): Promise<void> {
  await openOfficeMenu(page);
  await page.getByRole('button', { name: 'Meeting rooms', exact: true }).click();
}

export async function openKeyboardSelection(page: Page): Promise<void> {
  // An initializing page has neither picker nor Clear selection. Wait for the
  // mounted world before deciding whether the current inspector must be closed.
  await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
  const agentClose = page.getByRole('button', { name: 'Close agent conversation', exact: true });
  if (await agentClose.isVisible()) await agentClose.click();
  const summary = page.getByText('Keyboard selection', { exact: true });
  const details = summary.locator('..');
  if (!(await summary.isVisible()))
    await page.getByRole('button', { name: 'Clear selection', exact: true }).click();
  if ((await details.getAttribute('open')) === null) await summary.click();
}

export async function beginMeetingCreation(page: Page): Promise<void> {
  await openOfficeDirectory(page);
  const spaces = page.getByText('Available spaces', { exact: true });
  const details = spaces.locator('..');
  if ((await details.getAttribute('open')) === null) await spaces.click();
  await details.getByRole('button', { name: 'New meeting room', exact: true }).click();
}

/** Open the accessible directory without toggling an already-open panel closed. */
export async function openOfficeDirectory(page: Page): Promise<void> {
  if (await page.getByRole('complementary', { name: 'Office directory' }).isVisible()) return;
  await openOfficeMenu(page);
  const toggle = page.getByRole('button', { name: /^Directory · / });
  await toggle.waitFor({ state: 'visible' });
  if ((await toggle.getAttribute('aria-expanded')) !== 'true') await toggle.click();
}

/** Select an identity in the world HUD, including saved agents without a personal area. */
export async function openAgentDetails(page: Page, name: string): Promise<void> {
  await openOfficeDirectory(page);
  const escaped = name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  await page
    .getByRole('button', { name: new RegExp(`^${escaped} · (Online|Offline|Presence unknown)`) })
    .click();
}

/** Expose accessible object actions without changing the camera or activating one. */
export async function openOfficeObjects(page: Page): Promise<void> {
  await openOfficeDirectory(page);
  await page.getByRole('button', { name: 'Lobby · lobby', exact: true }).click();
}
