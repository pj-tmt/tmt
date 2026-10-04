import { expect, test, type Page } from '@playwright/test';
import { pairBrowser, startDoor } from './harness/browser.js';
import {
  createPage,
  freePort,
  openPage,
  previewAsk,
  run,
  selectInRenderer,
} from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

/** Every mutation below traverses paired Remote admission and the real Colab engine. */
async function manage(page: Page) {
  await page.getByRole('button', { name: 'Manage page', exact: true }).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog.getByLabel('Audience')).toBeVisible();
  return dialog;
}

test.afterEach(disposeActiveWorlds);
test('native sharing and lifecycle verification preserve Ask, page recovery and last-page uncertainty', async () => {
  const testInfo = test.info();
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const recipient = await world.startAgent('management-recipient');
    const device = await pairBrowser(world, 'management-browser');
    const first = createPage(world, 'Managed page', '<p id="quote">Management selection.</p>');
    const second = createPage(world, 'Remaining page', '<p>Still readable.</p>');
    const page = await openPage(door, device, first);
    await selectInRenderer(page, '#quote');
    await previewAsk(page, recipient.id, 'Do not dispatch this preview');
    let dialog = await manage(page);
    for (const theme of ['light', 'dark']) {
      await page.evaluate((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      await page.screenshot({
        path: testInfo.outputPath(`management-${theme}.png`),
        fullPage: true,
      });
    }
    await page.setViewportSize({ width: 390, height: 844 });
    await page.screenshot({ path: testInfo.outputPath('management-mobile.png'), fullPage: true });
    expect(await dialog.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
    await page.setViewportSize({ width: 1280, height: 900 });

    await dialog.getByLabel('Keep forever').check();
    await dialog.getByRole('button', { name: 'Set retention', exact: true }).click();
    await dialog.getByRole('button', { name: 'Confirm set retention', exact: true }).click();
    await expect(dialog.getByRole('status')).toContainText('Change verified');
    await expect(page.getByTestId('ask-preview')).toHaveCount(0);
    await dialog.getByRole('button', { name: 'Close', exact: true }).click();
    await expect(page.locator('iframe')).toBeVisible();
    expect(recipient.received()).toHaveLength(0);

    dialog = await manage(page);
    await dialog.getByLabel('Audience').selectOption('link');
    await dialog.getByRole('button', { name: 'Confirm make link' }).click();
    await expect(dialog.getByRole('status')).toContainText('Change verified');
    await dialog.getByRole('button', { name: 'Manage another change' }).click();
    await dialog.getByRole('button', { name: 'Create link', exact: true }).click();
    await expect(dialog.getByLabel('Link seed')).toHaveCount(0);
    await dialog.getByRole('button', { name: 'Confirm create link' }).click();
    await expect(dialog.getByLabel('Link seed')).toHaveValue(/^[A-Za-z0-9_-]{43}$/);
    const oldLink = await dialog.getByLabel('Link ID', { exact: true }).inputValue();
    await dialog.getByRole('button', { name: 'Manage another change' }).click();
    await dialog.getByRole('button', { name: 'Reset link', exact: true }).click();
    await dialog.getByRole('button', { name: 'Confirm reset link' }).click();
    await expect(dialog.getByRole('status')).toContainText('Change verified');
    expect(await dialog.getByLabel('Link ID', { exact: true }).inputValue()).not.toBe(oldLink);
    await dialog.getByRole('button', { name: 'Manage another change' }).click();
    await dialog.getByLabel('Audience').selectOption('private');
    await expect(dialog).toContainText('links are revoked and affected pages rotate');
    await dialog.getByRole('button', { name: 'Confirm make private' }).click();
    await expect(dialog.getByRole('status')).toContainText('Change verified');
    await dialog.getByRole('button', { name: 'Close', exact: true }).click();
    await expect(page.locator('iframe')).toBeVisible();
    await page.reload();
    await expect(page.getByTestId('ask-action')).toBeVisible();
    expect(recipient.received()).toHaveLength(0);
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      0,
    );

    dialog = await manage(page);
    await dialog.getByRole('button', { name: 'Archive page', exact: true }).click();
    await dialog.getByRole('button', { name: 'Confirm archive page' }).click();
    await expect(dialog.getByRole('status')).toContainText('Change verified');
    // Navigate explicitly: the archived page has no editing route.
    await dialog.getByRole('button', { name: 'Close', exact: true }).click();
    await page.goto(`${door.address}/x/colab/`);
    await page.getByLabel('Show archived pages').check();
    const row = page.locator('ul.pages li').filter({ hasText: first.pageId });
    await expect(row).toContainText('Archived');
    await row.getByRole('button', { name: 'Manage page' }).click();
    dialog = page.getByRole('dialog');
    await expect(dialog.getByRole('button', { name: 'Archive page', exact: true })).toBeDisabled();
    await dialog.getByRole('button', { name: 'Delete page', exact: true }).click();
    await expect(dialog).toContainText('Copies already made cannot be recalled');
    await dialog.getByRole('button', { name: 'Confirm delete page' }).click();
    await expect(dialog.getByRole('status')).toContainText('Deletion verified');
    await dialog.getByRole('button', { name: 'Close', exact: true }).click();
    await page.getByLabel('Show archived pages').uncheck();
    await page
      .locator('ul.pages li')
      .filter({ hasText: second.pageId })
      .getByRole('button', { name: 'Manage page' })
      .click();
    dialog = page.getByRole('dialog');
    await expect(dialog.getByRole('button', { name: 'Delete page', exact: true })).toBeVisible();
    await dialog.getByRole('button', { name: 'Delete page', exact: true }).click();
    await dialog.getByRole('button', { name: 'Confirm delete page' }).click();
    await expect(dialog.getByRole('alert')).toContainText(
      'Deletion acknowledged, awaiting verification',
    );
    await dialog.getByRole('button', { name: 'Verify signed log' }).click();
    await expect(dialog.getByRole('alert')).toContainText(
      'Deletion acknowledged, awaiting verification',
    );
    const catalog = JSON.parse(run(world, world.binaries.colab, ['ls', '--archived', '--json']));
    expect(catalog.pages).toHaveLength(0);
    expect(recipient.received()).toHaveLength(0);
  });
});
