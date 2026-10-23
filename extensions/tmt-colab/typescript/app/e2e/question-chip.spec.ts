import { mkdirSync } from 'node:fs';
import { expect, test, type Page } from '@playwright/test';
import { pageAction } from '../test/page-actions.js';
import { captureDirectory } from './captures.js';
import { expectAnswered } from './question-line.js';

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
  const rows = page.locator('.question-line');
  await expect(rows.first()).toBeVisible();
  await expect(rows.first()).toHaveAttribute('aria-expanded', 'false');
  return rows;
}
const panel = (page: Page) => page.locator('[data-question-group]');
const frame = (page: Page) => page.frameLocator('#ask-page-fixture iframe');
const card = (page: Page) => panel(page).locator('.question-card:visible').first();
const afterTop = (page: Page) =>
  frame(page)
    .locator('#after-question')
    .evaluate((node) => node.getBoundingClientRect().top + scrollY);

/** The compact block starts at the existing after spacer in the authored column. */
async function settled(page: Page) {
  await expect
    .poll(async () => {
      const block = await page.locator('.question-compact-block').boundingBox();
      const spacer = await frame(page).locator('[data-colab-proposal-slot]').boundingBox();
      return !!block && !!spacer && Math.abs(block.y - spacer.y) <= 1;
    })
    .toBe(true);
  await expect(page.getByText('waiting for the verified view')).toHaveCount(0);
}

test('existing chip-form markup becomes compact rows and opening pushes following prose down', async ({
  page,
}) => {
  const rows = await mount(page);
  await expect(rows).toHaveCount(2);
  await expect(page.getByRole('tab')).toHaveCount(0);
  await expect(rows.first()).toContainText('Which list version?');
  await expect(rows.first()).toContainText('Needs your answer');
  await expect(frame(page).locator('tmt-questions')).toBeHidden();
  await settled(page);
  const closed = await afterTop(page);
  await rows.first().click();
  await expect(rows.first()).toHaveAttribute('aria-expanded', 'true');
  await expect(card(page).getByLabel('One column', { exact: true })).toBeVisible();
  await expect.poll(() => afterTop(page)).toBeGreaterThan(closed + 100);
  await expect
    .poll(async () => {
      const trusted = await page.locator('.question-compact-block').boundingBox();
      const after = await frame(page).locator('#after-question').boundingBox();
      return !!trusted && !!after && after.y >= trusted.y + trusted.height - 1;
    })
    .toBe(true);
});

test('Escape and the disclosure return focus and reopening keeps both local drafts', async ({
  page,
}) => {
  const rows = await mount(page);
  await rows.first().click();
  await expect(card(page).getByLabel('One column', { exact: true })).toBeFocused();
  await card(page).getByLabel('Side categories', { exact: true }).check();
  await rows.nth(1).click();
  await card(page).getByLabel('Your answer', { exact: true }).fill('Fade them');
  await page.keyboard.press('Escape');
  await expect(rows.nth(1)).toBeFocused();
  await expect(rows.nth(1)).toHaveAttribute('aria-expanded', 'false');
  await rows.nth(1).press('Enter');
  await expect(card(page).getByLabel('Your answer', { exact: true })).toHaveValue('Fade them');
  await rows.first().click();
  await expect(card(page).getByLabel('Side categories', { exact: true })).toBeChecked();
  await rows.first().click();
  await expect(rows.first()).toBeFocused();
  await expect(rows.first()).toHaveAttribute('aria-expanded', 'false');
  expect(await run(page, 'discussionProof')).toHaveLength(0);
  expect((await run(page, 'proof')).sends).toHaveLength(0);
});

test('Escape inside the recipient popup closes only the popup', async ({ page }) => {
  const rows = await mount(page);
  await rows.first().click();
  await panel(page).getByRole('button', { name: 'Add recipient', exact: true }).click();
  const search = page.getByRole('combobox', { name: 'Find an agent' });
  await expect(search).toBeVisible();
  await search.press('Escape');
  await expect(search).toBeHidden();
  await expect(rows.first()).toHaveAttribute('aria-expanded', 'true');
  await expect(card(page)).toBeVisible();
});

