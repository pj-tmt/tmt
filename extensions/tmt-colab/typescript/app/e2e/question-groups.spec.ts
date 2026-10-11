import { mkdirSync } from 'node:fs';
import { expect, test, type Page } from '@playwright/test';
import { pageAction } from '../test/page-actions.js';

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
  await run(page, 'questionGroup', options);
  const group = page.locator('[data-question-group]').first();
  await expect(group).not.toHaveAttribute('data-detached');
  return group;
}
const activeCard = (page: Page) => page.locator('.question-card:visible').first();

test('drafts survive tab switches; verified answers advance and survive reload with the same quote', async ({
  page,
}) => {
  const group = await mount(page);
  const tabs = group.getByRole('tab');
  await expect(tabs.nth(0)).toHaveAttribute('aria-selected', 'true');
  await expect(group.locator('.tmt-ui-tabs-status')).toHaveText('0/2 answered');
  await tabs.nth(1).click();
  await activeCard(page).getByLabel('Your answer', { exact: true }).fill('Keep this draft 3');
  await tabs.nth(0).click();
  await tabs.nth(1).click();
  await expect(activeCard(page).getByLabel('Your answer', { exact: true })).toHaveValue(
    'Keep this draft 3',
  );
  await tabs.nth(0).click();
  await tabs.nth(0).press('2');
  await expect(activeCard(page).getByLabel('Change it', { exact: true })).toBeChecked();
  const send = activeCard(page).getByRole('button', {
    name: 'Send answer · to @Agent 1',
    exact: true,
  });
  await expect(send).toHaveText('Send & next');
  await send.evaluate((node) => (node as HTMLButtonElement).click());
  expect(await run(page, 'discussionProof')).toHaveLength(0);
  await send.press('Enter');
  await expect(tabs.nth(1)).toHaveAttribute('aria-selected', 'true');
  await expect(tabs.nth(0)).toHaveAccessibleName(/Q1.*Answered/);
  await expect(group.locator('.tmt-ui-tabs-status')).toHaveText('1/2 answered');
  await expect(activeCard(page).getByRole('button', { name: /^Send answer/ })).toHaveText(
    'Send answer',
  );
  const before = await run(page, 'discussionProof');
  expect(before).toHaveLength(1);
  expect(before[0].anchor.exact).toBe('Which heading should we use?');
  // The hidden authored question remains in canonical context; its UUID is never quote text.
  expect(before[0].anchor.suffix).toContain('Consider clarity');
  const inactive = page.frameLocator('#ask-page-fixture iframe').locator('tmt-question').first();
  await expect(inactive).not.toHaveAttribute('hidden');
  await expect(inactive).toHaveAttribute('data-colab-question-active', 'false');
  await run(page, 'saveQuestionPage');
  await page.reload();
  await run(page, 'mount');
  await run(page, 'restoreQuestionPage');
  const restored = page.locator('[data-question-group]').first();
  await expect(restored).not.toHaveAttribute('data-detached');
  await expect(restored.getByRole('tab').nth(1)).toHaveAttribute('aria-selected', 'true');
  await expect(restored.locator('.tmt-ui-tabs-status')).toHaveText('1/2 answered');
  await restored.getByRole('tab').nth(0).click();
  await expect(activeCard(page).locator('.question-answer')).toHaveText('Change it');
  expect((await run(page, 'discussionProof'))[0].anchor).toEqual(before[0].anchor);
  await activeCard(page).getByRole('button', { name: 'Change answer' }).click();
  await activeCard(page).getByLabel('Keep it', { exact: true }).check();
  await activeCard(page)
    .getByRole('button', { name: /^Send answer/ })
    .click();
  await expect(restored.getByRole('tab').nth(1)).toHaveAttribute('aria-selected', 'true');
  const after = await run(page, 'discussionProof');
  expect(after).toHaveLength(1);
  expect(after[0].ref).toEqual(before[0].ref);
  expect(after[0].comments).toHaveLength(2);
  expect(after[0].anchor).toEqual(before[0].anchor);
});

