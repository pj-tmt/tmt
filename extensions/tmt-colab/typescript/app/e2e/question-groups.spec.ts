import { mkdirSync } from 'node:fs';
import { expect, test, type Page } from '@playwright/test';
import { text } from '../src/strings.js';
import type { ThreadView } from '../src/thread-records.js';
import { pageAction } from '../test/page-actions.js';
import { rgb, tokens } from './tokens.js';

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
  await group.locator('.question-line').first().click();
  return group;
}
const activeCard = (page: Page) => page.locator('.question-card:visible').first();

test('drafts survive disclosure changes; one group Send and verified answers survive reload', async ({
  page,
}) => {
  const group = await mount(page);
  const rows = group.locator('.question-line');
  await expect(group.locator('.question-compact-heading')).toHaveText(
    'Questions from Agent 1 · 2 of 2 waiting',
  );
  await rows.nth(1).click();
  await activeCard(page).getByLabel('Your answer', { exact: true }).fill('Keep this draft 3');
  await rows.first().click();
  await activeCard(page).getByLabel('Change it', { exact: true }).focus();
  await activeCard(page).getByLabel('Change it', { exact: true }).press('Space');
  await expect(activeCard(page).getByLabel('Change it', { exact: true })).toBeChecked();
  // Local drafts are not answers and do not clear the waiting count.
  await expect(group.locator('.question-compact-heading')).toHaveText(
    'Questions from Agent 1 · 2 of 2 waiting',
  );
  expect(await run(page, 'discussionProof')).toHaveLength(0);
  const send = group.getByRole('button', { name: 'Send 2 answers · to @Agent 1', exact: true });
  await expect(send).toHaveText('Send 2 answers');
  await send.evaluate((node) => (node as HTMLButtonElement).click());
  expect(await run(page, 'discussionProof')).toHaveLength(0);
  await send.press('Enter');
  await expect(rows.first()).toHaveAttribute('aria-expanded', 'false');
  await expect(rows.first()).toContainText('Answered: Change it');
  await expect(rows.nth(1)).toContainText('Answered: Keep this draft 3');
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
  const authored = page.frameLocator('#ask-page-fixture iframe').locator('tmt-question').nth(1);
  await expect(authored).not.toHaveAttribute('hidden');
  await expect(authored).toHaveAttribute('data-colab-question-active', 'false');
  await run(page, 'saveQuestionPage');
  await page.reload();
  await run(page, 'mount');
  await run(page, 'restoreQuestionPage');
  const restored = page.locator('[data-question-group]').first();
  await expect(restored).not.toHaveAttribute('data-detached');
  await expect(restored.locator('.question-compact-heading')).toHaveText(
    'Questions from Agent 1 · 0 of 2 waiting',
  );
  expect((await run(page, 'proof')).sends).toHaveLength(0);
  await restored.locator('.question-line').first().click();
  await activeCard(page).getByRole('button', { name: 'Change answer' }).click();
  await activeCard(page).getByLabel('Keep it', { exact: true }).check();
  await restored.getByRole('button', { name: /^Send 1 answer/ }).click();
  await expect(restored.locator('.question-line').first()).toContainText('Answered: Keep it');
  await expect(restored.locator('.question-line').first()).toHaveAttribute(
    'aria-expanded',
    'false',
  );
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
    await expect(group.locator('.question-line').first()).toContainText('Answered: Keep it');
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
    await group.locator('.question-line').first().click();
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
    await group.locator('.question-line').first().click();
    await activeCard(page).getByLabel('Keep it', { exact: true }).check();
    const send = group.getByRole('button', { name: /^Send 1 answer/ });
    await expect(send).toBeDisabled();
    await run(page, 'finishQuestionDirectory');
    await expect(send).toBeEnabled();
    await expect(group.locator('.annotation-hint[role="status"]')).toHaveText(
      'Stays on the page, no one is asked',
    );
    await send.click();
    await expect(group.locator('.question-line').first()).toContainText('Answered: Keep it');
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
  await expect(group.locator('.question-line').first()).toContainText('Answered: Keep it');
  expect((await run(page, 'proof')).sends).toHaveLength(0);
});

