import { readerAction } from '../test/page-actions.js';
import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';
const fixture = '/test/reader-metadata-browser.tsx';
async function run(page: Page, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
async function mount(page: Page, role = 'editor') {
  await page.goto('/');
  await run(page, 'mount', role);
  await expect(page.locator('#reader-metadata-fixture .status')).toContainText('Live');
}
async function info(page: Page) {
  if (!(await page.getByRole('dialog', { name: 'Info', exact: true }).isVisible()))
    await (await readerAction(page, 'Info')).click();
  return page.locator('.reader-information-panel');
}
async function creator(page: Page) {
  await (await info(page)).getByRole('button', { name: 'Change creator…', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Change creator', exact: true });
  await expect(dialog).toBeVisible();
  await expect(page.getByRole('dialog', { name: 'Info', exact: true })).toBeHidden();
  return dialog;
}
test.beforeEach(async ({ page }) => {
  await page.route('**/src/main.tsx', (route) =>
    route.fulfill({ contentType: 'text/javascript', body: '' }),
  );
});
test('editor metadata direct actions preserve source, focus, summaries and responsive look', async ({
  page,
}) => {
  test.setTimeout(90_000);
  for (const width of [1440, 390])
    for (const theme of ['light', 'dark']) {
      await page.setViewportSize({ width, height: 900 });
      await mount(page);
      await page.evaluate((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      if (process.env.COLAB_5195_BEFORE_MODULE) {
        await run(page, 'reference');
        await expect(page.getByRole('dialog', { name: 'Questions', exact: true })).toBeVisible();
        await page.screenshot({
          path: capturePath(`5195-${width}-${theme}-reference-questions.png`),
          fullPage: true,
        });
        await run(page, 'before', process.env.COLAB_5195_BEFORE_MODULE);
        await info(page);
        await expect(page.getByRole('button', { name: 'Change creator…' })).toHaveCount(0);
        await page.screenshot({
          path: capturePath(`5195-${width}-${theme}-before-info.png`),
          fullPage: true,
        });
        await run(page, 'after');
      }
      const panel = await info(page);
      await expect(
        panel.getByRole('button', { name: 'Change creator…', exact: true }),
      ).toBeVisible();
      await expect(panel).toContainText('1 open · 0 answered · 1 handled');
      await expect(panel).toContainText('Assigned to Current creator');
      await expect(panel).toContainText('Removed from page source');
      await page.screenshot({
        path: capturePath(`5195-${width}-${theme}-after-info.png`),
        fullPage: true,
      });
      await panel.getByRole('button', { name: 'Mark handled', exact: true }).click();
      await expect(panel).toContainText('0 open · 0 answered · 2 handled');
      await page.screenshot({
        path: capturePath(`5195-${width}-${theme}-handled.png`),
        fullPage: true,
      });
      await expect(panel).toContainText('Change applied');
      await panel.getByRole('button', { name: 'Reopen', exact: true }).first().click();
      await expect(panel).toContainText('1 open · 0 answered · 1 handled');
      const dialog = await creator(page);
      await dialog.getByLabel('Agent name or ID').fill('New creator');
      await page.screenshot({
        path: capturePath(`5195-${width}-${theme}-creator.png`),
        fullPage: true,
      });
      await dialog.getByRole('button', { name: 'Change creator', exact: true }).click();
      await expect(dialog).toBeHidden();
      await expect(page.getByRole('button', { name: 'More', exact: true })).toBeFocused();
      await expect(await info(page)).toContainText('New creator');
      const proof = await run(page, 'proof');
      expect(proof.requests.map((r: { action: string }) => r.action)).toEqual([
        'question.handle',
        'question.reopen',
        'creator.set',
      ]);
      expect(proof.saves).toEqual([]);
      expect(proof.view.source).toContain('First question');
      await run(page, 'script', 'refused');
      let pendingDialog = await creator(page);
      await pendingDialog.getByLabel('Agent name or ID').fill('Missing agent');
      await pendingDialog.getByRole('button', { name: 'Change creator', exact: true }).click();
      await expect(pendingDialog).toContainText('No saved agent matches');
      await page.screenshot({
        path: capturePath(`5195-${width}-${theme}-refused.png`),
        fullPage: true,
      });
      await pendingDialog.getByRole('button', { name: 'Close', exact: true }).click();
      await run(page, 'script', 'waiting');
      pendingDialog = await creator(page);
      await pendingDialog.getByLabel('Agent name or ID').fill('Busy creator');
      await pendingDialog.getByRole('button', { name: 'Change creator', exact: true }).click();
      await expect(pendingDialog).toContainText('Changing…');
      await page.screenshot({
        path: capturePath(`5195-${width}-${theme}-busy.png`),
        fullPage: true,
      });
      await pendingDialog.getByRole('button', { name: 'Close', exact: true }).click();
      await run(page, 'settle');
      await expect(await info(page)).toContainText('Busy creator');
      await run(page, 'script', 'unconfirmed');
      pendingDialog = await creator(page);
      await pendingDialog.getByLabel('Agent name or ID').fill('Later creator');
      await pendingDialog.getByRole('button', { name: 'Change creator', exact: true }).click();
      await expect(pendingDialog).toContainText('Unconfirmed');
      await page.screenshot({
        path: capturePath(`5195-${width}-${theme}-unconfirmed.png`),
        fullPage: true,
      });
      await pendingDialog.getByRole('button', { name: 'Check result', exact: true }).click();
      await run(page, 'settle');
      await expect(pendingDialog).toContainText('Change applied');
      await pendingDialog.getByRole('button', { name: 'Close', exact: true }).click();
      expect((await run(page, 'proof')).requests).toHaveLength(6);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
    }
});
test('refusal, busy close and uncertain check never replay a submitted metadata action', async ({
  page,
}) => {
  await mount(page);
  await (await readerAction(page, 'Source')).click();
  const area = page.locator('#source-edit');
  await area.fill('<h1>Keep this unsaved source draft</h1>');
  await page.getByRole('button', { name: 'Close Source', exact: true }).click();
  await run(page, 'script', 'refused');
  let dialog = await creator(page);
  await dialog.getByLabel('Agent name or ID').fill('Missing agent');
  await dialog.getByRole('button', { name: 'Change creator', exact: true }).click();
  await expect(dialog).toContainText('No saved agent matches that name or ID.');
  await page.screenshot({ path: capturePath('5195-refused.png'), fullPage: true });
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  await (await readerAction(page, 'Source')).click();
  await expect(area).toHaveValue('<h1>Keep this unsaved source draft</h1>');
  await page.getByRole('button', { name: 'Close Source', exact: true }).click();
  await run(page, 'script', 'waiting');
  dialog = await creator(page);
  await dialog.getByLabel('Agent name or ID').fill('Eventually applied');
  await dialog.getByRole('button', { name: 'Change creator', exact: true }).click();
  await expect(dialog).toContainText('Changing…');
  await page.screenshot({ path: capturePath('5195-busy.png'), fullPage: true });
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  await run(page, 'settle');
  await expect(await info(page)).toContainText('Eventually applied');
  expect((await run(page, 'proof')).requests).toHaveLength(2);
  await run(page, 'script', 'unconfirmed');
  dialog = await creator(page);
  await dialog.getByLabel('Agent name or ID').fill('Confirmed later');
  await dialog.getByRole('button', { name: 'Change creator', exact: true }).click();
  await expect(dialog).toContainText('Unconfirmed');
  await expect(dialog.getByRole('button', { name: 'Change creator', exact: true })).toBeDisabled();
  await page.screenshot({ path: capturePath('5195-unconfirmed.png'), fullPage: true });
  await dialog.getByRole('button', { name: 'Check result', exact: true }).click();
  await run(page, 'settle');
  await expect(dialog).toContainText('Change applied');
  expect((await run(page, 'proof')).requests).toHaveLength(3);
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  await run(page, 'script', 'waiting');
  dialog = await creator(page);
  await dialog.getByLabel('Agent name or ID').fill('Late result from replaced page');
  await dialog.getByRole('button', { name: 'Change creator', exact: true }).click();
  await expect(dialog).toContainText('Changing…');
  await run(page, 'mount', 'viewer');
  await run(page, 'settle');
  await expect((await info(page)).getByRole('button')).toHaveCount(0);
  expect((await run(page, 'proof')).view.creator.agent.name).toBe('Current creator');
});
test('non-editors, archived and legacy readers keep their read-only boundary', async ({ page }) => {
  for (const role of ['viewer', 'commenter']) {
    await mount(page, role);
    const panel = await info(page);
    await expect(panel.getByRole('button')).toHaveCount(0);
    await expect(panel).toContainText('Only the page owner can change this here');
    expect((await run(page, 'proof')).requests).toEqual([]);
  }
  await mount(page);
  await run(page, 'archived');
  await expect((await info(page)).getByRole('button')).toHaveCount(0);
  await expect(await info(page)).toContainText('Archived pages cannot be changed');
  await mount(page);
  await run(page, 'legacy');
  await expect((await info(page)).getByRole('button', { name: 'Mark handled' })).toHaveCount(0);
  expect((await run(page, 'proof')).requests).toEqual([]);
});