test('verified answers close the rows and stay openable after reload', async ({ page }) => {
  const rows = await mount(page);
  await rows.first().click();
  await card(page).getByLabel('Side categories', { exact: true }).check();
  await rows.nth(1).click();
  await card(page).getByLabel('Your answer', { exact: true }).fill('Fade them');
  await panel(page)
    .getByRole('button', { name: /^Send 2 answers/ })
    .press('Enter');
  await expectAnswered(rows.first(), 'Side categories');
  await expectAnswered(rows.nth(1), 'Fade them');
  await expect(rows.nth(1)).toHaveAttribute('aria-expanded', 'false');
  await expect.poll(async () => (await run(page, 'discussionProof')).length).toBe(2);
  await run(page, 'saveQuestionPage');
  await page.reload();
  await run(page, 'mount');
  await run(page, 'restoreQuestionPage');
  const restored = page.locator('.question-line');
  await expectAnswered(restored.first(), 'Side categories');
  await expectAnswered(restored.nth(1), 'Fade them');
  await expect(restored.first()).toHaveAttribute('aria-expanded', 'false');
  await restored.first().click();
  await expect(card(page).getByRole('button', { name: 'Change answer' })).toBeVisible();
  await expect(card(page).locator('.question-answer > p')).toHaveText('Side categories');
  expect((await run(page, 'proof')).sends).toHaveLength(0);
});

test('a lone legacy chip question is one compact line', async ({ page }) => {
  const rows = await mount(page, { count: 1 });
  await expect(rows).toHaveCount(1);
  await expect(page.locator('.question-compact-heading')).toHaveCount(0);
  await expect(rows).toContainText('Which list version?');
  await rows.click();
  await expect(card(page).getByLabel('One column', { exact: true })).toBeVisible();
});

for (const [width, theme] of [
  [1440, 'light'],
  [1440, 'dark'],
  [390, 'light'],
  [390, 'dark'],
] as const) {
  test(`compact legacy block look ${width} ${theme}: closed, open and answered`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    const rows = await mount(page, { darkPage: theme === 'dark' });
    await (
      await pageAction(
        page.locator('#ask-page-fixture'),
        theme === 'dark' ? 'Theme: Dark' : 'Theme: Light',
      )
    ).click();
    await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
    const directory = `${captureDirectory()}/question-chip`;
    mkdirSync(directory, { recursive: true });
    const inside = async () =>
      page.evaluate(() => {
        const frame = document.querySelector('#ask-page-fixture iframe')!.getBoundingClientRect();
        const boxes = [...document.querySelectorAll('.question-compact-block')].map((node) =>
          node.getBoundingClientRect(),
        );
        return {
          overflow: document.documentElement.scrollWidth > innerWidth,
          outside: boxes.some((box) => box.left < frame.left - 1 || box.right > frame.right + 1),
        };
      });
    const neutralText = async () => {
      const colours = await rows.first().evaluate((node) => ({
        text: getComputedStyle(node).color,
        title: getComputedStyle(node.querySelector('.question-line-title')!).color,
        mark: getComputedStyle(node.querySelector('.question-line-mark')!).color,
      }));
      expect(colours.title).toBe(colours.text);
      expect(colours.mark).not.toBe(colours.text);
    };
    await settled(page);
    expect(await inside()).toEqual({ overflow: false, outside: false });
    await neutralText();
    await page.screenshot({ path: `${directory}/compact-closed-${width}-${theme}.png` });
    await rows.first().hover();
    await neutralText();
    await rows.first().click();
    await expect(card(page)).toBeVisible();
    expect(await inside()).toEqual({ overflow: false, outside: false });
    await page.screenshot({ path: `${directory}/compact-open-${width}-${theme}.png` });
    await card(page).getByLabel('Side categories', { exact: true }).check();
    await rows.nth(1).click();
    await card(page).getByLabel('Your answer', { exact: true }).fill('Fade them');
    await panel(page)
      .getByRole('button', { name: /^Send 2 answers/ })
      .press('Enter');
    await expectAnswered(rows.first(), 'Side categories');
    await expect(rows.nth(1)).toHaveAttribute('aria-expanded', 'false');
    await settled(page);
    await neutralText();
    await page.screenshot({ path: `${directory}/compact-answered-${width}-${theme}.png` });
    await rows.first().click();
    await expect(card(page).getByRole('button', { name: 'Change answer' })).toBeVisible();
    await page.screenshot({ path: `${directory}/compact-answered-open-${width}-${theme}.png` });
    expect(await inside()).toEqual({ overflow: false, outside: false });
  });
}