test('single-question Enter cannot bypass pending recipient initialization', async ({ page }) => {
  await page.goto('/');
  await run(page, 'mount', { holdQuestionDirectory: true });
  await run(page, 'question', { kind: 'short' });
  const group = page.locator('[data-question-group]').first();
  await expect(group).not.toHaveAttribute('data-detached');
  await group.locator('.question-line').first().click();
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
  await group.locator('.question-line').first().click();
  await activeCard(page).getByLabel('Keep it', { exact: true }).check();
  await expect(
    group.getByRole('button', { name: 'Send 1 answer · to thread only', exact: true }),
  ).toBeEnabled();
  await expect(group.getByRole('button', { name: /^Remove @Agent/ })).toHaveCount(0);
  expect((await run(page, 'proof')).sends).toHaveLength(0);
});

test('disclosures require current placement while digits preserve typing and long-answer Enter', async ({
  page,
}) => {
  const group = await mount(page, { count: 3 });
  const rows = group.locator('.question-line');
  // Digits in trusted controls retain their existing local-choice behavior.
  await activeCard(page).focus();
  await page.keyboard.press('2');
  await expect(activeCard(page).getByLabel('Change it', { exact: true })).toBeChecked();
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
  await rows.nth(2).click();
  // Compact rows share a physical block; changing its disclosure alone keeps
  // valid placement. Narrowing retires the old column until the held reply.
  await page.setViewportSize({ width: 390, height: 900 });
  await expect(group).toHaveAttribute('data-detached', 'true');
  await expect(
    group
      .getByRole('button', { name: /^Send (?:answer|\d+ answers?)/, includeHidden: true })
      .first(),
  ).toBeDisabled();
  expect(await run(page, 'discussionProof')).toHaveLength(0);
  await frame.locator('body').evaluate(() => {
    (window as unknown as { releaseGroupSlots(): void }).releaseGroupSlots();
  });
  await expect(group).not.toHaveAttribute('data-detached');
  const long = activeCard(page).getByLabel('Your answer', { exact: true });
  await long.fill('Line 3');
  await long.press('Enter');
  await expect(long).toHaveValue('Line 3\n');
  expect(await run(page, 'discussionProof')).toHaveLength(0);
  await rows.nth(2).focus();
  await rows.nth(2).press('ArrowDown');
  await expect(rows.first()).toBeFocused();
  await rows.first().press('Enter');
  await expect(activeCard(page).getByLabel('Change it', { exact: true })).toBeChecked();
  await rows.nth(1).click();
  const input = activeCard(page).getByLabel('Your answer', { exact: true });
  await input.fill('Text ');
  await input.press('3');
  await expect(input).toHaveValue('Text 3');
  await group.getByRole('button', { name: 'Add recipient', exact: true }).click();
  const recipient = page.getByRole('combobox', { name: 'Find an agent' });
  await recipient.press('3');
  await expect(recipient).toHaveValue('3');
  await recipient.press('Escape');
  expect((await run(page, 'proof')).sends).toHaveLength(0);
});

test('a single-member group and a lone question have no tab strip', async ({ page }) => {
  await mount(page, { count: 1, lone: true });
  await expect(page.locator('[data-question-group]')).toHaveCount(2);
  await expect(page.getByRole('tab')).toHaveCount(0);
  await expect(page.locator('[data-question-group][data-detached]')).toHaveCount(0);
});

test('a successful local callback cannot supply a verified answer or close the question', async ({
  page,
}) => {
  await page.goto('/');
  await run(page, 'mount', { holdQuestionView: true });
  await run(page, 'questionGroup');
  const group = page.locator('[data-question-group]');
  await expect(group).not.toHaveAttribute('data-detached');
  const rows = group.locator('.question-line');
  await rows.first().click();
  await activeCard(page).getByLabel('Keep it', { exact: true }).check();
  await group.getByRole('button', { name: /^Send (?:answer|\d+ answers?)/ }).click();
  // The answer stands at once on its folded line as an echo that no record backs: the count does
  // not move, and no status step is spelled out.
  await expect(rows.first()).toContainText('Answered: Keep it');
  await expect(rows.first()).toHaveAttribute('aria-expanded', 'false');
  await expect(rows.first()).toHaveAttribute('aria-disabled', 'true');
  await expect(page.getByText(/Sending…|waiting for the verified view/)).toHaveCount(0);
  await expect(group.locator('.question-compact-heading')).toHaveText(
    'Questions from Agent 1 · 2 of 2 waiting',
  );
  expect(await run(page, 'discussionProof')).toHaveLength(1);
  await run(page, 'releaseQuestionView');
  await expect(group.locator('.question-compact-heading')).toHaveText(
    'Questions from Agent 1 · 1 of 2 waiting',
  );
  await expect(rows.first()).toContainText('Answered: Keep it');
  await expect(rows.first()).not.toHaveAttribute('aria-disabled');
  await expect(rows.first()).toHaveAttribute('aria-expanded', 'false');
  await expect(group.locator('.question-compact-heading')).toHaveText(
    'Questions from Agent 1 · 1 of 2 waiting',
  );
});

