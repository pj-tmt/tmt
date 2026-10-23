import { expect, test, type Page } from '@playwright/test';
import { pageAction } from '../test/page-actions.js';

const fixture = '/test/ask-page-browser.tsx';
const PNG = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==',
  'base64',
);
const proof = (page: Page) =>
  page.evaluate(async (path) => (await import(path)).proof().actions as string[], fixture);
async function open(page: Page) {
  await page.goto('/');
  await page.evaluate(async (path) => (await import(path)).mount({ attachments: true }), fixture);
  await expect(page.locator('#ask-page-fixture .status')).toContainText('Live');
  await (await pageAction(page.locator('#ask-page-fixture'), 'Files')).click();
  return page.getByTestId('files-panel');
}
async function pick(page: Page, name: string, png = false) {
  await page.getByLabel('Choose files to stage', { exact: true }).setInputFiles({
    name,
    mimeType: png ? 'image/png' : 'text/plain',
    buffer: png ? PNG : Buffer.from('local staged bytes'),
  });
}

for (const width of [1586, 390]) {
  for (const colorScheme of ['light', 'dark'] as const) {
    test(`Files follows the website at ${width} ${colorScheme} without reading bytes`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 992 });
      await page.emulateMedia({ colorScheme });
      const panel = await open(page);
      await expect(
        panel.getByRole('heading', { name: 'Page files (0)', exact: true }),
      ).toBeVisible();
      await pick(page, 'image.png', true);
      expect(await proof(page)).toEqual([]);
      await panel.getByRole('button', { name: 'Add to page', exact: true }).click();
      await expect(
        panel.getByRole('heading', { name: 'Page files (1)', exact: true }),
      ).toBeVisible();
      const row = panel.getByTestId('file-row');
      await expect(row.getByRole('button', { name: 'Preview', exact: true })).toBeVisible();
      await expect(row.getByRole('button', { name: 'Download', exact: true })).toBeVisible();
      await expect(row.getByRole('button', { name: 'Remove', exact: true })).toHaveAttribute(
        'data-variant',
        'ghost',
      );
      expect(await proof(page)).toEqual(['attach:upload:image.png', 'files:add:image.png']);
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        width,
      );
    });
  }
}

test('preview replaces Files and returns focus without losing staged bytes; remove needs confirmation', async ({
  page,
}) => {
  const panel = await open(page);
  await pick(page, 'image.png', true);
  await panel.getByRole('button', { name: 'Add to page', exact: true }).click();
  const row = panel.getByTestId('file-row');
  await expect(row).toHaveCount(1);
  await pick(page, 'kept.txt');
  const preview = row.getByRole('button', { name: 'Preview', exact: true });
  await preview.click();
  const dialog = page.getByRole('dialog', { name: 'image.png', exact: true });
  await expect(dialog).toBeVisible();
  await expect(dialog.locator('img')).toHaveAttribute('src', /^data:image\/png;base64,/);
  await expect(page.locator('dialog[open]')).toHaveCount(1);
  await page.evaluate(async (path) => (await import(path)).advanceDisclosure(), fixture);
  await expect(dialog.locator('img')).toHaveCount(0);
  await expect(dialog.getByRole('alert')).toHaveText(
    'The page changed while opening this file. Try again.',
  );
  expect((await proof(page)).filter((entry) => entry.startsWith('attach:open'))).toHaveLength(1);
  await page.keyboard.press('Escape');
  await expect(preview).toBeFocused();
  await expect(panel.getByTestId('attachment-chip')).toContainText('kept.txt');
  const remove = row.getByRole('button', { name: 'Remove', exact: true });
  const before = await proof(page);
  await remove.click();
  const confirmation = page.getByRole('dialog', { name: 'Remove image.png?', exact: true });
  await expect(confirmation).toBeVisible();
  await expect(page.locator('dialog[open]')).toHaveCount(1);
  expect(await proof(page)).toEqual(before);
  await confirmation.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(remove).toBeFocused();
  await expect(row).toHaveCount(1);
  await remove.click();
  await confirmation.getByRole('button', { name: 'Remove file', exact: true }).click();
  await expect(panel.getByTestId('file-row')).toHaveCount(0);
  await expect(panel.getByTestId('attachment-chip')).toContainText('kept.txt');
  expect(await proof(page)).toEqual([...before, 'files:remove:1']);
});

