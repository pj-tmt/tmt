import { expect, test, type Page } from '@playwright/test';
import { text } from '../src/strings.js';
import { capturePath } from './captures.js';

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

const typed = '<main><h1>Release notes v2</h1></main>';
async function edit(page: Page, answer: string, source = typed) {
  await script(page, answer);
  await page.locator('#source-edit').fill(source);
  await page.getByRole('button', { name: text.save, exact: true }).click();
}

test('an editor link tells an applied request, a changed page, a request still waiting and a too-large edit apart', async ({
  page,
}) => {
  await mount(page, 'editor');
  await page.getByTestId('source-toggle').click();
  const area = page.locator('#source-edit');
  const save = page.getByRole('button', { name: text.save, exact: true });
  // Waiting: the request is sent, the page has not changed, and the draft and its button hold.
  await edit(page, 'waiting');
  await expect(page.getByText(text.linkEditWaiting)).toBeVisible();
  await expect(page.getByText(text.saveUnconfirmed, { exact: true })).toBeVisible();
  await expect(save).toBeDisabled();
  await expect(area).toHaveValue(typed);
  // The page then shows the proposed text: applied, and no notice is left.
  await page.evaluate(async ({ fixture, typed }) => (await import(fixture)).settle(typed), {
    fixture,
    typed,
  });
  await expect(page.getByText(text.linkEditWaiting)).toHaveCount(0);
  await expect(save).toBeDisabled();
  await expect(page.locator('.source-notice')).toHaveCount(0);
  // Changed: the page shows something other than the proposed text. Nothing says it was applied.
  await edit(page, 'changed', '<main>two</main>');
  await expect(page.getByText(text.linkEditChanged)).toBeVisible();
  await expect(page.getByText(text.saveUnconfirmed, { exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: text.saveCopyDraft })).toBeVisible();
  await expect(area).toHaveValue('<main>two</main>');
  // Too large: nothing is sent, and the sizes are the edit's and a link's.
  await edit(page, 'large', '<main>three</main>');
  await expect(page.getByText(text.linkEditTooLarge(200 * 1024, 128 * 1024))).toBeVisible();
  await expect(page.getByText(text.saveNotSaved, { exact: true })).toBeVisible();
  // A lost race for the link's stream: nothing was sent, the draft is kept, and saving again is
  // offered.
  await edit(page, 'raced', '<main>four</main>');
  await expect(page.getByText(text.linkEditRaced)).toBeVisible();
  await expect(page.getByText(text.saveNotSaved, { exact: true })).toBeVisible();
  await expect(area).toHaveValue('<main>four</main>');
  await expect(save).toBeEnabled();
  await expect(page.getByText(text.saveMerged)).toHaveCount(0);
});

// The design-review captures of the three notices, at a phone and a desktop width in both themes.
for (const width of [390, 1280])
  for (const theme of ['light', 'dark'] as const)
    for (const notice of ['waiting', 'changed', 'large', 'raced'] as const)
      test(`link edit notice ${notice} at ${width} ${theme}`, async ({ page }) => {
        await page.setViewportSize({ width, height: width === 390 ? 844 : 800 });
        await page.emulateMedia({ colorScheme: theme });
        await mount(page, 'editor');
        await page.getByTestId('source-toggle').click();
        await edit(page, notice);
        await expect(
          page.getByText(
            notice === 'waiting'
              ? text.linkEditWaiting
              : notice === 'changed'
                ? text.linkEditChanged
                : notice === 'raced'
                  ? text.linkEditRaced
                  : text.linkEditTooLarge(200 * 1024, 128 * 1024),
          ),
        ).toBeVisible();
        await page.screenshot({ path: capturePath(`link-edit-${notice}-${width}-${theme}.png`) });
      });