test('a delayed verified answer preserves a question the reader explicitly opened meanwhile', async ({
  page,
}) => {
  await page.goto('/');
  await run(page, 'mount', { holdQuestionView: true });
  await run(page, 'questionGroup', { count: 3 });
  const group = page.locator('[data-question-group]');
  await expect(group).not.toHaveAttribute('data-detached');
  const rows = group.locator('.question-line');
  await rows.first().click();
  await activeCard(page).getByLabel('Keep it', { exact: true }).check();
  await group.getByRole('button', { name: /^Send (?:answer|\d+ answers?)/ }).click();
  await expect(rows.first()).toHaveAttribute('aria-disabled', 'true');
  await expect(rows.first()).toContainText('Answered: Keep it');
  await rows.nth(2).click();
  await activeCard(page).getByLabel('Your answer', { exact: true }).fill('A newer local draft.');
  await run(page, 'releaseQuestionView');
  await expect(rows.first()).toContainText('Answered: Keep it');
  await expect(group.locator('.question-compact-heading')).toHaveText(
    'Questions from Agent 1 · 2 of 3 waiting',
  );
  await expect(rows.nth(2)).toHaveAttribute('aria-expanded', 'true');
  await expect(activeCard(page).getByLabel('Your answer', { exact: true })).toHaveValue(
    'A newer local draft.',
  );
  expect(await run(page, 'discussionProof')).toHaveLength(1);
});

test('an echo on the folded line is replaced in place by the verified answer, nothing moved', async ({
  page,
}) => {
  await page.goto('/');
  await run(page, 'mount', { holdQuestionView: true });
  await run(page, 'questionGroup');
  const group = page.locator('[data-question-group]');
  const rows = group.locator('.question-line');
  await rows.first().click();
  const card = activeCard(page);
  await card.getByLabel('Keep it', { exact: true }).check();
  // An idle card has no status line: only a failure takes one.
  await expect(card.locator('.question-status')).toHaveCount(0);
  await group.getByRole('button', { name: /^Send (?:answer|\d+ answers?)/ }).click();
  // Send folds the card to its answered line; focus stays on that line, not the page.
  await expect(rows.first()).toContainText('Answered: Keep it');
  await expect(rows.first()).toHaveAttribute('aria-expanded', 'false');
  await expect(rows.first()).toBeFocused();
  await expect(page.locator('.question-card:visible')).toHaveCount(0);
  // Unconfirmed past 0.4 s, the line's mark turns to a spinner with no words; no button turns busy.
  const mark = rows.first().locator('.question-line-mark .comment-echo-mark');
  await expect(mark).toHaveCSS('visibility', 'visible');
  await expect(mark).toHaveText('');
  await expect(rows.first().locator('.question-line-mark')).toHaveAttribute('aria-hidden', 'true');
  const measure = () =>
    page.evaluate(() => {
      const box = (selector: string) => {
        const rect = document.querySelector(selector)!.getBoundingClientRect();
        return { top: rect.top, height: rect.height };
      };
      return { slot: box('[data-inline-slot]'), line: box('.question-line') };
    });
  const echoed = await measure();
  await run(page, 'releaseQuestionView');
  await expect(group.locator('.question-compact-heading')).toHaveText(
    'Questions from Agent 1 · 1 of 2 waiting',
  );
  await expect(rows.first()).not.toHaveAttribute('aria-disabled');
  await expect(rows.first().locator('.comment-echo-mark')).toHaveCount(0);
  await expect(rows.first()).toContainText('Answered: Keep it');
  expect(await measure()).toEqual(echoed);
  await rows.first().click();
  await expect(card.locator('.question-answer > p')).toHaveText('Keep it');
  await expect(card.locator('.question-status')).toHaveCount(0);
});

