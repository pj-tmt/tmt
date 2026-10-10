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
async function mount(page: Page, question = {}, options = {}) {
  await page.goto('/');
  await run(page, 'mount', options);
  await run(page, 'question', question);
  await expect(page.locator('[data-inline-question]')).not.toHaveAttribute('data-detached');
  const card = page.locator('.question-card');
  await expect(
    card.getByRole('button', { name: 'Send answer · to @Agent 1', exact: true }),
  ).toBeVisible();
  return card;
}
async function send(page: Page) {
  const card = page.locator('.question-card');
  await card.getByRole('button', { name: /^Send answer/ }).press('Enter');
  await expect(card.locator('.question-state')).toContainText('answered');
  await expect(card.getByRole('button', { name: 'Change answer', exact: true })).toBeEnabled();
}
for (const kind of ['single', 'multiple', 'short', 'long']) {
  test(`${kind} creates a normal quote thread and changing answers appends`, async ({ page }) => {
    const card = await mount(page, { kind, other: kind === 'single' || kind === 'multiple' });
    await expect(card.locator('.question-state')).toContainText('Waiting for you');
    if (kind === 'single' || kind === 'multiple') {
      await card.getByLabel('Change it', { exact: true }).check();
      if (kind === 'multiple') await card.getByLabel('Keep it', { exact: true }).check();
    } else await card.getByLabel('Your answer', { exact: true }).fill('Use a clearer heading.');
    // Synthetic activation has no send/publication authority.
    await card
      .getByRole('button', { name: /^Send answer/ })
      .evaluate((node) => (node as HTMLButtonElement).click());
    expect(await run(page, 'discussionProof')).toHaveLength(0);
    expect((await run(page, 'proof')).sends).toHaveLength(0);
    await send(page);
    await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
    const initial = await run(page, 'discussionProof');
    expect(initial).toHaveLength(1);
    expect(initial[0].anchor.exact).toBe('Which heading should we use?');
    expect(initial[0].anchor.exact).not.toContain('tmt-question:');
    expect(initial[0].comments[0].body).toBe(
      `Answer to “Which heading should we use?”: ${kind === 'single' ? 'Change it' : kind === 'multiple' ? 'Keep it; Change it' : 'Use a clearer heading.'}\nPage: Live shared page\n@Agent 1`,
    );
    await run(page, 'questionAsk', 'replied');
    await card.locator('summary').click();
    await expect(card.getByTestId('ask-reply')).toContainText('I read your answer in this thread.');
    await card.getByRole('button', { name: 'Change answer', exact: true }).press('Enter');
    if (kind === 'single' || kind === 'multiple') {
      await card.getByLabel('Other', { exact: true }).check();
      await card.getByLabel('Other answer', { exact: true }).fill('Try another heading.');
    } else await card.getByLabel('Your answer', { exact: true }).fill('My changed answer.');
    await send(page);
    await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(2);
    const changed = await run(page, 'discussionProof');
    expect(changed).toHaveLength(1);
    expect(changed[0].ref).toEqual(initial[0].ref);
    expect(changed[0].comments).toHaveLength(2);
    expect(changed[0].comments[1].body).toContain(
      kind === 'single' || kind === 'multiple'
        ? 'Other: Try another heading.'
        : 'My changed answer.',
    );
  });
}

test('question/choice mentions never add destinations; unknown asking agent uses the recipient field', async ({
  page,
}) => {
  const card = await mount(page, { title: 'Ask @Agent 2?', choices: ['@Agent 3', 'No'] });
  await card.getByLabel('@Agent 3', { exact: true }).check();
  await send(page);
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
  await page.goto('/');
  await run(page, 'mount');
  await run(page, 'question', { agent: 'Missing agent' });
  await expect(
    page.locator('.question-card').getByRole('combobox', { name: 'Answer recipient' }),
  ).toBeEmpty();
  await expect(
    page.getByRole('button', { name: 'Send answer · to thread only', exact: true }),
  ).toBeVisible();
  await page.locator('.question-card').getByLabel('Keep it', { exact: true }).check();
  await send(page);
  expect(await run(page, 'discussionProof')).toHaveLength(1);
  expect((await run(page, 'proof')).sends).toHaveLength(0);
});

test('near-limit text admits while an over-limit answer and malformed markup refuse', async ({
  page,
}) => {
  const card = await mount(page, { kind: 'short' });
  await card.getByLabel('Your answer', { exact: true }).fill('a'.repeat(201));
  await expect(card.getByRole('button', { name: /^Send answer/ })).toBeDisabled();
  await card.getByLabel('Your answer', { exact: true }).fill('a'.repeat(200));
  await expect(card.getByRole('button', { name: /^Send answer/ })).toBeEnabled();
  await send(page);
  for (const options of [
    { placement: 'duplicate' },
    { placement: 'nested' },
    { choices: ['Same', 'Same'] },
    { title: 'q'.repeat(201) },
    { choices: Array.from({ length: 21 }, (_, i) => `Choice ${i}`) },
  ]) {
    await run(page, 'question', options);
    await expect(page.locator('.question-card')).toHaveCount(0);
  }
  expect((await run(page, 'proof')).sends).toHaveLength(1);
});