test('tab navigation and digits preserve normal typing and long-answer Enter', async ({ page }) => {
  const group = await mount(page, { count: 3 });
  const tabs = group.getByRole('tab');
  const before = await group.locator('.tmt-ui-tabs-header').boundingBox();
  const frame = page.frameLocator('#ask-page-fixture iframe');
  await frame.locator('body').evaluate(() => {
    const original = MessagePort.prototype.postMessage;
    const held: Array<() => void> = [];
    MessagePort.prototype.postMessage = function (message) {
      const send = () => original.call(this, message);
      if (message?.type === 'colab.render.slots') held.push(send);
      else send();
    };
    Object.assign(window, {
      releaseGroupSlots: () => {
        MessagePort.prototype.postMessage = original;
        held.forEach((send) => send());
      },
    });
  });
  await tabs.nth(0).press('End');
  await expect(tabs.nth(2)).toBeFocused();
  await expect(tabs.nth(2)).toHaveAttribute('aria-selected', 'true');
  await expect(group).toHaveAttribute('data-detached', 'true');
  await expect(group.locator('.tmt-ui-tabs-header')).toBeVisible();
  await expect(group.locator('.question-controls:visible')).toHaveCount(0);
  const pending = await group.locator('.tmt-ui-tabs-header').boundingBox();
  expect(Math.abs(pending!.y - before!.y)).toBeLessThanOrEqual(1);
  // Placement gates Send, not trusted local draft choices from the visible strip.
  await tabs.nth(2).press('Home');
  await tabs.nth(0).press('2');
  await expect(group.getByLabel('Change it', { exact: true })).toBeChecked();
  await expect(
    group.getByRole('button', { name: /^Send answer/, includeHidden: true }).first(),
  ).toBeDisabled();
  expect(await run(page, 'discussionProof')).toHaveLength(0);
  await tabs.nth(0).press('End');
  await frame.locator('body').evaluate(() => {
    (window as unknown as { releaseGroupSlots(): void }).releaseGroupSlots();
  });
  await expect(group).not.toHaveAttribute('data-detached');
  await expect(tabs.nth(2)).toBeFocused();
  await activeCard(page).getByLabel('Your answer', { exact: true }).fill('Line 3');
  await activeCard(page).getByLabel('Your answer', { exact: true }).press('Enter');
  await expect(activeCard(page).getByLabel('Your answer', { exact: true })).toHaveValue('Line 3\n');
  expect(await run(page, 'discussionProof')).toHaveLength(0);
  await tabs.nth(2).press('ArrowRight');
  await expect(tabs.nth(0)).toBeFocused();
  await tabs.nth(0).press('ArrowRight');
  const input = activeCard(page).getByLabel('Your answer', { exact: true });
  await input.fill('Text ');
  await expect(input).toBeFocused();
  await input.press('3');
  await expect(input).toHaveValue('Text 3');
  const recipient = activeCard(page).getByRole('combobox', { name: 'Answer recipient' });
  await recipient.press('End');
  await recipient.press('3');
  await expect(recipient).toContainText('3');
  await expect(tabs.nth(1)).toHaveAttribute('aria-selected', 'true');
  expect(await run(page, 'discussionProof')).toHaveLength(0);
});

test('a single-member group and a lone question have no tab strip', async ({ page }) => {
  await mount(page, { count: 1, lone: true });
  await expect(page.locator('[data-question-group]')).toHaveCount(2);
  await expect(page.getByRole('tab')).toHaveCount(0);
  await expect(page.locator('[data-question-group][data-detached]')).toHaveCount(0);
});

test('a successful local callback cannot mark or advance before the verified view', async ({
  page,
}) => {
  await page.goto('/');
  await run(page, 'mount', { holdQuestionView: true });
  await run(page, 'questionGroup');
  const group = page.locator('[data-question-group]');
  await expect(group).not.toHaveAttribute('data-detached');
  await activeCard(page).getByLabel('Keep it', { exact: true }).check();
  await activeCard(page)
    .getByRole('button', { name: /^Send answer/ })
    .click();
  await expect(
    activeCard(page).getByText('Answer recorded; waiting for the verified view.'),
  ).toBeVisible();
  await expect(group.getByRole('tab').nth(0)).toHaveAttribute('aria-selected', 'true');
  await expect(group.locator('.tmt-ui-tabs-status')).toHaveText('0/2 answered');
  expect(await run(page, 'discussionProof')).toHaveLength(1);
  await run(page, 'releaseQuestionView');
  await expect(group.getByRole('tab').nth(1)).toHaveAttribute('aria-selected', 'true');
  await expect(group.locator('.tmt-ui-tabs-status')).toHaveText('1/2 answered');
});