test('an answer verified within 0.4 s never shows the pending mark', async ({ page }) => {
  await page.goto('/');
  await run(page, 'mount');
  await run(page, 'questionGroup');
  const group = page.locator('[data-question-group]');
  const rows = group.locator('.question-line');
  await rows.first().click();
  await page.evaluate(() => {
    const seen = { mark: false };
    (window as unknown as { markSeen: typeof seen }).markSeen = seen;
    new MutationObserver(() => {
      if (document.querySelector('.question-line .comment-echo-mark[data-shown]')) seen.mark = true;
    }).observe(document.body, { subtree: true, childList: true, attributes: true });
  });
  await activeCard(page).getByLabel('Keep it', { exact: true }).check();
  await group.getByRole('button', { name: /^Send (?:answer|\d+ answers?)/ }).click();
  await expect(rows.first()).toContainText('Answered: Keep it');
  await page.waitForTimeout(600);
  expect(
    await page.evaluate(() => (window as unknown as { markSeen: { mark: boolean } }).markSeen.mark),
  ).toBe(false);
});

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'])
    for (const darkPage of [false, true]) {
      test(`group look ${width} ${theme} page-${darkPage ? 'dark' : 'light'}`, async ({ page }) => {
        await page.setViewportSize({ width, height: 1100 });
        const group = await mount(page, { darkPage });
        if (theme === 'dark')
          await (await pageAction(page.locator('#ask-page-fixture'), 'Theme: Dark')).click();
        const block = group.locator('.question-compact-block');
        const rows = group.locator('.question-line');
        // The block is Colab's layer over the author's page: the sheet of the scheme the page paints
        // (#4081), whatever the app theme, with a light rule; what it holds adds no surface of its own.
        const scheme = darkPage ? 'dark' : 'light';
        await expect(block).toHaveCSS('outline-width', '1px');
        await expect(
          page.locator('.page-cards', { has: page.locator('[data-question-group]') }).first(),
        ).toHaveAttribute('data-theme', scheme);
        const ink = rgb(tokens.browser.color.text[scheme]);
        await expect(block).toHaveCSS(
          'background-color',
          rgb(tokens.browser.surface.sheet[scheme]),
        );
        await expect(block).toHaveCSS('outline-color', rgb(tokens.browser.surface.rule[scheme]));
        await expect(block).toHaveCSS('border-top-width', '0px');
        for (const surface of [
          activeCard(page).locator('.question-controls'),
          group.locator('.question-group-footer'),
        ])
          await expect(surface).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
        await expect(block).toHaveCSS('color', ink);
        await expect(activeCard(page).locator('.question-controls')).toHaveCSS(
          'color-scheme',
          scheme,
        );
        await expect(rows.first()).toHaveCSS('color', ink);
        const waiting = rows.first().locator('.question-line-mark');
        const role = await waiting.evaluate((mark) => {
          const probe = document.createElement('i');
          probe.style.color = 'var(--tmt-ui-color-waiting)';
          mark.append(probe);
          const color = getComputedStyle(probe).color;
          probe.remove();
          return color;
        });
        await expect(waiting).toHaveCSS('color', role);
        await rows.first().hover();
        await expect(rows.first()).toHaveCSS('color', ink);
        const capture = async (state: string) => {
          await expect(group).not.toHaveAttribute('data-detached');
          expect(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth)).toBe(
            false,
          );
          await expect
            .poll(async () => {
              const trusted = await block.boundingBox();
              const below = await page
                .frameLocator('#ask-page-fixture iframe')
                .locator('#after-question')
                .boundingBox();
              return !!trusted && !!below && below.y >= trusted.y + trusted.height - 1;
            })
            .toBe(true);
          const captures = process.env.COLAB_QUESTION_CAPTURE_DIR;
          const states = process.env.COLAB_GROUP_CAPTURE_STATES?.split(',');
          if (captures && (!states || states.includes(state))) {
            mkdirSync(captures, { recursive: true });
            await page.screenshot({
              fullPage: true,
              path: `${captures}/groups-${state}-${width}-${theme}-page-${darkPage ? 'dark' : 'light'}.png`,
            });
          }
        };
        await capture('waiting');
        await activeCard(page).getByLabel('Keep it', { exact: true }).check();
        const send = group.getByRole('button', { name: /^Send (?:answer|\d+ answers?)/ });
        await expect(send).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
        await expect(send).toHaveCSS('text-decoration-line', 'underline');
        await send.click();
        await expect(rows.first()).toContainText('Answered: Keep it');
        await expect(rows.first()).toHaveAttribute('aria-expanded', 'false');
        await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
        const threads: readonly ThreadView[] = await run(page, 'discussionProof');
        expect(threads).toHaveLength(1);
        expect(threads[0].anchor?.exact).toBe('Which heading should we use?');
        // The compact row is parent-owned; its hidden authored title remains the quote source.
        await run(page, 'questionAsk', 'replied');
        await rows.first().click();
        await expect(activeCard(page).getByTestId('ask-reply')).toContainText(
          'I read your answer in this thread.',
        );
        await expect(rows.first()).toHaveCSS('color', ink);
        await capture('answered');
        await run(page, 'questionGroup', { count: 12, darkPage, longLabels: true, lone: true });
        await expect(rows).toHaveCount(12);
        // A remounted group stays `visibility: hidden` until its slot is placed on the page, and
        // `focus()` on a hidden row does nothing (the ArrowUp then lands on the body).
        await expect(rows.first()).toBeVisible();
        await rows.first().focus();
        await rows.first().press('ArrowUp');
        await expect(rows.nth(11)).toBeFocused();
        await rows.nth(11).press('Enter');
        await expect(rows.nth(11)).toHaveAttribute('aria-expanded', 'true');
        await expect(activeCard(page)).toHaveAttribute(
          'aria-label',
          'What should section 12 explain?',
        );
        await capture('overflow-lone');
        await run(page, 'questionGroup', { count: 1, darkPage });
        await expect(rows).toHaveCount(1);
        await expect(group.locator('.question-compact-heading')).toHaveCount(0);
        await expect(page.getByRole('tab')).toHaveCount(0);
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
              const card = await group.locator('.question-compact-block').boundingBox();
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
        expect(
          await page.evaluate(
            () => document.documentElement.scrollWidth <= document.documentElement.clientWidth,
          ),
        ).toBe(true);
        expect(
          await container.evaluate(
            () => document.documentElement.scrollWidth <= document.documentElement.clientWidth,
          ),
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
      await expect(floating).toBeVisible();
      await expect
        .poll(
          async () => {
            const fab = await floating.boundingBox();
            const controls = await group
              .locator('.tmt-ui-choice, .question-group-footer button')
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
      // Hit the far edge that the old FAB covered, not only the radio at the left.
      const lastOption = group.locator('.tmt-ui-choice').last();
      const option = await lastOption.boundingBox();
      expect(option).not.toBeNull();
      await lastOption.click({ position: { x: option!.width - 2, y: option!.height - 2 } });
      await expect(activeCard(page).getByLabel('Change it', { exact: true })).toBeChecked();
      await activeCard(page).getByLabel('Keep it', { exact: true }).check();
      // A trusted browser click must reach the recipient control, not chrome above it.
      await group.getByRole('button', { name: 'Add recipient', exact: true }).click();
      await expect(page.getByRole('combobox', { name: 'Find an agent' })).toBeVisible();
      await page.getByRole('combobox', { name: 'Find an agent' }).press('Escape');
      await send.click();
      await expect(group.locator('.question-line').first()).toContainText('Answered: Keep it');
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
    await groups.nth(1).locator('.question-line').first().click();
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
        const card = await groups.nth(0).locator('.question-compact-block').boundingBox();
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
    await groups.nth(2).locator('.question-line').first().click();
    await place(2);
    await page.evaluate(() => window.scrollTo(0, 0));
    await expect(floating).toBeVisible();
    await check();
  });

test('question collision placement preserves the fixed header controls', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 800 });
  const group = await mount(page, { layout: 'centered' });
  await activeCard(page).getByLabel('Keep it', { exact: true }).check();
  const surface = page.locator('#ask-page-fixture');
  const floating = surface.locator('.selection-mode');
  const header = await surface.locator('.tmt-ui-header').boundingBox();
  const fab = await floating.boundingBox();
  expect(header).not.toBeNull();
  expect(fab).not.toBeNull();
  const top = header!.y + header!.height + fab!.height - 12;
  await group
    .locator('.question-compact-block')
    .evaluate((node, top) => window.scrollBy(0, node.getBoundingClientRect().top - top), top);
  await expect
    .poll(async () =>
      Math.abs((await group.locator('.question-compact-block').boundingBox())!.y - top),
    )
    .toBeLessThanOrEqual(1);
  await expect
    .poll(
      async () => {
        if (!(await floating.isVisible())) return true;
        const fab = await floating.boundingBox();
        const header = await surface.locator('.tmt-ui-header').boundingBox();
        return !!fab && !!header && fab.y >= header.y + header.height + 8;
      },
      { message: 'moving FAB above a card must not overlap fixed chrome' },
    )
    .toBe(true);
  await surface.getByRole('button', { name: 'More', exact: true }).click();
  await expect(page.getByRole('menu')).toBeVisible();
  await page.getByRole('menu').press('Escape');
});
