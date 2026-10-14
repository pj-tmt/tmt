import { mkdirSync } from 'node:fs';
import { expect, test, type Page } from '@playwright/test';
import { text } from '../src/strings.js';
import type { ThreadView } from '../src/thread-records.js';
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

test('drafts survive tabs; one group Send keeps its tab and verified answers survive reload', async ({
  page,
}) => {
  const group = await mount(page);
  const tabs = group.getByRole('tab');
  await expect(group.locator('.tmt-ui-tabs-status')).toHaveText('0 of 2 answered');
  await tabs.nth(1).click();
  await activeCard(page).getByLabel('Your answer', { exact: true }).fill('Keep this draft 3');
  await tabs.nth(0).click();
  await tabs.nth(0).press('2');
  await expect(activeCard(page).getByLabel('Change it', { exact: true })).toBeChecked();
  await expect(group.locator('.tmt-ui-tabs-status')).toHaveText('2 of 2 answered');
  expect(await run(page, 'discussionProof')).toHaveLength(0);
  const send = group.getByRole('button', { name: 'Send 2 answers · to @Agent 1', exact: true });
  await expect(send).toHaveText('Send 2 answers');
  await send.evaluate((node) => (node as HTMLButtonElement).click());
  expect(await run(page, 'discussionProof')).toHaveLength(0);
  await send.press('Enter');
  await expect(tabs.nth(0)).toHaveAttribute('aria-selected', 'true');
  await expect(activeCard(page).locator('.question-answer')).toHaveText('Change it');
  const before: readonly ThreadView[] = await run(page, 'discussionProof');
  expect(before).toHaveLength(2);
  expect(before.every((thread) => thread.comments.length === 1)).toBe(true);
  expect(before[0].anchor?.exact).toBe('Which heading should we use?');
  expect(before[0].anchor?.suffix).toContain('Consider clarity');
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
  const attempts = await run(page, 'answerBatchProof');
  expect(attempts).toHaveLength(1);
  expect(attempts[0].selection.messageIds).toHaveLength(2);
  expect(attempts[0].selection.comment).toContain('Answers to 2 questions:');
  expect(attempts[0].selection.comment).toContain(
    'What should the introduction say?: Keep this draft 3',
  );
  const inactive = page.frameLocator('#ask-page-fixture iframe').locator('tmt-question').nth(1);
  await expect(inactive).not.toHaveAttribute('hidden');
  await expect(inactive).toHaveAttribute('data-colab-question-active', 'false');
  await run(page, 'saveQuestionPage');
  await page.reload();
  await run(page, 'mount');
  await run(page, 'restoreQuestionPage');
  const restored = page.locator('[data-question-group]').first();
  await expect(restored).not.toHaveAttribute('data-detached');
  await expect(restored.locator('.tmt-ui-tabs-status')).toHaveText('2 of 2 answered');
  expect((await run(page, 'proof')).sends).toHaveLength(0);
  await activeCard(page).getByRole('button', { name: 'Change answer' }).click();
  await activeCard(page).getByLabel('Keep it', { exact: true }).check();
  await restored.getByRole('button', { name: /^Send 1 answer/ }).click();
  await expect(restored.getByRole('tab').nth(0)).toHaveAttribute('aria-selected', 'true');
  await expect(activeCard(page).locator('.question-answer')).toHaveText('Keep it');
  const after: readonly ThreadView[] = await run(page, 'discussionProof');
  expect(after).toHaveLength(2);
  expect(after.find((thread) => thread.ref.id === before[0].ref.id)?.comments).toHaveLength(2);
  const changed = await run(page, 'answerBatchProof');
  expect(changed).toHaveLength(1);
  expect(changed[0].input.batch).toBeUndefined();
});