test('long filenames remain inside the phone file row and both dialog headers', async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 992 });
  await page.emulateMedia({ colorScheme: 'dark' });
  const panel = await open(page);
  const name = `${'long-file-name-'.repeat(16)}.png`;
  await pick(page, name, true);
  await panel.getByRole('button', { name: 'Add to page', exact: true }).click();
  const row = panel.getByTestId('file-row');
  await expect(row).toContainText(name);
  expect(await row.evaluate((e) => e.scrollWidth <= e.clientWidth)).toBe(true);
  for (const action of ['Preview', 'Remove']) {
    await row.getByRole('button', { name: action, exact: true }).click();
    const dialog = page.getByRole('dialog');
    await expect(dialog).toBeVisible();
    const heading = dialog.getByRole('heading');
    const fits = await heading.evaluate((e) => {
      const box = e.getBoundingClientRect();
      const parent = e.parentElement!.getBoundingClientRect();
      return e.scrollWidth <= e.clientWidth && box.right <= parent.right;
    });
    expect(fits).toBe(true);
    await page.keyboard.press('Escape');
  }
});

test('untrusted confirmation and a descriptor removed while confirming write nothing', async ({
  page,
}) => {
  const panel = await open(page);
  await pick(page, 'notes.txt');
  await panel.getByRole('button', { name: 'Add to page', exact: true }).click();
  await panel.getByTestId('file-row').getByRole('button', { name: 'Remove', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Remove notes.txt?', exact: true });
  const remove = dialog.getByRole('button', { name: 'Remove file', exact: true });
  const before = await proof(page);
  await remove.dispatchEvent('click');
  expect(await proof(page)).toEqual(before);
  await page.evaluate(async (path) => (await import(path)).removePageFiles(), fixture);
  await expect(remove).toBeDisabled();
  await expect(dialog.getByRole('alert')).toHaveText(
    'The page changed while opening this file. Try again.',
  );
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(panel.getByTestId('file-row')).toHaveCount(0);
  expect(await proof(page)).toEqual(before);
});

test('archiving while removal confirmation is open withdraws write admission', async ({ page }) => {
  const panel = await open(page);
  await pick(page, 'notes.txt');
  await panel.getByRole('button', { name: 'Add to page', exact: true }).click();
  await panel.getByTestId('file-row').getByRole('button', { name: 'Remove', exact: true }).click();
  await page.evaluate(async (path) => (await import(path)).archive(), fixture);
  const dialog = page.getByRole('dialog', { name: 'Remove notes.txt?', exact: true });
  await expect(dialog.getByRole('button', { name: 'Remove file', exact: true })).toBeDisabled();
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(panel.getByTestId('file-row')).toHaveCount(1);
  await expect(panel.getByRole('button', { name: 'Remove', exact: true })).toHaveCount(0);
  await expect(panel.getByLabel('Choose files to stage', { exact: true })).toHaveCount(0);
  expect(await proof(page)).toEqual(['attach:upload:notes.txt', 'files:add:notes.txt']);
});

test('closing a pending preview retains staged bytes and never displays its late result', async ({
  page,
}) => {
  const panel = await open(page);
  await pick(page, 'slow.png', true);
  await panel.getByRole('button', { name: 'Add to page', exact: true }).click();
  await pick(page, 'kept.txt');
  await panel.getByTestId('file-row').getByRole('button', { name: 'Preview', exact: true }).click();
  await expect
    .poll(async () => (await proof(page)).filter((a) => a.startsWith('attach:open')).length)
    .toBe(1);
  await page.keyboard.press('Escape');
  await page.evaluate(async (path) => (await import(path)).releaseOpen(), fixture);
  await expect(
    panel.getByTestId('file-row').getByRole('button', { name: 'Preview', exact: true }),
  ).not.toHaveAttribute('aria-busy', 'true');
  await expect(page.getByTestId('file-preview')).toHaveCount(0);
  await expect(panel.getByTestId('attachment-chip')).toContainText('kept.txt');
  expect((await proof(page)).filter((a) => a.startsWith('attach:open'))).toEqual([
    'attach:open:slow.png:document',
  ]);
});