test('a delayed verified answer preserves a tab the reader explicitly selected meanwhile', async ({
  page,
}) => {
  await page.goto('/');
  await run(page, 'mount', { holdQuestionView: true });
  await run(page, 'questionGroup', { count: 3 });
  const group = page.locator('[data-question-group]');
  await expect(group).not.toHaveAttribute('data-detached');
  await activeCard(page).getByLabel('Keep it', { exact: true }).check();
  await activeCard(page)
    .getByRole('button', { name: /^Send answer/ })
    .click();
  await expect(
    activeCard(page).getByText('Answer recorded; waiting for the verified view.'),
  ).toBeVisible();
  await group.getByRole('tab').nth(2).click();
  await run(page, 'releaseQuestionView');
  await expect(group.locator('.tmt-ui-tabs-status')).toHaveText('1/3 answered');
  await expect(group.getByRole('tab').nth(2)).toHaveAttribute('aria-selected', 'true');
  expect(await run(page, 'discussionProof')).toHaveLength(1);
});

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'])
    for (const darkPage of [false, true]) {
      test(`group look ${width} ${theme} page-${darkPage ? 'dark' : 'light'}`, async ({ page }) => {
        await page.setViewportSize({ width, height: 1100 });
        const group = await mount(page, { darkPage });
        if (theme === 'dark')
          await (await pageAction(page.locator('#ask-page-fixture'), 'Theme: Dark')).click();
        const heading = page
          .frameLocator('#ask-page-fixture iframe')
          .locator('tmt-questions > [data-title]');
        await expect(heading).toHaveCSS('font-size', '22px');
        await expect(heading).toHaveCSS(
          'color',
          darkPage ? 'rgb(238, 238, 238)' : 'rgb(34, 34, 34)',
        );
        await expect(activeCard(page).locator('.question-controls')).toHaveCSS(
          'background-color',
          'rgb(255, 255, 255)',
        );
        const captures = process.env.COLAB_QUESTION_CAPTURE_DIR;
        const capture = async (state: string) => {
          await expect(group).not.toHaveAttribute('data-detached');
          await expect
            .poll(async () => {
              const source = await page
                .frameLocator('#ask-page-fixture iframe')
                .locator('tmt-questions > tmt-question[data-colab-question-active="true"]')
                .boundingBox();
              const gap = await activeCard(page)
                .locator('.question-description-space')
                .boundingBox();
              const shell = await group.locator('.tmt-ui-tabs,.question-group-card').boundingBox();
              return (
                !!source &&
                !!gap &&
                !!shell &&
                Math.abs(source.y - gap.y) <= 2 &&
                Math.abs(source.height - gap.height) <= 1 &&
                Math.abs(source.x - shell.x) <= 1 &&
                Math.abs(source.width - shell.width) <= 1
              );
            })
            .toBe(true);
          if (captures) {
            mkdirSync(captures, { recursive: true });
            await page.screenshot({
              path: `${captures}/groups-${state}-${width}-${theme}-page-${darkPage ? 'dark' : 'light'}.png`,
            });
          }
        };
        await capture('waiting');
        await activeCard(page).getByLabel('Keep it', { exact: true }).check();
        await expect(activeCard(page).getByRole('button', { name: /^Send answer/ })).toHaveCSS(
          'background-color',
          'rgb(0, 107, 143)',
        );
        await activeCard(page)
          .getByRole('button', { name: /^Send answer/ })
          .click();
        await expect(group.getByRole('tab').nth(1)).toHaveAttribute('aria-selected', 'true');
        await expect(
          page.frameLocator('#ask-page-fixture iframe').locator('[data-colab-thread]'),
        ).toHaveCount(0);
        await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
        await run(page, 'questionAsk', 'replied');
        await group.getByRole('tab').nth(0).click();
        await expect(activeCard(page).getByTestId('ask-reply')).toContainText(
          'I read your answer in this thread.',
        );
        await expect(activeCard(page).locator('.question-state')).toContainText('You answered');
        await expect
          .poll(async () => {
            const frame = page.frameLocator('#ask-page-fixture iframe');
            const marker = await frame.locator('[data-colab-thread]').boundingBox();
            const heading = await frame
              .locator('tmt-question')
              .first()
              .locator('[data-question]')
              .boundingBox();
            return !!marker && !!heading && Math.abs(marker.y - heading.y) <= 8;
          })
          .toBe(true);
        await expect
          .poll(async () => {
            const box = await group.boundingBox(),
              below = await page
                .frameLocator('#ask-page-fixture iframe')
                .locator('#after-question')
                .boundingBox();
            return !!box && !!below && below.y >= box.y + box.height - 1;
          })
          .toBe(true);
        await capture('answered');
        await run(page, 'questionGroup', { count: 12, darkPage, longLabels: true, lone: true });
        await expect(group.getByRole('tab')).toHaveCount(12);
        await group.getByRole('tab').nth(0).press('End');
        await expect(group.getByRole('tab').nth(11)).toHaveAttribute('aria-selected', 'true');
        await expect(
          page
            .frameLocator('#ask-page-fixture iframe')
            .locator(
              'tmt-questions > tmt-question[data-colab-question-active="true"] > [data-question]',
            ),
        ).toHaveText('What should section 12 explain?');
        await expect
          .poll(() =>
            group
              .getByRole('tab')
              .nth(11)
              .evaluate((tab) => {
                const box = tab.getBoundingClientRect(),
                  strip = tab.parentElement!.getBoundingClientRect();
                return box.left >= strip.left - 1 && box.right <= strip.right + 1;
              }),
          )
          .toBe(true);
        await capture('overflow-lone');
        await run(page, 'questionGroup', { count: 1, darkPage });
        await expect(group.getByRole('tab')).toHaveCount(0);
        await capture('one-member');
      });
    }