for (const [width, theme] of [
  [1440, 'light'],
  [390, 'dark'],
] as const)
  test(`reload keeps group Send pending until its default recipients resolve ${width} ${theme}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 1100 });
    let group = await mount(page, { agents: ['Agent 1', 'Agent 2'] });
    const captures = process.env.COLAB_QUESTION_CAPTURE_DIR;
    const capture = async (state: string) => {
      if (!captures) return;
      mkdirSync(captures, { recursive: true });
      await page.screenshot({ path: `${captures}/recipients-${state}-${width}-${theme}.png` });
    };
    await activeCard(page).getByLabel('Keep it', { exact: true }).check();
    await group
      .getByRole('button', { name: 'Send 1 answer · to @Agent 1, @Agent 2', exact: true })
      .click();
    await expect(activeCard(page).locator('.question-answer')).toHaveText('Keep it');
    await run(page, 'saveQuestionPage');
    await page.reload();
    await run(page, 'mount', { restoreQuestionPage: true, holdQuestionDirectory: true });
    group = page.locator('[data-question-group]').first();
    await expect(group).not.toHaveAttribute('data-detached');
    await (
      await pageAction(
        page.locator('#ask-page-fixture'),
        theme === 'dark' ? 'Theme: Dark' : 'Theme: Light',
      )
    ).click();
    await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
    await activeCard(page).getByRole('button', { name: 'Change answer' }).click();
    await activeCard(page).getByLabel('Change it', { exact: true }).check();
    const send = group.getByRole('button', { name: /^Send 1 answer/ });
    await expect(send).toBeDisabled();
    await expect(group.locator('.annotation-hint[role="status"]')).toHaveText(
      text.messageAgentsChecking,
    );
    expect(await run(page, 'discussionProof')).toHaveLength(1);
    expect((await run(page, 'proof')).sends).toHaveLength(0);
    await capture('pending');
    await run(page, 'finishQuestionDirectory');
    await expect(send).toHaveAccessibleName('Send 1 answer · to @Agent 1, @Agent 2');
    await expect(send).toBeEnabled();
    await capture('ready');
    await send.click();
    await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(2);
    const threads: readonly ThreadView[] = await run(page, 'discussionProof');
    expect(threads).toHaveLength(1);
    expect(threads[0].comments).toHaveLength(2);
  });

for (const outcome of ['empty', 'failed'] as const) {
  test(`resolved-${outcome} directory keeps intentional comment-only group Send`, async ({
    page,
  }) => {
    await page.goto('/');
    await run(page, 'mount', {
      holdQuestionDirectory: true,
      ...(outcome === 'empty' ? { agents: [] } : { agentsUnavailable: true }),
    });
    await run(page, 'questionGroup');
    const group = page.locator('[data-question-group]').first();
    await expect(group).not.toHaveAttribute('data-detached');
    await activeCard(page).getByLabel('Keep it', { exact: true }).check();
    const send = group.getByRole('button', { name: /^Send 1 answer/ });
    await expect(send).toBeDisabled();
    await run(page, 'finishQuestionDirectory');
    await expect(send).toBeEnabled();
    await expect(group.locator('.annotation-hint[role="status"]')).toHaveText(
      'Stays on the page, no one is asked',
    );
    await send.click();
    await expect(activeCard(page).locator('.question-answer')).toHaveText('Keep it');
    expect((await run(page, 'proof')).sends).toHaveLength(0);
    expect(await run(page, 'discussionProof')).toHaveLength(1);
  });
}

test('explicitly removing seeded recipients keeps comment-only Send', async ({ page }) => {
  const group = await mount(page, { agents: ['Agent 1', 'Agent 2'] });
  await activeCard(page).getByLabel('Keep it', { exact: true }).check();
  await group.getByRole('button', { name: 'Remove @Agent 1', exact: true }).click();
  await group.getByRole('button', { name: 'Remove @Agent 2', exact: true }).click();
  const send = group.getByRole('button', { name: 'Send 1 answer · to thread only', exact: true });
  await expect(send).toBeEnabled();
  await send.click();
  await expect(activeCard(page).locator('.question-answer')).toHaveText('Keep it');
  expect((await run(page, 'proof')).sends).toHaveLength(0);
});

test('single-question Enter cannot bypass pending recipient initialization', async ({ page }) => {
  await page.goto('/');
  await run(page, 'mount', { holdQuestionDirectory: true });
  await run(page, 'question', { kind: 'short' });
  const group = page.locator('[data-question-group]').first();
  await expect(group).not.toHaveAttribute('data-detached');
  const answer = activeCard(page).getByLabel('Your answer', { exact: true });
  await answer.fill('Keep it short');
  await answer.press('Enter');
  expect(await run(page, 'discussionProof')).toHaveLength(0);
  expect((await run(page, 'proof')).sends).toHaveLength(0);
  await expect(group.getByRole('button', { name: /^Send answer/ })).toBeDisabled();
  await run(page, 'finishQuestionDirectory');
  await expect(
    group.getByRole('button', { name: 'Send answer · to @Agent 1', exact: true }),
  ).toBeEnabled();
  await answer.press('Enter');
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
  expect(await run(page, 'discussionProof')).toHaveLength(1);
});

test('replacing a held directory cannot seed the replacement group', async ({ page }) => {
  await page.goto('/');
  await run(page, 'mount', { holdQuestionDirectory: true });
  await run(page, 'questionGroup');
  await expect(page.locator('[data-question-group]').first()).not.toHaveAttribute('data-detached');
  // Mount resolves the old barrier while retiring its binding. The replacement is empty.
  await run(page, 'mount', { agents: [] });
  await run(page, 'questionGroup');
  const group = page.locator('[data-question-group]').first();
  await expect(group).not.toHaveAttribute('data-detached');
  await activeCard(page).getByLabel('Keep it', { exact: true }).check();
  await expect(
    group.getByRole('button', { name: 'Send 1 answer · to thread only', exact: true }),
  ).toBeEnabled();
  await expect(group.getByRole('button', { name: /^Remove @Agent/ })).toHaveCount(0);
  expect((await run(page, 'proof')).sends).toHaveLength(0);
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
    group
      .getByRole('button', { name: /^Send (?:answer|\d+ answers?)/, includeHidden: true })
      .first(),
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
  await group.getByRole('button', { name: 'Add recipient', exact: true }).click();
  const recipient = page.getByRole('combobox', { name: 'Find an agent' });
  await recipient.press('3');
  await expect(recipient).toHaveValue('3');
  await recipient.press('Escape');
  await expect(tabs.nth(1)).toHaveAttribute('aria-selected', 'true');
  expect(await run(page, 'discussionProof')).toHaveLength(0);
});

test('a single-member group and a lone question have no tab strip', async ({ page }) => {
  await mount(page, { count: 1, lone: true });
  await expect(page.locator('[data-question-group]')).toHaveCount(2);
  await expect(page.getByRole('tab')).toHaveCount(0);
  await expect(page.locator('[data-question-group][data-detached]')).toHaveCount(0);
});

test('a successful local callback cannot supply a verified answer or change the tab', async ({
  page,
}) => {
  await page.goto('/');
  await run(page, 'mount', { holdQuestionView: true });
  await run(page, 'questionGroup');
  const group = page.locator('[data-question-group]');
  await expect(group).not.toHaveAttribute('data-detached');
  await activeCard(page).getByLabel('Keep it', { exact: true }).check();
  await group.getByRole('button', { name: /^Send (?:answer|\d+ answers?)/ }).click();
  await expect(
    activeCard(page).getByText('Answer recorded; waiting for the verified view.'),
  ).toBeVisible();
  await expect(group.getByRole('tab').nth(0)).toHaveAttribute('aria-selected', 'true');
  await expect(group.locator('.tmt-ui-tabs-status')).toHaveText('1 of 2 answered');
  expect(await run(page, 'discussionProof')).toHaveLength(1);
  await expect(activeCard(page).locator('.question-answer')).toHaveCount(0);
  await run(page, 'releaseQuestionView');
  await expect(group.getByRole('tab').nth(0)).toHaveAttribute('aria-selected', 'true');
  await expect(group.locator('.tmt-ui-tabs-status')).toHaveText('1 of 2 answered');
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
  await group.getByRole('button', { name: /^Send (?:answer|\d+ answers?)/ }).click();
  await expect(
    activeCard(page).getByText('Answer recorded; waiting for the verified view.'),
  ).toBeVisible();
  await group.getByRole('tab').nth(2).click();
  await run(page, 'releaseQuestionView');
  await expect(group.locator('.tmt-ui-tabs-status')).toHaveText('1 of 3 answered');
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
              const shell = await group.locator('.question-group-card').boundingBox();
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
          const captureStates = process.env.COLAB_GROUP_CAPTURE_STATES?.split(',');
          if (captures && (!captureStates || captureStates.includes(state))) {
            mkdirSync(captures, { recursive: true });
            await page.screenshot({
              fullPage: true,
              path: `${captures}/groups-${state}-${width}-${theme}-page-${darkPage ? 'dark' : 'light'}.png`,
            });
          }
        };
        await capture('waiting');
        await activeCard(page).getByLabel('Keep it', { exact: true }).check();
        await expect(
          group.getByRole('button', { name: /^Send (?:answer|\d+ answers?)/ }),
        ).toHaveCSS('background-color', 'rgb(0, 107, 143)');
        await group.getByRole('button', { name: /^Send (?:answer|\d+ answers?)/ }).click();
        await expect(group.getByRole('tab').nth(0)).toHaveAttribute('aria-selected', 'true');
        await expect(
          page.frameLocator('#ask-page-fixture iframe').locator('[data-colab-thread]'),
        ).toHaveCount(1);
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
            const tail = await frame
              .locator('tmt-question')
              .first()
              .locator('[data-question]')
              .evaluate((heading) => {
                const range = document.createRange();
                range.selectNodeContents(heading);
                const rects = [...range.getClientRects()].filter(
                  (rect) => rect.width > 0 && rect.height > 0,
                );
                const last = rects.at(-1);
                return last ? { top: last.top, right: last.right } : null;
              });
            const canvas = await page.locator('#ask-page-fixture iframe').boundingBox();
            // The numbered marker is raised at the selection's last line, not its start.
            return (
              !!marker &&
              !!tail &&
              !!canvas &&
              marker.y < canvas.y + tail.top &&
              Math.abs(marker.y + marker.height - (canvas.y + tail.top)) <= 8 &&
              marker.x <= canvas.x + tail.right &&
              marker.x + marker.width >= canvas.x + tail.right - 8
            );
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

// #3069: browser DOM is the independent oracle, not the slot claims used by chrome.
for (const width of [1440, 1024, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`author container bounds ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 800 });
      for (const layout of ['centered', 'nested'] as const) {
        const group = await mount(page, { layout, darkPage: theme === 'dark' });
        if (theme === 'dark')
          await (await pageAction(page.locator('#ask-page-fixture'), 'Theme: Dark')).click();
        const container = page
          .frameLocator('#ask-page-fixture iframe')
          .locator('#author-container');
        await expect
          .poll(
            async () => {
              const content = await container.boundingBox();
              const card = await group.locator('.question-group-card').boundingBox();
              return (
                !!content &&
                !!card &&
                Math.abs(card.x - content.x) <= 1 &&
                Math.abs(card.width - content.width) <= 1
              );
            },
            { message: 'question card must use the real author container edges' },
          )
          .toBe(true);
        expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
          true,
        );
        expect(
          await container.evaluate(() => document.documentElement.scrollWidth <= innerWidth),
        ).toBe(true);
      }
    });
    test(`footer clears floating controls ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 800 });
      const group = await mount(page, { layout: 'centered', darkPage: theme === 'dark' });
      if (theme === 'dark')
        await (await pageAction(page.locator('#ask-page-fixture'), 'Theme: Dark')).click();
      await activeCard(page).getByLabel('Keep it', { exact: true }).check();
      const send = group.getByRole('button', { name: /^Send 1 answer/ });
      await expect(send).toBeEnabled();
      await send.evaluate((node) =>
        window.scrollBy(0, node.getBoundingClientRect().bottom - innerHeight + 30),
      );
      const floating = page.locator('#ask-page-fixture .selection-mode');
      await expect
        .poll(
          async () => {
            const fab = await floating.boundingBox();
            const controls = await group
              .locator('.question-option, .question-group-footer button')
              .all();
            const boxes = await Promise.all(controls.map((control) => control.boundingBox()));
            return (
              !!fab &&
              boxes.every(
                (box) =>
                  !box ||
                  fab.x >= box.x + box.width ||
                  fab.x + fab.width <= box.x ||
                  fab.y >= box.y + box.height ||
                  fab.y + fab.height <= box.y,
              )
            );
          },
          { message: 'FAB must not cover answers, Send or recipient controls' },
        )
        .toBe(true);
      const captures = process.env.COLAB_QUESTION_CAPTURE_DIR;
      if (captures) {
        mkdirSync(captures, { recursive: true });
        await page.screenshot({ path: `${captures}/container-footer-${width}-${theme}.png` });
      }
      // A trusted browser click must reach the recipient control, not chrome above it.
      await group.getByRole('button', { name: 'Add recipient', exact: true }).click();
      await expect(page.getByRole('combobox', { name: 'Find an agent' })).toBeVisible();
      await page.getByRole('combobox', { name: 'Find an agent' }).press('Escape');
      await send.click();
      await expect(activeCard(page).locator('.question-answer')).toHaveText('Keep it');
    });
  }

for (const width of [1440, 390])
  test(`footer placement follows resize, size changes and group removal ${width}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 800 });
    await mount(page, {
      layout: 'centered',
      multiple: true,
      lone: true,
      agents: ['Agent 1', 'Agent 2'],
    });
    const groups = page.locator('#ask-page-fixture [data-question-group]');
    await expect(groups).toHaveCount(3);
    const floating = page.locator('#ask-page-fixture .selection-mode');
    const frame = page.frameLocator('#ask-page-fixture iframe');
    const check = async () => {
      await expect
        .poll(async () => {
          const fab = await floating.boundingBox();
          if (!fab || !(await floating.isVisible())) return true;
          const footers = await groups.locator('.question-group-footer').all();
          for (const footer of footers) {
            if (!(await footer.isVisible())) continue;
            const box = await footer.locator('..').boundingBox();
            if (
              box &&
              fab.x < box.x + box.width &&
              fab.x + fab.width > box.x &&
              fab.y < box.y + box.height &&
              fab.y + fab.height > box.y
            )
              return false;
          }
          return true;
        })
        .toBe(true);
    };
    const place = async (index: number) => {
      const footer = groups.nth(index).locator('.question-group-footer');
      await footer.evaluate((node) =>
        window.scrollBy(0, node.getBoundingClientRect().bottom - innerHeight + 30),
      );
      await check();
    };
    await expect(
      page.locator('#ask-page-fixture [data-question-group][data-detached]'),
    ).toHaveCount(0);
    // Disabled Send still shares its footer with enabled recipient controls.
    await expect(groups.nth(1).getByRole('button', { name: /^Send answers/ })).toBeDisabled();
    await place(1);
    await groups.nth(1).getByRole('button', { name: 'Remove @Agent 2', exact: true }).click();
    await check();
    await page.setViewportSize({ width: width === 390 ? 1024 : 390, height: 800 });
    await place(0);
    const container = await frame.locator('#author-container').boundingBox();
    await expect
      .poll(async () => {
        const card = await groups.nth(0).locator('.question-group-card').boundingBox();
        return (
          !!card &&
          !!container &&
          Math.abs(card.x - container.x) <= 1 &&
          Math.abs(card.width - container.width) <= 1
        );
      })
      .toBe(true);
    await frame
      .locator('tmt-questions')
      .nth(0)
      .evaluate((node) => {
        (node as HTMLElement).hidden = true;
      });
    await expect(groups.nth(0)).toHaveAttribute('data-detached', 'true');
    await place(1);
    await frame
      .locator('tmt-questions')
      .nth(1)
      .evaluate((node) => node.remove());
    await expect(groups.nth(1)).toHaveAttribute('data-detached', 'true');
    await place(2);
    await page.evaluate(() => window.scrollTo(0, 0));
    await expect(floating).toBeVisible();
    await check();
  });
