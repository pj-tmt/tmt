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
  const card = page.locator('.question-group-card');
  await card.locator('.question-line').click();
  await expect(
    card.getByRole('button', { name: 'Send answer · to @Agent 1', exact: true }),
  ).toBeVisible();
  return card;
}
async function send(page: Page) {
  const card = page.locator('.question-group-card');
  await card.getByRole('button', { name: /^Send answer/ }).press('Enter');
  const line = card.locator('.question-line');
  await expect(line).toContainText('Answered:');
  await expect(line).toHaveAttribute('aria-expanded', 'false');
  await line.click();
  await expect(card.getByRole('button', { name: 'Change answer', exact: true })).toBeEnabled();
}
for (const kind of ['single', 'multiple', 'short', 'long']) {
  test(`${kind} creates a normal quote thread and changing answers appends`, async ({ page }) => {
    const card = await mount(page, { kind, other: kind === 'single' || kind === 'multiple' });
    await expect(card.locator('.question-line')).toContainText('Needs you');
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
  await page.locator('.question-line').click();
  await expect(
    page.getByRole('group', { name: 'Answer recipients' }).locator('.tmt-ui-chip'),
  ).toHaveCount(0);
  await expect(
    page.getByRole('button', { name: 'Send answer · to thread only', exact: true }),
  ).toBeVisible();
  await expect(page.getByRole('button', { name: /^Send answer/ })).toHaveText('Send answer');
  await page.locator('.question-group-card').getByLabel('Keep it', { exact: true }).check();
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
    await expect(page.locator('.question-group-card')).toHaveCount(0);
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
  await expect(card.locator('.question-line')).toContainText('Answered:');
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
  await expect(card.locator('.question-line')).toContainText('Answered:');
  await card.getByRole('button', { name: 'Change answer', exact: true }).click();
  await card.getByLabel('Change it', { exact: true }).check();
  await send(page);
  await expect(card.locator('.question-answer')).toContainText('Change it');
  await expect(card.locator('.question-line')).toContainText('Answered:');
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

test('an answered question closes to its focused line and Change answer restores Send', async ({
  page,
}) => {
  const card = await mount(page);
  await card.getByLabel('Keep it', { exact: true }).check();
  await card.getByRole('button', { name: /^Send answer/ }).press('Enter');
  const line = card.locator('.question-line');
  await expect(line).toContainText('Answered: Keep it');
  await expect(line).toHaveAttribute('aria-expanded', 'false');
  await expect(line).toBeFocused();
  // Nothing of the answer being written stays next to the folded line.
  await expect(card.getByRole('button', { name: /^Send answer/ })).toHaveCount(0);
  await expect(card.getByRole('group', { name: 'Choose one' })).toHaveCount(0);
  await expect(card.getByRole('button', { name: 'Remove @Agent 1' })).toHaveCount(0);
  await line.click();
  await expect(card.locator('.question-answer > p')).toHaveText('Keep it');
  await card.getByRole('button', { name: 'Change answer', exact: true }).click();
  await expect(card.getByRole('group', { name: 'Choose one' })).toBeVisible();
  await expect(card.getByRole('button', { name: /^Send answer/ })).toBeVisible();
  await expect(card.getByRole('button', { name: 'Remove @Agent 1' })).toBeVisible();
});

/** Each change of the card's reserved height (the page moves by it), with the step it came in. */
async function watchSlotHeight(page: Page) {
  await page.evaluate(() => {
    const steps: { step: string; height: number }[] = [];
    const watch = window as unknown as { slotSteps: typeof steps; slotStep: string };
    watch.slotSteps = steps;
    watch.slotStep = 'before';
    let last = 0;
    const tick = () => {
      const slot = document.querySelector('[data-inline-slot]');
      const height = slot ? Math.round(slot.getBoundingClientRect().height) : last;
      if (Math.abs(height - last) > 1) {
        if (last) steps.push({ step: watch.slotStep, height: height - last });
        last = height;
      }
      requestAnimationFrame(tick);
    };
    tick();
  });
}
const slotStep = (page: Page, step: string) =>
  page.evaluate((step) => {
    (window as unknown as { slotStep: string }).slotStep = step;
  }, step);
const slotSteps = (page: Page) =>
  page.evaluate(() => (window as unknown as { slotSteps: unknown[] }).slotSteps);

test('answering a card moves the page once, however late the record and the Ask land', async ({
  page,
}) => {
  const card = await mount(page, {}, { holdQuestionView: true });
  await card.getByLabel('Keep it', { exact: true }).check();
  await run(page, 'pausePrepare');
  await watchSlotHeight(page);
  await slotStep(page, 'send');
  await card.getByRole('button', { name: /^Send answer/ }).click();
  await expect(card.locator('.question-line')).toContainText('Answered: Keep it');
  await page.waitForTimeout(600);
  await slotStep(page, 'record');
  await run(page, 'releaseQuestionView');
  // The record has landed once the answer's history stands in the folded card.
  await expect(card.locator('.question-history').first()).toBeAttached();
  await page.waitForTimeout(600);
  await slotStep(page, 'ask');
  await run(page, 'resumePrepare');
  await expect(card.locator('.question-line')).toHaveAttribute('aria-expanded', 'false');
  await page.waitForTimeout(600);
  // The sampler sees every frame: the one fold is Send's; the record and the Ask move nothing.
  const steps = (await slotSteps(page)) as { step: string; height: number }[];
  expect(
    steps.map(({ step }) => step),
    JSON.stringify(steps),
  ).toEqual(['send']);
  expect(steps[0].height).toBeLessThan(0);
});

test('a failed answer re-opens its folded card on the error row, Retry focused, draft kept', async ({
  page,
}) => {
  const card = await mount(page, {}, { failAnswerAt: 1 });
  const line = card.locator('.question-line');
  await card.getByLabel('Keep it', { exact: true }).check();
  await card.getByRole('button', { name: /^Send answer/ }).click();
  // The failure is the one move after Send's fold: the card opens in place on its error row.
  const retry = card.getByRole('button', { name: 'Retry', exact: true });
  await expect(card.getByRole('alert')).toHaveText(
    'Could not confirm this answer. Your draft is kept for the next Send.',
  );
  await expect(line).toHaveAttribute('aria-expanded', 'true');
  await expect(line).toContainText('Needs you');
  await expect(retry).toBeFocused();
  await expect(card.getByLabel('Keep it', { exact: true })).toBeChecked();
  expect(await run(page, 'discussionProof')).toHaveLength(0);
  await retry.click();
  await expect(line).toContainText('Answered: Keep it');
  await expect(line).toHaveAttribute('aria-expanded', 'false');
  await expect.poll(async () => (await run(page, 'discussionProof')).length).toBe(1);
});

for (const change of [false, true]) {
  test(`a verified answer before Ask completion releases its recorded draft (Change answer ${change})`, async ({
    page,
  }) => {
    const card = await mount(page);
    await card.getByLabel('Keep it', { exact: true }).check();
    if (change) {
      await send(page);
      await card.getByRole('button', { name: 'Change answer', exact: true }).click();
      await card.getByLabel('Change it', { exact: true }).check();
    }
    await run(page, 'pausePrepare');
    await card.getByRole('button', { name: /^Send answer/ }).click();
    // The answer stands on its folded line at once; no step is spelled out.
    const line = card.locator('.question-line');
    await expect(line).toContainText(`Answered: ${change ? 'Change it' : 'Keep it'}`);
    await expect(line).toHaveAttribute('aria-expanded', 'false');
    await expect(page.getByText('Sending…', { exact: true })).toHaveCount(0);
    await expect(page.getByText(/waiting for the verified view/)).toHaveCount(0);
    const recorded = await run(page, 'discussionProof');
    expect(recorded).toHaveLength(1);
    expect(recorded[0].comments).toHaveLength(change ? 2 : 1);
    expect((await run(page, 'proof')).sends).toHaveLength(change ? 1 : 0);
    await run(page, 'resumePrepare');
    await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(change ? 2 : 1);
    await expect(card.locator('.question-line')).toHaveAttribute('aria-expanded', 'false');
    await card.locator('.question-line').click();
    await expect(card.getByRole('button', { name: 'Change answer', exact: true })).toBeEnabled();
    expect(await run(page, 'discussionProof')).toEqual(recorded);
    await expect(card.getByRole('button', { name: /^Send answer/ })).toHaveCount(0);
  });
}

test('the answer conversation keeps an agent name with a space in one mention chip', async ({
  page,
}) => {
  // The viewer holds no Ask record for the comment; the observed directory names the agent.
  const card = await mount(page, {}, { presentation: ['Agent 1'] });
  await card.getByLabel('Keep it', { exact: true }).check();
  await send(page);
  const body = card.locator('.question-history .comment-body');
  await expect(body.locator('.agent-chip-name')).toHaveText(['Agent 1']);
  await expect(body).toContainText('Page: Live shared page Agent 1');
  // The same chip stays whole once the viewer's own Ask record covers the comment.
  await run(page, 'questionAsk', 'replied');
  await expect(card.getByTestId('ask-reply')).toContainText('I read your answer in this thread.');
  await expect(body.locator('.agent-chip-name')).toHaveText(['Agent 1']);
});

test('ordinary answer history growth keeps the inline card in window scrolling', async ({
  page,
}) => {
  const card = await mount(page);
  await card.getByLabel('Keep it', { exact: true }).check();
  await send(page);
  const frame = page.locator('#ask-page-fixture iframe');
  for (let index = 0; index < 6; index++) {
    const oldHeight = await frame.evaluate((node) => node.getBoundingClientRect().height);
    const block = card.locator('.question-compact-block');
    const oldContent = await block.evaluate((node) => node.scrollHeight);
    const oldBox = (await block.boundingBox())!.height;
    await run(page, 'questionSibling', {
      body: `Ordinary follow-up ${index}:\n` + 'Details about the requested revision.\n'.repeat(12),
      sequence: String(index + 1),
      device: false,
      index,
    });
    await expect(card.locator('.question-history article')).toHaveCount(index + 2);
    await expect
      .poll(() => block.evaluate((node) => node.scrollHeight))
      .toBeGreaterThan(oldContent);
    if (oldBox < 1900)
      await expect
        .poll(() => frame.evaluate((node) => node.getBoundingClientRect().height))
        .toBeGreaterThan(oldHeight);
    else await expect(frame).toHaveJSProperty('clientHeight', oldHeight);
    expect((await block.boundingBox())!.height).toBeLessThanOrEqual(1900);
    await expect(frame).toHaveAttribute('data-scroll-mode', 'window');
    await expect(card.getByRole('button', { name: 'Change answer', exact: true })).toBeEnabled();
    await expect(page.locator('[data-inline-question]')).not.toHaveAttribute('data-detached');
  }
  await expect(card.locator('.question-answer > p')).toHaveText('Keep it');
  expect((await run(page, 'proof')).sends).toHaveLength(1);
});

test('repeated Change answer keeps growing history attached in window scrolling', async ({
  page,
}) => {
  const card = await mount(page);
  await card.getByLabel('Keep it', { exact: true }).check();
  await send(page);
  for (let index = 0; index < 6; index++) {
    await card.getByRole('button', { name: 'Change answer', exact: true }).click();
    await card.getByLabel(index % 2 ? 'Keep it' : 'Change it', { exact: true }).check();
    await send(page);
    await expect(card.locator('.question-history article')).toHaveCount(index + 2);
    await expect(page.locator('#ask-page-fixture iframe')).toHaveAttribute(
      'data-scroll-mode',
      'window',
    );
    await expect(page.locator('[data-inline-question]')).not.toHaveAttribute('data-detached');
  }
  const threads = await run(page, 'discussionProof');
  expect(threads).toHaveLength(1);
  expect(threads[0].comments).toHaveLength(7);
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(7);
});

test('authored viewport feedback still detaches a question card while its history grows', async ({
  page,
}) => {
  const card = await mount(page);
  await card.getByLabel('Keep it', { exact: true }).check();
  await send(page);
  await page
    .frameLocator('#ask-page-fixture iframe')
    .locator('body')
    .evaluate((body) => {
      const growth = body.ownerDocument.createElement('div');
      growth.style.height = 'calc(100vh + 120px)';
      growth.textContent = 'Authored viewport feedback';
      body.append(growth);
    });
  await run(page, 'questionSibling', {
    body: 'An ordinary follow-up grows the trusted conversation during authored viewport feedback.',
    sequence: '1',
    device: false,
    index: 0,
  });
  await expect(card.locator('.question-history article')).toHaveCount(2);
  const frame = page.locator('#ask-page-fixture iframe');
  await expect(frame).toHaveAttribute('data-scroll-mode', 'frame');
  await expect(frame).toHaveAttribute('scrolling', 'auto');
  await expect(page.locator('[data-inline-question]')).toHaveAttribute('data-detached', 'true');
  await expect(page.locator('.question-group-card')).not.toBeVisible();
  expect(await run(page, 'discussionProof')).toHaveLength(2);
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
});

test('uncertain delivery keeps the answer and never resends automatically', async ({ page }) => {
  const card = await mount(page);
  await card.getByLabel('Keep it', { exact: true }).check();
  await send(page);
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
  await run(page, 'questionAsk', 'uncertain');
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
  await expect(page.locator('.question-group-card')).toHaveCount(0);
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
        await expect
          .poll(async () => {
            const questionBox = await card.locator('.question-compact-block').boundingBox();
            const proposalBox = await proposal.locator('.proposal-card').boundingBox();
            if (!questionBox || !proposalBox) return null;
            return {
              left: Math.round(proposalBox.x - questionBox.x),
              right: Math.round(
                proposalBox.x + proposalBox.width - questionBox.x - questionBox.width,
              ),
            };
          })
          .toEqual({ left: 0, right: 0 });
        const frame = page.frameLocator('#ask-page-fixture iframe');
        await expect(frame.locator('html')).toHaveAttribute('data-theme', theme);
        // The cards take the scheme the page paints, whatever the app theme (#4081).
        const scheme = darkPage ? 'dark' : 'light';
        await expect(
          page.locator('.page-cards', { has: page.locator('.question-group-card') }),
        ).toHaveAttribute('data-theme', scheme);
        const question = card.locator('.question-compact-block');
        // The sheet of the scheme the page paints (#3959, #4068, #4081).
        await expect(question).toHaveCSS(
          'background-color',
          scheme === 'dark' ? 'rgb(16, 16, 16)' : 'rgb(255, 255, 255)',
        );
        await expect(question).toHaveCSS(
          'color',
          scheme === 'dark' ? 'rgb(176, 176, 176)' : 'rgb(52, 52, 52)',
        );
        // Compact questions stay flat while text and controls follow the painted scheme.
        for (const surface of [
          card.locator('.question-controls'),
          card.locator('.question-group-footer'),
        ]) {
          await expect(surface).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
          await expect(surface).toHaveCSS(
            'color',
            scheme === 'dark' ? 'rgb(176, 176, 176)' : 'rgb(52, 52, 52)',
          );
          await expect(surface).toHaveCSS('color-scheme', scheme);
          await expect(surface).toHaveCSS('box-shadow', 'none');
        }
        // Compact disclosure text stays neutral; only its mark carries attention.
        await expect(card.locator('.question-line')).toHaveCSS(
          'color',
          scheme === 'dark' ? 'rgb(176, 176, 176)' : 'rgb(52, 52, 52)',
        );
        await expect(card.locator('.question-group-footer')).toHaveCSS('border-top-width', '0px');
        // Sibling agent cards follow the same painted scheme.
        await expect(proposal.locator('.proposal-card')).toHaveCSS(
          'background-color',
          scheme === 'dark' ? 'rgb(16, 16, 16)' : 'rgb(255, 255, 255)',
        );
        await expect(proposal.locator('.proposal-card')).toHaveCSS(
          'color',
          scheme === 'dark' ? 'rgb(176, 176, 176)' : 'rgb(52, 52, 52)',
        );
        await expect(proposal.locator('.proposal-card')).toHaveCSS('color-scheme', scheme);
        await expect(proposal.locator('.proposal-card')).toHaveCSS('box-shadow', 'none');
        await expect(proposal.locator('.proposal-author')).toHaveCSS(
          'color',
          scheme === 'dark' ? 'rgb(144, 144, 144)' : 'rgb(98, 98, 98)',
        );
        await expect(proposal.getByRole('button', { name: 'Approve', exact: true })).toHaveCSS(
          'background-color',
          scheme === 'dark' ? 'rgb(176, 176, 176)' : 'rgb(52, 52, 52)',
        );
        await expect(card.getByRole('button', { name: /^Send answer/ })).toHaveCSS(
          'color',
          scheme === 'dark' ? 'rgb(133, 133, 133)' : 'rgb(98, 98, 98)',
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
            const box = await card.locator('.question-compact-block').boundingBox(),
              below = await after.boundingBox();
            return !!box && !!below && below.y >= box.y + box.height - 1;
          })
          .toBe(true);
        await settledSlots();
        const captures = process.env.COLAB_QUESTION_CAPTURE_DIR;
        if (captures) {
          mkdirSync(captures, { recursive: true });
          await expect(page.locator('[data-inline-question]')).not.toHaveAttribute('data-detached');
          await page.screenshot({
            fullPage: false,
            path: `${captures}/question-open-${width}-${theme}-page-${darkPage ? 'dark' : 'light'}.png`,
          });
        }
        await card.getByLabel('Other', { exact: true }).check();
        await card.getByLabel('Other answer', { exact: true }).fill('Try a clearer heading.');
        const sendButton = card.getByRole('button', { name: /^Send answer/ });
        await expect(sendButton).toBeEnabled();
        // Send is an inline text action, like the other needs-you actions.
        await expect(sendButton).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
        await expect(sendButton).toHaveCSS('text-decoration-line', 'underline');
        await expect(sendButton).toHaveCSS(
          'color',
          scheme === 'dark' ? 'rgb(176, 176, 176)' : 'rgb(52, 52, 52)',
        );
        await sendButton.press('Tab');
        await page.keyboard.press('Shift+Tab');
        await expect(sendButton).toBeFocused();
        await expect(sendButton).toHaveCSS(
          'outline-color',
          scheme === 'dark' ? 'rgb(176, 176, 176)' : 'rgb(98, 98, 98)',
        );
        await expect(sendButton).toHaveCSS('outline-width', '1px');
        await send(page);
        await expect(card.locator('.question-controls')).toHaveCSS(
          'background-color',
          'rgba(0, 0, 0, 0)',
        );
        // Focus moved to the folded line's action; its resting colour is the text role.
        const change = card.getByRole('button', { name: 'Change answer', exact: true });
        await expect(change).toBeFocused();
        await change.blur();
        await expect(change).toHaveCSS(
          'color',
          scheme === 'dark' ? 'rgb(176, 176, 176)' : 'rgb(52, 52, 52)',
        );
        await expect(proposal).not.toHaveAttribute('data-detached');
        await settledSlots();
        if (captures) {
          await expect(page.locator('[data-inline-question]')).not.toHaveAttribute('data-detached');
          await page.screenshot({
            fullPage: false,
            path: `${captures}/question-answered-${width}-${theme}-page-${darkPage ? 'dark' : 'light'}.png`,
          });
        }
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
      Object.assign(window, {
        questionClaimCount: () => claims.count,
        restoreQuestionClaims: () => {
          MessagePort.prototype.postMessage = original;
        },
      });
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
  await expect(card.locator('.question-line')).toContainText('Needs you');
  await expect(card.locator('.question-answer')).toHaveCount(0);
  await card.getByLabel('Keep it', { exact: true }).check();
  await expect(
    card.getByRole('button', { name: 'Send answer · to @Agent 1', exact: true }),
  ).toBeEnabled();
  expect(await run(page, 'discussionProof')).toHaveLength(0);
  expect((await run(page, 'proof')).sends).toHaveLength(0);
  // Resume valid cosmetic quote responses; unknown fields cannot associate a new verified thread.
  await page
    .frameLocator('#ask-page-fixture iframe')
    .locator('body')
    .evaluate(() => {
      (window as unknown as { restoreQuestionClaims(): void }).restoreQuestionClaims();
      document.body.append(document.createTextNode('Valid geometry resumed.'));
    });
  await send(page);
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
});

test('archiving an unanswered question preserves text for Copy and removes answering controls', async ({
  page,
}) => {
  const card = await mount(page, { kind: 'short' });
  await card.getByRole('textbox', { name: 'Your answer' }).fill('Retain this answer draft');
  await run(page, 'archive');
  await expect(card.getByText('Unsent draft', { exact: true })).toBeVisible();
  await expect(card.locator('.archive-unsent-draft pre')).toContainText('Retain this answer draft');
  await expect(card.getByRole('button', { name: 'Copy', exact: true })).toBeVisible();
  await expect(card.getByRole('button', { name: /^Send answer/ })).toHaveCount(0);
  await expect(card.getByRole('textbox')).toHaveCount(0);
  await expect(card.getByRole('combobox')).toHaveCount(0);
  expect((await run(page, 'proof')).sends).toEqual([]);
});