// The Save button with nothing to save is the design system's disabled role: dim text, and no
// hover colour even with the pointer on it.
for (const width of [390, 1280])
  for (const theme of ['light', 'dark'] as const)
    test(`link edit Save is dim and unhighlighted while disabled at ${width} ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: width === 390 ? 844 : 800 });
      await page.emulateMedia({ colorScheme: theme });
      await mount(page, 'editor');
      await page.getByTestId('source-toggle').click();
      const save = page.getByRole('button', { name: text.save, exact: true });
      await expect(save).toBeDisabled();
      await save.hover({ force: true });
      const colors = await page.evaluate(() => {
        const probe = document.createElement('span');
        probe.style.color = 'var(--tmt-ui-color-disabled-text)';
        document.body.append(probe);
        const dim = getComputedStyle(probe).color;
        probe.remove();
        const button = document.querySelector<HTMLButtonElement>('.source > span button')!;
        return {
          dim,
          button: getComputedStyle(button).color,
          cursor: getComputedStyle(button).cursor,
        };
      });
      expect(colors.button).toBe(colors.dim);
      expect(colors.cursor).toBe('default');
      await page.screenshot({ path: capturePath(`link-edit-save-disabled-${width}-${theme}.png`) });
    });

for (const outcome of ['refused', 'unconfirmed', 'writer-in-use'] as const) {
  test(`a ${outcome} edit request retains its draft without claiming an owner-applied save`, async ({
    page,
  }) => {
    await mount(page, 'editor');
    await script(page, outcome);
    await page.getByTestId('source-toggle').click();
    const area = page.locator('#source-edit'),
      save = page.getByRole('button', { name: text.save, exact: true });
    await area.fill('<main>requested draft</main>');
    await save.click();
    await expect(
      page.getByText(
        outcome === 'writer-in-use'
          ? 'Link in use'
          : outcome === 'refused'
            ? 'Not sent'
            : 'Unconfirmed',
        { exact: true },
      ),
    ).toBeVisible();
    await expect(area).toHaveValue('<main>requested draft</main>');
    expect(await saves(page)).toHaveLength(1);
    // Only the still-admitted source is rendered; sending never substitutes the requested text.
    await expect(
      page.frameLocator('iframe').getByRole('heading', { name: 'Release notes', exact: true }),
    ).toBeVisible();
    if (outcome === 'writer-in-use') await expect(save).toBeDisabled();
    else await expect(save).toBeEnabled();
    if (outcome === 'writer-in-use') {
      await area.fill('<main>kept after definitive loss</main>');
      await expect(save).toBeDisabled();
      await expect(page.getByText('Link in use', { exact: true })).toBeVisible();
      await expect(page.locator('.source-notice [data-tone="blocked"]')).toBeVisible();
      await expect(page.locator('.chip')).toHaveText('Read-only');
      await expect(page.getByRole('button', { name: text.saveCopyDraft })).toBeVisible();
      await page.keyboard.press('Escape');
      await expect(area).not.toBeVisible();
      await page.getByTestId('source-toggle').click();
      await expect(save).toBeDisabled();
      await expect(area).toHaveValue('<main>kept after definitive loss</main>');
      await page.evaluate(() => {
        Object.defineProperty(navigator, 'clipboard', {
          value: {
            writeText: async (value: string) => {
              (window as unknown as { copiedDraft: string }).copiedDraft = value;
            },
          },
          configurable: true,
        });
      });
      await page.getByRole('button', { name: text.saveCopyDraft }).click();
      await expect
        .poll(() => page.evaluate(() => (window as unknown as { copiedDraft: string }).copiedDraft))
        .toBe('<main>kept after definitive loss</main>');
      expect(await saves(page)).toHaveLength(1);
    }
    if (outcome === 'unconfirmed') {
      await expect(page.locator('.source-notice [data-tone="waiting"]')).toBeVisible();
      // No observation or render effect can silently submit another operation.
      expect(await saves(page)).toHaveLength(1);
    }
  });
}

for (const width of [390, 1280])
  for (const theme of ['light', 'dark'] as const)
    test(`hosted Link in use keeps the draft at ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: width === 390 ? 844 : 800 });
      await page.emulateMedia({ colorScheme: theme });
      await mount(page, 'editor');
      await script(page, 'writer-in-use');
      await page.getByTestId('source-toggle').click();
      const area = page.locator('#source-edit'),
        save = page.getByRole('button', { name: text.save, exact: true });
      await area.fill('<main>A draft to keep</main>');
      await save.click();
      await expect(page.getByText('Link in use', { exact: true })).toBeVisible();
      await expect(
        page.getByText(
          'Another browser is already sending edits through this link, so this one can only read. Ask the owner for an editor link of your own.',
          { exact: true },
        ),
      ).toBeVisible();
      await expect(page.locator('.chip')).toHaveText('Read-only');
      await expect(page.getByRole('button', { name: text.saveCopyDraft })).toBeVisible();
      await expect(area).toHaveValue('<main>A draft to keep</main>');
      await expect(save).toBeDisabled();
      await page.screenshot({ path: capturePath(`hosted-link-in-use-${width}-${theme}.png`) });
    });