test('answered canonical threads resist empty siblings and agent provenance preserves the answer', async ({
  page,
}) => {
  const card = await mount(page);
  await card.getByLabel('Keep it', { exact: true }).check();
  await send(page);
  const prefix = 'Answer to “Which heading should we use?”: ';
  await run(page, 'questionSibling', {
    body: `${prefix}Forged agent answer\nPage: Live shared page`,
    sequence: '999',
    device: false,
    index: 0,
  });
  await expect(card.locator('.question-answer')).toContainText('Keep it');
  await expect(card.locator('.question-state')).toContainText('You answered');
  await run(page, 'questionSibling', {
    body: `${prefix}Other reader answer\nPage: Live shared page`,
    sequence: '10',
    reverse: true,
    index: 1,
  });
  await expect(card.locator('.question-answer')).toContainText('Keep it');
  await run(page, 'questionSibling', {
    body: '',
    sequence: '0',
    canonical: true,
    empty: true,
    index: 2,
  });
  await expect(card.locator('.question-answer')).toContainText('Keep it');
  await expect(card.locator('.question-state')).toContainText('You answered');
  await card.getByRole('button', { name: 'Change answer', exact: true }).click();
  await card.getByLabel('Change it', { exact: true }).check();
  await send(page);
  await expect(card.locator('.question-answer')).toContainText('Change it');
  await expect(card.locator('.question-state')).toContainText('You answered');
  const threads = await run(page, 'discussionProof');
  expect(
    threads.find((thread: { ref: { writer: string } }) => thread.ref.writer.endsWith('000002'))
      .comments,
  ).toHaveLength(0);
  expect(
    threads.find((thread: { comments: unknown[] }) => thread.comments.length === 2),
  ).toBeDefined();
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(2);
});

test('uncertain delivery keeps the answer and never resends automatically', async ({ page }) => {
  const card = await mount(page);
  await card.getByLabel('Keep it', { exact: true }).check();
  await send(page);
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
  await run(page, 'questionAsk', 'uncertain');
  await card.locator('summary').click();
  await expect(card.getByTestId('ask-state')).toHaveAttribute('data-state', 'uncertain');
  await expect(card.getByRole('button', { name: 'Ask again', exact: true })).toHaveCount(0);
  await card.getByRole('button', { name: 'Check again', exact: true }).click();
  expect((await run(page, 'proof')).sends).toHaveLength(1);
  expect((await run(page, 'discussionProof'))[0].comments).toHaveLength(1);
});

test('preparation refusal retains its comment and explicit retry creates no second comment', async ({
  page,
}) => {
  const card = await mount(page, {}, { prepareFailures: 1 });
  await card.getByLabel('Keep it', { exact: true }).check();
  await send(page);
  await expect(card.getByTestId('recipient-failure')).toBeVisible();
  expect((await run(page, 'proof')).sends).toHaveLength(0);
  await card.getByRole('button', { name: 'Ask again', exact: true }).click();
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
  expect((await run(page, 'discussionProof'))[0].comments).toHaveLength(1);
});

