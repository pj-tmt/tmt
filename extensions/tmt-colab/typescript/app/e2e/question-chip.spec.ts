import { mkdirSync } from 'node:fs';
import { expect, test, type Page } from '@playwright/test';
import { pageAction } from '../test/page-actions.js';
import { captureDirectory } from './captures.js';

const fixture = '/test/ask-page-browser.tsx';
async function run(page: Page, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
async function mount(page: Page, options = {}) {
  await page.goto('/');
  await run(page, 'mount');
  await run(page, 'chipPage', options);
  const chip = page.locator('.question-chip');
  await expect(chip).toBeVisible();
  await expect(page.locator('[data-question-group]')).toHaveAttribute('hidden', '');
  return chip;
}
const panel = (page: Page) => page.locator('[data-question-group]');
const frame = (page: Page) => page.frameLocator('#ask-page-fixture iframe');
/** Where the page text after the chip's paragraph starts, in frame coordinates. */
const afterTop = (page: Page) =>
  frame(page)
    .locator('#after-question')
    .evaluate((node) => node.getBoundingClientRect().top + scrollY);

/** The chip is drawn over the frame's placeholder: same place, same size, text reflowed around it. */
async function settled(page: Page) {
  await expect
    .poll(async () => {
      const chip = await page.locator('.question-chip').evaluate((node) => {
        const box = node.getBoundingClientRect();
        const origin = document.querySelector('#ask-page-fixture iframe')!.getBoundingClientRect();
        return { left: box.left - origin.left, top: box.top - origin.top, width: box.width };
      });
      const hole = await frame(page)
        .locator('[data-colab-chip-slot]')
        .evaluate((node) => {
          const box = node.getBoundingClientRect();
          return { left: box.left + scrollX, top: box.top + scrollY, width: box.width };
        });
      // The placeholder is the chip's size rounded up, so up to one pixel apart.
      return Math.max(
        Math.abs(chip.left - hole.left),
        Math.abs(chip.top - hole.top),
        Math.abs(chip.width - hole.width),
      );
    })
    .toBeLessThan(1.01);
  await expect(page.getByText('waiting for the verified view')).toHaveCount(0);
}

test('a chip-form block is one chip in the text; its panel opens below the paragraph and pushes the text down', async ({
  page,
}) => {
  const chip = await mount(page);
  await expect(chip).toHaveText('◆ Please answer 2 questions ▾');
  await expect(chip).toHaveAttribute('aria-expanded', 'false');
  // The authored block collapsed to an inline placeholder: its text is not drawn in the page.
  await expect(frame(page).locator('tmt-questions')).toBeHidden();
  await settled(page);
  const closed = await afterTop(page);
  await chip.click();
  await expect(chip).toHaveAttribute('aria-expanded', 'true');
  await expect(chip).toHaveText('◆ Please answer 2 questions ▴');
  await expect(panel(page)).toBeVisible();
  await expect(panel(page).locator('.needs-you-bar').first()).toContainText(
    'Please answer 2 questions · Agent 1',
  );
  // Below the paragraph that holds the chip, never over it, and the text below made room.
  await expect.poll(() => afterTop(page)).toBeGreaterThan(closed + 100);
  const geometry = await page.evaluate(() => {
    const chipBox = document.querySelector('.question-chip')!.getBoundingClientRect();
    const panelBox = document.querySelector('[data-question-group]')!.getBoundingClientRect();
    return { chipBottom: chipBox.bottom, panelTop: panelBox.top };
  });
  expect(geometry.panelTop).toBeGreaterThanOrEqual(geometry.chipBottom);
  await expect(panel(page).locator('.question-title').first()).toHaveText('Which list version?');
});

test('Esc and Collapse close the panel and return focus to the chip; reopening keeps drafts', async ({
  page,
}) => {
  const chip = await mount(page);
  await chip.click();
  await expect(panel(page)).toBeVisible();
  // Opening hands focus to the panel, so Esc works at once.
  await expect(panel(page).locator('.question-group-card')).toBeFocused();
  const card = () => panel(page).locator('.question-card:visible').first();
  await card().getByLabel('Side categories', { exact: true }).check();
  await panel(page).getByRole('tab', { name: /Q2/ }).click();
  await card().getByLabel('Your answer', { exact: true }).fill('Fade them');
  await page.keyboard.press('Escape');
  await expect(panel(page)).toBeHidden();
  await expect(chip).toBeFocused();
  await expect(chip).toHaveAttribute('aria-expanded', 'false');
  await chip.press('Enter');
  await expect(panel(page)).toBeVisible();
  await expect(card().getByLabel('Your answer', { exact: true })).toHaveValue('Fade them');
  await panel(page).getByRole('tab', { name: /Q1/ }).click();
  await expect(card().getByLabel('Side categories', { exact: true })).toBeChecked();
  await panel(page).getByRole('button', { name: 'Collapse', exact: true }).click();
  await expect(panel(page)).toBeHidden();
  await expect(chip).toBeFocused();
  // Nothing was published by choosing, tabbing or collapsing.
  expect(await run(page, 'discussionProof')).toHaveLength(0);
});

test('Esc inside the recipient popup closes only the popup, not the panel', async ({ page }) => {
  const chip = await mount(page);
  await chip.click();
  await panel(page).getByRole('button', { name: 'Add recipient', exact: true }).click();
  const search = page.getByRole('combobox', { name: 'Find an agent' });
  await expect(search).toBeVisible();
  await search.press('Escape');
  await expect(search).toBeHidden();
  await expect(panel(page)).toBeVisible();
  await expect(chip).toHaveAttribute('aria-expanded', 'true');
});

test('answering and sending turns the chip into an answered chip that stays openable, also after reload', async ({
  page,
}) => {
  const chip = await mount(page);
  await chip.click();
  const card = () => panel(page).locator('.question-card:visible').first();
  await card().getByLabel('Side categories', { exact: true }).check();
  await panel(page).getByRole('tab', { name: /Q2/ }).click();
  await card().getByLabel('Your answer', { exact: true }).fill('Fade them');
  await expect(chip).toHaveText('◆ Please answer 2 questions ▴');
  await panel(page)
    .getByRole('button', { name: /^Send 2 answers/ })
    .press('Enter');
  await expect(chip).toHaveText('✓ Answered 2 questions ▴');
  await expect(panel(page).locator('.needs-you-bar').first()).toContainText('Answered 2 questions');
  await expect.poll(async () => (await run(page, 'discussionProof')).length).toBe(2);
  // A neutral chip: the attention colour belongs to the open ◆ only.
  const colours = await chip.evaluate((node) => ({
    chip: getComputedStyle(node).color,
    mark: getComputedStyle(node.querySelector('.question-chip-mark')!).color,
  }));
  expect(colours.mark).toBe(colours.chip);
  await run(page, 'saveQuestionPage');
  await page.reload();
  await run(page, 'mount');
  await run(page, 'restoreQuestionPage');
  const restored = page.locator('.question-chip');
  await expect(restored).toHaveText('✓ Answered 2 questions ▾');
  await restored.click();
  await expect(panel(page).getByRole('button', { name: 'Change answer' }).first()).toBeVisible();
  await expect(panel(page).locator('.question-answer').first()).toHaveText('Side categories');
});

test('a lone question can be a chip and counts one question', async ({ page }) => {
  const chip = await mount(page, { count: 1 });
  await expect(chip).toHaveText('◆ Please answer 1 question ▾');
  await chip.click();
  await expect(panel(page).locator('.needs-you-bar').first()).toContainText(
    'Please answer 1 question',
  );
  await expect(panel(page).locator('.question-title')).toHaveText('Which list version?');
});

for (const [width, theme] of [
  [1440, 'light'],
  [1440, 'dark'],
  [390, 'light'],
  [390, 'dark'],
] as const)
  test(`chip look ${width} ${theme}: closed, open and answered stay in the text column`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/');
    await run(page, 'mount');
    await run(page, 'chipPage', { darkPage: theme === 'dark' });
    await (
      await pageAction(
        page.locator('#ask-page-fixture'),
        theme === 'dark' ? 'Theme: Dark' : 'Theme: Light',
      )
    ).click();
    await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
    const chip = page.locator('.question-chip');
    await expect(chip).toBeVisible();
    const directory = `${captureDirectory()}/question-chip`;
    mkdirSync(directory, { recursive: true });
    const inside = async () =>
      page.evaluate(() => {
        const frame = document.querySelector('#ask-page-fixture iframe')!.getBoundingClientRect();
        const boxes = ['.question-chip', '[data-question-group]:not([hidden])'].flatMap(
          (selector) => {
            const node = document.querySelector(selector);
            return node ? [node.getBoundingClientRect()] : [];
          },
        );
        return {
          overflow: document.documentElement.scrollWidth > innerWidth,
          outside: boxes.some((box) => box.left < frame.left - 1 || box.right > frame.right + 1),
        };
      });
    await settled(page);
    expect(await inside()).toEqual({ overflow: false, outside: false });
    await page.screenshot({ path: `${directory}/chip-closed-${width}-${theme}.png` });
    await chip.click();
    await expect(panel(page)).toBeVisible();
    await expect(panel(page).locator('.needs-you-bar').first()).toBeVisible();
    expect(await inside()).toEqual({ overflow: false, outside: false });
    await page.screenshot({ path: `${directory}/chip-open-${width}-${theme}.png` });
    const card = () => panel(page).locator('.question-card:visible').first();
    await card().getByLabel('Side categories', { exact: true }).check();
    await panel(page).getByRole('tab', { name: /Q2/ }).click();
    await card().getByLabel('Your answer', { exact: true }).fill('Fade them');
    await panel(page)
      .getByRole('button', { name: /^Send 2 answers/ })
      .press('Enter');
    await expect(chip).toHaveText('✓ Answered 2 questions ▴');
    await settled(page);
    await page.screenshot({ path: `${directory}/chip-answered-open-${width}-${theme}.png` });
    await chip.click();
    await expect(panel(page)).toBeHidden();
    await settled(page);
    await page.screenshot({ path: `${directory}/chip-answered-${width}-${theme}.png` });
    expect(await inside()).toEqual({ overflow: false, outside: false });
  });
