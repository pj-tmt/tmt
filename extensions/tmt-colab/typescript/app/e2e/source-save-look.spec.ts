import { pageAction } from '../test/page-actions.js';
import { expect, test, type Locator, type Page } from '@playwright/test';
import { text } from '../src/strings.js';
import { capturePath } from './captures.js';

/**
 * A Source save in flight fences its controls without changing how they look (gui-style.md,
 * Transient states): the Save keeps its idle colours under the pointer, nothing moves, and the
 * Files panel stays enabled but writes nothing until the save has landed.
 */
const fixture = '/test/ask-page-browser.tsx';
const run = (page: Page, method: string, argument?: unknown) =>
  page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
const actions = async (page: Page): Promise<string[]> => (await run(page, 'proof')).actions;

/** The look a capture run uses, `COLAB_LOOK=390:dark`; the assertions hold in every look. */
const appearance = process.env.COLAB_LOOK ?? '1440:light';
async function openSource(page: Page) {
  const [width, scheme] = appearance.split(':');
  await page.setViewportSize({ width: Number(width), height: 900 });
  await page.emulateMedia({ colorScheme: scheme === 'dark' ? 'dark' : 'light' });
  await page.goto('/');
  await run(page, 'mount', { attachments: true });
  await expect(page.locator('#ask-page-fixture .status')).toContainText('Live');
  await (await pageAction(page.locator('#ask-page-fixture'), 'Source')).click();
  await page
    .locator('#source-edit:visible')
    .fill('<p id="selected">Exact selected text, edited</p>');
  return page.getByRole('button', { name: text.save, exact: true });
}
const look = (save: Locator) =>
  save.evaluate((node) => {
    const style = getComputedStyle(node);
    const rect = node.getBoundingClientRect();
    const field = document.querySelector('#source-edit')!.getBoundingClientRect();
    return {
      color: style.color,
      background: style.backgroundColor,
      rect: [rect.x, rect.y, rect.width, rect.height],
      field: [field.x, field.y, field.width, field.height],
    };
  });

test('a Source save under the pointer keeps the look and the place of its Save', async ({
  page,
}) => {
  const save = await openSource(page);
  await page.mouse.move(2, 2);
  const idle = await look(save);
  await run(page, 'holdEdit');
  await save.click();
  await expect(save).toHaveAttribute('aria-busy', 'true');
  // The pointer is still over the Save: a fenced action does not tint like an active one.
  const busy = await look(save);
  expect(busy.background).toBe(idle.background);
  expect(busy.color).toBe(idle.color);
  expect(busy.rect).toEqual(idle.rect);
  expect(busy.field).toEqual(idle.field);
  if (process.env.COLAB_CAPTURE_DIR)
    await page.screenshot({
      path: capturePath(`source-save-busy-${appearance.replace(':', '-')}.png`),
    });
  await run(page, 'releaseEdit');
  await expect(save).not.toHaveAttribute('aria-busy', 'true');
});

const note = (name: string) => ({
  name,
  mimeType: 'text/plain',
  buffer: Buffer.from(`bytes of ${name}`),
});
const files = async (page: Page) =>
  (await actions(page)).filter((value) => value.startsWith('files:'));
const open = async (page: Page, panel: 'Files' | 'Source') =>
  (await pageAction(page.locator('#ask-page-fixture'), panel)).click();

/** A page with one file, a Source draft, and a Source save held in flight; the Files panel open. */
async function savingWithAFile(page: Page) {
  const save = await openSource(page);
  await open(page, 'Files');
  const panel = page.getByTestId('files-panel');
  const chooser = page.waitForEvent('filechooser');
  await panel.getByRole('button', { name: text.attachFiles }).click();
  await (await chooser).setFiles([note('first.txt')]);
  await panel.getByRole('button', { name: text.filesAdd, exact: true }).click();
  await expect(panel.getByRole('button', { name: text.attachRemove })).toBeVisible();
  await open(page, 'Source');
  await page.locator('#source-edit:visible').fill('<p id="selected">Edited while files change</p>');
  await run(page, 'holdEdit');
  await save.click();
  await expect(save).toHaveAttribute('aria-busy', 'true');
  await open(page, 'Files');
  return panel;
}
async function attachSecond(page: Page, panel: Locator) {
  const chooser = page.waitForEvent('filechooser');
  await panel.getByRole('button', { name: text.attachFiles }).click();
  await (await chooser).setFiles([note('second.txt')]);
}

test('an Add and a Remove during a Source save wait for it, then apply once', async ({ page }) => {
  const panel = await savingWithAFile(page);
  await attachSecond(page, panel);
  const add = panel.getByRole('button', { name: text.filesAdd, exact: true });
  await expect(add).toBeEnabled();
  await panel.getByRole('button', { name: text.attachRemove }).first().click();
  await add.click();
  // Nothing is lost and nothing is written while the save is in flight.
  expect(await files(page)).toEqual(['files:add:first.txt']);
  await run(page, 'releaseEdit');
  // Each applies exactly once; the two were asked together, so their order is not asserted.
  await expect
    .poll(async () => (await files(page)).sort())
    .toEqual(['files:add:first.txt', 'files:add:second.txt', 'files:remove:1']);
});

test('an Add and a Remove during a Source save that fails apply nothing', async ({ page }) => {
  const panel = await savingWithAFile(page);
  await attachSecond(page, panel);
  await panel.getByRole('button', { name: text.attachRemove }).first().click();
  await panel.getByRole('button', { name: text.filesAdd, exact: true }).click();
  await run(page, 'releaseEdit', { fail: true });
  // The save's error stays in the Source drawer; the staged file is kept to add again.
  await expect(panel.getByRole('button', { name: text.filesAdd, exact: true })).toBeEnabled();
  await open(page, 'Source');
  await expect(page.getByText(text.saveNotSaved, { exact: true })).toBeVisible();
  expect(await files(page)).toEqual(['files:add:first.txt']);
});