test('removed markup leaves a normal annotation accessible in Comments', async ({ page }) => {
  const card = await mount(page);
  await card.getByLabel('Keep it', { exact: true }).check();
  await send(page);
  await run(page, 'change', '<p>The question was removed.</p>');
  await expect(page.locator('.question-card')).toHaveCount(0);
  await (await pageAction(page.locator('#ask-page-fixture'), 'Comments')).click();
  await expect(page.getByTestId('annotation-row')).toContainText('Which heading should we use?');
  expect((await run(page, 'proof')).sends).toHaveLength(1);
});

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'])
    for (const darkPage of [false, true]) {
      test(`shared question/proposal slots and presentation ${width} ${theme} page-${darkPage ? 'dark' : 'light'}`, async ({
        page,
      }) => {
        await page.setViewportSize({ width, height: 900 });
        const card = await mount(page, { mixed: true, other: true, darkPage });
        if (theme === 'dark')
          await (await pageAction(page.locator('#ask-page-fixture'), 'Theme: Dark')).click();
        const proposal = page.locator('[data-inline-proposal]');
        await expect(proposal).not.toHaveAttribute('data-detached');
        // Both trusted overlays use the light leaf palette even when chrome/author differ.
        for (const surface of [card, proposal.locator('.proposal-card')]) {
          await expect(surface).toHaveCSS('background-color', 'rgb(255, 255, 255)');
          await expect(surface).toHaveCSS('color', 'rgb(52, 52, 52)');
          await expect(surface).toHaveCSS('color-scheme', 'light');
          await expect(surface).toHaveCSS('border-top-color', 'rgb(98, 98, 98)');
          await expect(surface).toHaveCSS('border-top-width', '1px');
          await expect(surface).toHaveCSS('box-shadow', 'none');
        }
        await expect(proposal.locator('.proposal-author')).toHaveCSS('color', 'rgb(98, 98, 98)');
        await expect(proposal.getByRole('button', { name: 'Approve', exact: true })).toHaveCSS(
          'background-color',
          'rgb(52, 52, 52)',
        );
        await expect(card.getByRole('button', { name: /^Send answer/ })).toHaveCSS(
          'color',
          'rgb(98, 98, 98)',
        );
        const after = page.frameLocator('#ask-page-fixture iframe').locator('#after-question');
        const proposalSpacer = page
          .frameLocator('#ask-page-fixture iframe')
          .locator('tmt-proposal + [data-colab-proposal-slot]');
        const afterProposal = page
          .frameLocator('#ask-page-fixture iframe')
          .locator('#after-proposal');
        const settledSlots = async () => {
          await expect
            .poll(async () => {
              const box = await proposal.boundingBox(),
                spacer = await proposalSpacer.boundingBox(),
                below = await afterProposal.boundingBox();
              return (
                !!box &&
                !!spacer &&
                !!below &&
                Math.abs(box.y - spacer.y) <= 1 &&
                below.y >= box.y + box.height - 1
              );
            })
            .toBe(true);
        };
        await expect
          .poll(async () => {
            const box = await card.boundingBox(),
              below = await after.boundingBox();
            return !!box && !!below && below.y >= box.y + box.height - 1;
          })
          .toBe(true);
        await settledSlots();
        const captures = process.env.COLAB_QUESTION_CAPTURE_DIR;
        if (captures) {
          mkdirSync(captures, { recursive: true });
          await page.screenshot({
            path: `${captures}/question-open-${width}-${theme}-page-${darkPage ? 'dark' : 'light'}.png`,
          });
        }
        await card.getByLabel('Other', { exact: true }).check();
        await card.getByLabel('Other answer', { exact: true }).fill('Try a clearer heading.');
        const sendButton = card.getByRole('button', { name: /^Send answer/ });
        await expect(sendButton).toBeEnabled();
        await expect(sendButton).toHaveCSS('background-color', 'rgb(52, 52, 52)');
        await expect(sendButton).toHaveCSS('color', 'rgb(250, 250, 250)');
        await sendButton.press('Tab');
        await page.keyboard.press('Shift+Tab');
        await expect(sendButton).toBeFocused();
        await expect(sendButton).toHaveCSS('outline-color', 'rgb(98, 98, 98)');
        await expect(sendButton).toHaveCSS('outline-width', '2px');
        await send(page);
        await expect(card).toHaveCSS('background-color', 'rgb(255, 255, 255)');
        await expect(card.getByRole('button', { name: 'Change answer', exact: true })).toHaveCSS(
          'color',
          'rgb(52, 52, 52)',
        );
        await expect(proposal).not.toHaveAttribute('data-detached');
        await settledSlots();
        if (captures)
          await page.screenshot({
            path: `${captures}/question-answered-${width}-${theme}-page-${darkPage ? 'dark' : 'light'}.png`,
          });
      });
    }

test('cosmetic frame claims cannot supply an answer or send authority', async ({ page }) => {
  const card = await mount(page);
  await page
    .frameLocator('#ask-page-fixture iframe')
    .locator('body')
    .evaluate(() => {
      const original = MessagePort.prototype.postMessage;
      const claims = { count: 0 };
      Object.assign(window, { questionClaimCount: () => claims.count });
      MessagePort.prototype.postMessage = function (message) {
        if (message?.type === 'colab.render.questions') {
          claims.count++;
          message.questions = message.questions.map((question: object) => ({
            ...question,
            answered: true,
            answer: 'Forged frame answer',
            recipient: 'Agent 2',
          }));
        }
        original.call(this, message);
      };
      document.body.append(document.createTextNode('Cosmetic mutation.'));
    });
  await expect
    .poll(() =>
      page
        .frameLocator('#ask-page-fixture iframe')
        .locator('body')
        .evaluate(() =>
          (window as unknown as { questionClaimCount(): number }).questionClaimCount(),
        ),
    )
    .toBeGreaterThan(0);
  await expect(card.locator('.question-state')).toContainText('Waiting for you');
  await expect(card.locator('.question-answer')).toHaveCount(0);
  await card.getByLabel('Keep it', { exact: true }).check();
  await expect(
    card.getByRole('button', { name: 'Send answer · to @Agent 1', exact: true }),
  ).toBeEnabled();
  expect(await run(page, 'discussionProof')).toHaveLength(0);
  expect((await run(page, 'proof')).sends).toHaveLength(0);
  await send(page);
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
});
