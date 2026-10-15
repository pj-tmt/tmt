import { expect, test, type Page } from '@playwright/test';
import { text } from '../src/strings.js';

const fixture = '/test/reader-editor-browser.tsx';
async function mount(page: Page, role: 'viewer' | 'editor') {
  await page.goto('/');
  await page.evaluate(async ({ fixture, role }) => (await import(fixture)).mount(role), {
    fixture,
    role,
  });
  await expect(
    page
      .locator('#reader-editor-fixture')
      .getByRole('heading', { name: 'Release notes', exact: true }),
  ).toBeVisible();
}
const script = (page: Page, answer: string) =>
  page.evaluate(async ({ fixture, answer }) => (await import(fixture)).script(answer), {
    fixture,
    answer,
  });
const saves = (page: Page) =>
  page.evaluate(async (fixture) => (await import(fixture)).proof().saves, fixture);
async function note(page: Page) {
  await page.getByLabel('Info', { exact: true }).click();
  return page.locator('.reader-information-panel');
}

test('a viewer link is marked read-only, says so, and has no Source control', async ({ page }) => {
  await mount(page, 'viewer');
  await expect(page.locator('.chip')).toHaveText(text.readerOnly);
  await expect(page.getByTestId('source-toggle')).toHaveCount(0);
  await expect(await note(page)).toContainText(text.readerNote);
});

test('an editor link is marked Can edit, names what a save does, and saves from the Source drawer', async ({
  page,
}) => {
  await mount(page, 'editor');
  await expect(page.locator('.chip')).toHaveText(text.readerCanEdit);
  await expect(await note(page)).toContainText(text.readerNoteEditor);
  await expect(page.locator('.reader-information-panel')).not.toContainText('You cannot edit it');
  await page.getByTestId('source-toggle').click();
  const area = page.locator('#source-edit');
  const save = page.getByRole('button', { name: text.save, exact: true });
  await expect(area).toHaveValue('<main><h1>Release notes</h1></main>');
  await expect(save).toBeDisabled();
  await area.fill('<main><h1>Release notes v2</h1></main>');
  await expect(save).toBeEnabled();
  await save.click();
  await expect
    .poll(() => saves(page))
    .toEqual([
      {
        source: '<main><h1>Release notes v2</h1></main>',
        base: '<main><h1>Release notes</h1></main>',
      },
    ]);
  // The saved text is the editor's new base: nothing left to save, and no problem is shown.
  await expect(save).toBeDisabled();
  await expect(page.locator('.source-notice')).toHaveCount(0);
});

test('an editor link tells a merged save, a conflict and a full page apart', async ({ page }) => {
  await mount(page, 'editor');
  await page.getByTestId('source-toggle').click();
  const area = page.locator('#source-edit');
  const save = page.getByRole('button', { name: text.save, exact: true });
  await script(page, 'merged');
  await area.fill('<main>one</main>');
  await save.click();
  await expect(page.getByText(text.saveMerged)).toBeVisible();
  await script(page, 'conflict');
  await area.fill('<main>two</main>');
  await save.click();
  await expect(page.getByText(text.saveNotSaved)).toBeVisible();
  await expect(page.getByRole('button', { name: text.saveCopyDraft })).toBeVisible();
  // The draft is kept for another try.
  await expect(area).toHaveValue('<main>two</main>');
  await script(page, 'full');
  await save.click();
  await expect(page.getByText(text.pageSize.saveFullReader)).toBeVisible();
});
