import { expect, test, type Locator, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { pageAction } from '../test/page-actions.js';
import type { ThreadView } from '../src/thread-records.js';
import { destination, id } from '../test/ask-fixtures.js';

const fixture = '/test/ask-page-browser.tsx';
async function run(page: Page, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
async function mount(page: Page, group = {}, options = {}) {
  await page.goto('/');
  await run(page, 'mount', options);
  await run(page, 'questionGroup', group);
  const surface = page.locator('[data-question-group]').first();
  await expect(surface).not.toHaveAttribute('data-detached');
  await surface.locator('.question-line').first().click();
  return surface;
}
const card = (page: Page) => page.locator('.question-card:visible').first();
async function answerThree(page: Page, group: Locator) {
  await card(page).getByLabel('Keep it', { exact: true }).check();
  await group.locator('.question-line').nth(1).click();
  await card(page).getByLabel('Your answer', { exact: true }).fill('A clear introduction');
  await group.locator('.question-line').nth(2).click();
  await card(page).getByLabel('Your answer', { exact: true }).fill('Explain the tradeoffs');
  await expect(group.locator('.question-compact-heading')).toContainText('3 of 3 waiting');
}
async function openRecorded(group: Locator, index: number) {
  const row = group.locator('.question-line').nth(index);
  await expect(row).toHaveAttribute('aria-expanded', 'false');
  await row.click();
}

async function recorded(page: Page, count: number) {
  await expect
    .poll(async () => {
      const threads: readonly ThreadView[] = await run(page, 'discussionProof');
      return threads.reduce((sum, thread) => sum + thread.comments.length, 0);
    })
    .toBe(count);
}

test('an empty group keeps the lone label and omits a zero count; a recorded answer folds Send away', async ({
  page,
}) => {
  for (const count of [1, 3]) {
    const group = await mount(page, { count }, { agents: [] });
    const emptyLabel = count === 1 ? 'Send answer' : 'Send answers';
    const empty = group.getByRole('button', {
      name: `${emptyLabel} · to thread only`,
      exact: true,
    });
    await expect(empty).toHaveText(emptyLabel);
    await expect(empty).toBeDisabled();
    await card(page).getByLabel('Keep it', { exact: true }).check();
    const label = count === 1 ? 'Send answer' : 'Send 1 answer';
    const send = group.getByRole('button', { name: `${label} · to thread only`, exact: true });
    await expect(send).toBeEnabled();
    await send.click();
    await recorded(page, 1);
    await openRecorded(group, 0);
    await expect(
      card(page).getByRole('button', { name: 'Change answer', exact: true }),
    ).toBeVisible();
    // The folded answered line never has a Send next to it; Change answer brings it back.
    await expect(empty).toHaveCount(0);
    expect((await run(page, 'proof')).sends).toHaveLength(0);
  }
});

test('a lone question folds to its answered line, without Send, while its publication is in flight', async ({
  page,
}) => {
  const group = await mount(page, { count: 1 });
  const send = group.getByRole('button', { name: 'Send answer · to @Agent 1', exact: true });
  await expect(send).toBeDisabled();
  await card(page).getByLabel('Keep it', { exact: true }).check();
  await expect(send).toBeEnabled();
  await run(page, 'pausePrepare');
  await send.click();
  await recorded(page, 1);
  await expect(
    card(page).getByRole('button', { name: 'Change answer', exact: true }),
  ).toBeVisible();
  await expect(send).toHaveCount(0);
  expect((await run(page, 'proof')).sends).toHaveLength(0);
  await run(page, 'resumePrepare');
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
  await expect(send).toHaveCount(0);
});

test('three comments make one Ask per distinct recipient; only the canonical question renders replies, including after reload', async ({
  page,
}) => {
  const group = await mount(page, { count: 3, agents: ['Agent 1', 'Agent 2', 'Agent 1'] });
  const host = page.locator('#ask-page-fixture');
  const live = host.locator('.tmt-ui-notification-status');
  const notifications = host.locator('.tmt-ui-notification');
  await expect(group.getByRole('button', { name: 'Remove @Agent 1', exact: true })).toHaveCount(1);
  await expect(group.getByRole('button', { name: 'Remove @Agent 2', exact: true })).toHaveCount(1);
  await answerThree(page, group);
  expect(await run(page, 'discussionProof')).toHaveLength(0);
  expect((await run(page, 'proof')).sends).toHaveLength(0);
  await group
    .getByRole('button', { name: 'Send 3 answers · to @Agent 1, @Agent 2', exact: true })
    .click();
  await recorded(page, 3);
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(2);
  await openRecorded(group, 2);
  const threads: readonly ThreadView[] = await run(page, 'discussionProof');
  expect(threads).toHaveLength(3);
  expect(threads.map((thread) => thread.anchor?.exact)).toEqual([
    'Which heading should we use?',
    'What should the introduction say?',
    'What should section 3 explain?',
  ]);
  const attempts = await run(page, 'answerBatchProof');
  expect(attempts).toHaveLength(2);
  for (const attempt of attempts) {
    expect(attempt.selection.thread).toBe(threads[0].threadId);
    expect([...attempt.selection.messageIds].sort()).toEqual(
      threads.map((thread) => thread.comments[0].messageId).sort(),
    );
    expect(attempt.selection.comment).toBe(
      'Answers to 3 questions:\nWhich heading should we use?: Keep it\nWhat should the introduction say?: A clear introduction\nWhat should section 3 explain?: Explain the tradeoffs',
    );
  }
  await expect(page.locator('.reply-announcement')).toHaveCount(0);
  await expect(live).toHaveCount(1);
  await page.evaluate(() => {
    const region = document.querySelector('#ask-page-fixture .tmt-ui-notification-status')!;
    const events: string[] = [];
    Object.assign(window, { batchAnnouncements: events });
    new MutationObserver(() => {
      const value = region.textContent;
      if (value) events.push(value);
    }).observe(region, { childList: true, characterData: true, subtree: true });
  });
  await run(page, 'questionBatchAsk', 'replied');
  await expect(
    card(page).getByText('Asked Agent 1, Agent 2 with 3 answers.', { exact: true }),
  ).toBeVisible();
  await expect(card(page).getByTestId('ask-reply')).toHaveCount(0);
  // One reply per recipient is present in the canonical panel, never once per answer.
  await expect(group.getByTestId('ask-reply')).toHaveCount(2);
  await expect(notifications).toHaveCount(2);
  await expect(live).toContainText('I read the answers: Agent 2.');
  const announcements = await page.evaluate(
    () => (window as unknown as { batchAnnouncements: string[] }).batchAnnouncements,
  );
  for (const name of ['Agent 1', 'Agent 2'])
    expect(
      announcements.filter((value) => value.includes(`I read the answers: ${name}.`)),
    ).toHaveLength(1);
  // The same admitted batch replies cannot enter the shared queue twice.
  await run(page, 'releaseQuestionView');
  await card(page).getByRole('button', { name: 'Open reply · Q1', exact: true }).click();
  await expect(group.locator('.question-line').nth(0)).toHaveAttribute('aria-expanded', 'true');
  await expect(card(page).getByTestId('ask-entry').first()).toBeFocused();
  await expect(card(page).getByTestId('ask-reply')).toHaveCount(2);
  await card(page).getByTestId('ask-reply').last().scrollIntoViewIfNeeded();
  await expect(notifications).toHaveCount(0);
  expect(
    await page.evaluate(
      () => (window as unknown as { batchAnnouncements: string[] }).batchAnnouncements,
    ),
  ).toEqual(announcements);
  await run(page, 'saveQuestionPage');
  await page.reload();
  await run(page, 'mount', { restoreQuestionPage: true });
  const restored = page.locator('[data-question-group]').first();
  await expect(restored).not.toHaveAttribute('data-detached');
  await expect(restored.getByTestId('ask-reply')).toHaveCount(2);
  await expect(notifications).toHaveCount(0);
  await expect(live).toHaveText('');
  expect((await run(page, 'proof')).sends).toHaveLength(0);
  await restored.locator('.question-line').nth(1).click();
  await expect(
    card(page).getByText('Asked Agent 1, Agent 2 with 3 answers.', { exact: true }),
  ).toBeVisible();
  await expect(card(page).getByTestId('ask-reply')).toHaveCount(0);
  await card(page).getByRole('button', { name: 'Change answer', exact: true }).click();
  await card(page).getByLabel('Your answer', { exact: true }).fill('One ordinary follow-up');
  await restored.getByRole('button', { name: /^Send 1 answer/ }).click();
  await recorded(page, 4);
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(2);
  const followups = await run(page, 'answerBatchProof');
  expect(followups).toHaveLength(2);
  expect(
    followups.every((attempt: { input: { batch?: unknown } }) => attempt.input.batch === undefined),
  ).toBe(true);
});

test('same-named agents have no ambiguous default; explicit chip keys stay distinct in Send and shared copy', async ({
  page,
}) => {
  const agents = [
    { ...destination(), agentName: 'Review', machineName: 'First machine' },
    {
      ...destination(),
      agentName: 'Review',
      machine: id(7),
      agent: id(8),
      machineName: 'Second machine',
    },
  ];
  const group = await mount(page, { count: 2, agents: ['Review', 'Review'] }, { agents });
  await expect(group.locator('.tmt-ui-chip')).toHaveCount(0);
  await group.getByRole('button', { name: 'Add recipient', exact: true }).click();
  await page.getByRole('combobox', { name: 'Find an agent' }).fill('Review');
  await expect(page.getByRole('option')).toHaveCount(2);
  await page.getByRole('option').filter({ hasText: 'First machine' }).click();
  await group.getByRole('button', { name: 'Add recipient', exact: true }).click();
  await page.getByRole('combobox', { name: 'Find an agent' }).fill('Review');
  await expect(page.getByRole('option')).toHaveCount(1);
  await page.getByRole('option').filter({ hasText: 'Second machine' }).click();
  await expect(group.locator('.tmt-ui-chip')).toHaveCount(2);
  // Every recipient is the shared agent chip inside the design-system frame, and the option list too.
  await expect(group.locator('.tmt-ui-chip .agent-chip')).toHaveCount(2);
  await expect(group.getByRole('group', { name: 'Answer recipients' })).toContainText('Review');
  await card(page).getByLabel('Keep it', { exact: true }).check();
  await group.locator('.question-line').nth(1).click();
  await card(page).getByLabel('Your answer', { exact: true }).fill('Ask both chosen identities');
  await group
    .getByRole('button', { name: 'Send 2 answers · to @Review, @Review', exact: true })
    .click();
  await recorded(page, 2);
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(2);
  const attempts = await run(page, 'answerBatchProof');
  expect(
    attempts.map(
      (attempt: { input: { destination: { machine: string; agent: string } } }) =>
        `${attempt.input.destination.machine}:${attempt.input.destination.agent}`,
    ),
  ).toEqual(agents.map((agent) => `${agent.machine}:${agent.agent}`));
  await openRecorded(group, 1);
  await run(page, 'questionBatchAsk', 'replied');
  await expect(card(page).locator('.question-shared-ask')).toHaveText(
    'Asked Review, Review with 2 answers.',
  );
  await expect(card(page).getByTestId('ask-reply')).toHaveCount(0);
  await group.locator('.question-line').nth(0).click();
  await expect(card(page).locator('.question-shared-ask')).toHaveText(
    'Asked Review, Review with 2 answers.',
  );
  await expect(card(page).getByTestId('ask-reply')).toHaveCount(2);
});

test('partial Send publishes only complete drafts; a later Send leaves earlier comments alone', async ({
  page,
}) => {
  const group = await mount(page, { count: 3 });
  await card(page).getByLabel('Keep it', { exact: true }).check();
  await group.getByRole('button', { name: /^Send 1 answer/ }).click();
  await recorded(page, 1);
  await expect(group.locator('.question-line').nth(0)).toHaveAttribute('aria-expanded', 'false');
  const before: readonly ThreadView[] = await run(page, 'discussionProof');
  await group.locator('.question-line').nth(2).click();
  await card(page).getByLabel('Your answer', { exact: true }).fill('Only this next answer');
  await group.getByRole('button', { name: /^Send 1 answer/ }).click();
  await recorded(page, 2);
  const after: readonly ThreadView[] = await run(page, 'discussionProof');
  expect(after.find((thread) => thread.ref.id === before[0].ref.id)?.comments).toEqual(
    before[0].comments,
  );
  expect(after).toHaveLength(2);
  await group.locator('.question-line').nth(1).click();
  await expect(card(page).getByLabel('Your answer', { exact: true })).toHaveValue('');
  await expect(group.getByRole('button', { name: /^Send answers · to/ })).toBeDisabled();
});

for (const recordBeforeFail of [false, true]) {
  test(`an unconfirmed comment keeps its draft and is excluded from the Ask; retry only that answer (late record ${recordBeforeFail})`, async ({
    page,
  }) => {
    const group = await mount(page, { count: 3 }, { failAnswerAt: 1, recordBeforeFail });
    await answerThree(page, group);
    await group.getByRole('button', { name: /^Send 3 answers/ }).click();
    await recorded(page, recordBeforeFail ? 3 : 2);
    await expect(group.locator('.question-line').nth(2)).toHaveAttribute('aria-expanded', 'false');
    await group.locator('.question-line').first().click();
    await expect(group.getByRole('button', { name: /^Send 1 answer/ })).toBeEnabled();
    const before: readonly ThreadView[] = await run(page, 'discussionProof');
    const successful = before.filter(
      (thread) => thread.anchor?.exact !== 'Which heading should we use?',
    );
    const first = await run(page, 'answerBatchProof');
    expect(first).toHaveLength(1);
    expect(first[0].selection.messageIds).toHaveLength(2);
    expect(first[0].selection.thread).toBe(successful[0].threadId);
    expect(first[0].selection.comment).not.toContain('Which heading should we use?');
    await expect(card(page).getByRole('alert')).toHaveText(
      'Could not confirm this answer. Your draft is kept for the next Send.',
    );
    await expect(card(page).getByLabel('Keep it', { exact: true })).toBeChecked();
    if (recordBeforeFail)
      await expect(card(page).locator('.question-answer > p')).toHaveText('Keep it');
    else {
      await expect(card(page).locator('.question-answer')).toHaveCount(0);
      await expect(group.locator('.question-line').first()).toContainText('Waiting for you');
    }
    await group.getByRole('button', { name: /^Send 1 answer/ }).click();
    await recorded(page, recordBeforeFail ? 4 : 3);
    await openRecorded(group, 0);
    const after: readonly ThreadView[] = await run(page, 'discussionProof');
    for (const thread of successful)
      expect(after.find((value) => value.ref.id === thread.ref.id)?.comments).toEqual(
        thread.comments,
      );
    await expect(card(page).locator('.question-answer > p')).toHaveText('Keep it');
    await expect(card(page).getByRole('alert')).toHaveCount(0);
    const attempts = await run(page, 'answerBatchProof');
    expect(attempts).toHaveLength(2);
    expect(attempts[1].input.batch).toBeUndefined();
    expect(attempts[1].selection.messageIds).toHaveLength(1);
  });
}

test('a failed recipient retries the same frozen batch once without republishing comments', async ({
  page,
}) => {
  const group = await mount(page, { count: 3 }, { prepareFailures: 1 });
  await answerThree(page, group);
  await group.getByRole('button', { name: /^Send 3 answers/ }).click();
  await recorded(page, 3);
  await expect(group.getByTestId('recipient-failure')).toHaveCount(1);
  await openRecorded(group, 2);
  const before: readonly ThreadView[] = await run(page, 'discussionProof');
  await group
    .getByTestId('recipient-failure')
    .getByRole('button', { name: 'Ask again', exact: true })
    .click();
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
  await expect(group.getByTestId('recipient-failure')).toHaveCount(0);
  expect(await run(page, 'discussionProof')).toEqual(before);
  const attempts = await run(page, 'answerBatchProof');
  expect(attempts).toHaveLength(1);
  expect([...attempts[0].selection.messageIds].sort()).toEqual(
    before.map((thread) => thread.comments[0].messageId).sort(),
  );
  expect(attempts[0].input.retryOf).toBeNull();
});

test('an admitted refused batch retries its original set for one recipient after reload', async ({
  page,
}) => {
  const group = await mount(
    page,
    { count: 3, agents: ['Agent 1', 'Agent 2', 'Agent 1'] },
    { refuseAsks: 2 },
  );
  await answerThree(page, group);
  await group.getByRole('button', { name: /^Send 3 answers/ }).click();
  await recorded(page, 3);
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(2);
  const original = (await run(page, 'answerBatchProof'))[0];
  const before: readonly ThreadView[] = await run(page, 'discussionProof');
  await run(page, 'questionBatchAsk', 'refused');
  await group.locator('.question-line').nth(0).click();
  await expect(card(page).getByRole('button', { name: 'Ask again', exact: true })).toHaveCount(2);
  await run(page, 'saveQuestionPage');
  await page.reload();
  await run(page, 'mount');
  await run(page, 'restoreQuestionPage');
  await group.locator('.question-line').first().click();
  await expect(card(page).getByRole('button', { name: 'Ask again', exact: true })).toHaveCount(2);
  await card(page).getByRole('button', { name: 'Ask again', exact: true }).first().click();
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
  const retries = await run(page, 'answerBatchProof');
  expect(retries).toHaveLength(1);
  expect(retries[0].input.retryOf).toBe(original.operationId);
  expect(retries[0].input.destination.agent).toBe(original.input.destination.agent);
  expect([...retries[0].selection.messageIds].sort()).toEqual(
    [...original.selection.messageIds].sort(),
  );
  expect(retries[0].selection.thread).toBe(original.selection.thread);
  expect(retries[0].selection.comment).toBe(original.selection.comment);
  expect(await run(page, 'discussionProof')).toEqual(before);
});

test('no recipients posts answers only, and a removed default does not return', async ({
  page,
}) => {
  const group = await mount(page, { count: 3 });
  await group.getByRole('button', { name: 'Remove @Agent 1', exact: true }).click();
  await expect(
    group.getByText('Stays on the page, no one is asked', { exact: true }),
  ).toBeVisible();
  await answerThree(page, group);
  await group.getByRole('button', { name: 'Send 3 answers · to thread only', exact: true }).click();
  await recorded(page, 3);
  expect((await run(page, 'proof')).sends).toHaveLength(0);
  await expect(group.locator('.tmt-ui-chip')).toHaveCount(0);
  await group.locator('.question-line').nth(0).click();
  await card(page).getByRole('button', { name: 'Change answer', exact: true }).click();
  await card(page).getByLabel('Change it', { exact: true }).check();
  await expect(
    group.getByRole('button', { name: 'Send 1 answer · to thread only', exact: true }),
  ).toBeEnabled();
});

test('recipient chips add/remove/Backspace share the cap and never send by themselves', async ({
  page,
}) => {
  const agents = Array.from({ length: 9 }, (_, index) => ({
    ...destination(),
    agent: id(6 + index),
    agentName: `Agent ${index + 1}`,
  }));
  const group = await mount(page, { count: 3 }, { agents });
  for (let index = 2; index <= 8; index++) {
    await group.getByRole('button', { name: 'Add recipient', exact: true }).click();
    const search = page.getByRole('combobox', { name: 'Find an agent' });
    await search.fill(`Agent ${index}`);
    await expect(page.getByRole('option')).toHaveCount(1);
    await search.press('Enter');
  }
  await expect(group.locator('.tmt-ui-chip')).toHaveCount(8);
  await expect(group.getByRole('button', { name: 'Add recipient', exact: true })).toBeDisabled();
  await expect(group.getByText('At most 8', { exact: true })).toBeVisible();
  await group.getByRole('button', { name: 'Remove @Agent 8', exact: true }).press('Enter');
  await expect(group.getByRole('button', { name: 'Remove @Agent 7', exact: true })).toBeFocused();
  await group.getByRole('button', { name: 'Add recipient', exact: true }).click();
  const search = page.getByRole('combobox', { name: 'Find an agent' });
  await search.press('Backspace');
  await expect(group.locator('.tmt-ui-chip')).toHaveCount(6);
  await search.press('Escape');
  expect(await run(page, 'discussionProof')).toHaveLength(0);
  expect((await run(page, 'proof')).sends).toHaveLength(0);
});

test('the composed size refusal has no effects and keeps drafts for a smaller Send', async ({
  page,
}) => {
  const group = await mount(page, { count: 10 });
  await card(page).getByLabel('Keep it', { exact: true }).check();
  for (let index = 1; index < 10; index++) {
    await group.locator('.question-line').nth(index).click();
    await card(page)
      .getByLabel('Your answer', { exact: true })
      .fill(index === 1 ? 'Short answer' : 'x'.repeat(8192));
  }
  await group.getByRole('button', { name: /^Send 10 answers/ }).click();
  await expect(group.getByRole('alert')).toHaveText(
    'These answers are too long to send together. Send fewer answers.',
  );
  expect(await run(page, 'discussionProof')).toHaveLength(0);
  expect((await run(page, 'proof')).sends).toHaveLength(0);
  await expect(card(page).getByLabel('Your answer', { exact: true })).toHaveValue('x'.repeat(8192));
  await card(page).getByLabel('Your answer', { exact: true }).fill('');
  await group.getByRole('button', { name: /^Send 9 answers/ }).click();
  await recorded(page, 9);
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
});

for (const [width, theme] of [
  [1440, 'light'],
  [390, 'dark'],
] as const) {
  for (const darkPage of [false, true]) {
    test(`batch card look ${width} ${theme} page-${darkPage ? 'dark' : 'light'}`, async ({
      page,
    }) => {
      test.setTimeout(60000);
      await page.setViewportSize({ width, height: 1100 });
      const agents = Array.from({ length: 9 }, (_, index) => ({
        ...destination(),
        agent: id(6 + index),
        agentName: index === 2 ? 'Agent with a long review name' : `Agent ${index + 1}`,
      }));
      const remount = async (groupOptions = {}, options = {}) => {
        const surface = await mount(page, { ...groupOptions, darkPage }, options);
        await (
          await pageAction(
            page.locator('#ask-page-fixture'),
            theme === 'dark' ? 'Theme: Dark' : 'Theme: Light',
          )
        ).click();
        return surface;
      };
      const group = await remount(
        { count: 3, darkPage, agents: ['Agent 1', 'Agent 2', 'Agent 1'] },
        { agents },
      );
      const capture = async (state: string) => {
        await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
        await expect(group).not.toHaveAttribute('data-detached');
        await expect
          .poll(() =>
            group.locator('.question-compact-block').evaluate((node) => {
              const box = node.getBoundingClientRect();
              return (
                box.left >= -1 &&
                box.right <= innerWidth + 1 &&
                node.scrollWidth <= node.clientWidth + 1
              );
            }),
          )
          .toBe(true);
        await expect
          .poll(async () => {
            const box = await group.locator('.question-compact-block').boundingBox(),
              below = await page
                .frameLocator('#ask-page-fixture iframe')
                .locator('#after-question')
                .boundingBox();
            // This fixture has a normal paragraph margin after the reserved slot.
            // Await both growth and shrinkage before recording its presentation.
            return (
              !!box &&
              !!below &&
              below.y >= box.y + box.height - 1 &&
              below.y <= box.y + box.height + 40
            );
          })
          .toBe(true);
        const directory = process.env.COLAB_QUESTION_CAPTURE_DIR;
        const states = process.env.COLAB_BATCH_CAPTURE_STATES?.split(',');
        if (directory && (!states || states.includes(state))) {
          mkdirSync(directory, { recursive: true });
          await page.screenshot({
            fullPage: true,
            path: `${directory}/batch-${state}-${width}-${theme}-page-${darkPage ? 'dark' : 'light'}.png`,
          });
        }
      };
      await expect(group.locator('.question-compact-heading')).toContainText('3 of 3 waiting');
      await capture('waiting');
      await card(page).getByLabel('Keep it', { exact: true }).check();
      await group.locator('.question-line').nth(1).click();
      await expect(group.locator('.question-compact-heading')).toContainText('3 of 3 waiting');
      await capture('partial');
      await card(page)
        .getByLabel('Your answer', { exact: true })
        .fill('Make the purpose clear in the first paragraph.');
      await group.locator('.question-line').nth(2).click();
      await card(page)
        .getByLabel('Your answer', { exact: true })
        .fill('Explain the tradeoffs without repeating the introduction.');
      await capture('ready');
      await group.getByRole('button', { name: 'Add recipient', exact: true }).click();
      const search = page.getByRole('combobox', { name: 'Find an agent' });
      await search.fill('long review');
      await expect(page.getByRole('option')).toHaveCount(1);
      await capture('recipient-popup');
      await search.press('Enter');
      await expect(group.locator('.tmt-ui-chip')).toHaveCount(3);
      await capture('long-recipient');
      await run(page, 'pausePrepare');
      await group.getByRole('button', { name: /^Send 3 answers/ }).click();
      await expect(group.getByRole('button', { name: /^Send 3 answers/ })).toBeDisabled();
      await capture('sending');
      await run(page, 'resumePrepare');
      await recorded(page, 3);
      await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(3);
      await run(page, 'questionBatchAsk', 'uncertain');
      await group.locator('.question-line').nth(0).click();
      await expect(group.getByText('Delivery unconfirmed').first()).toBeVisible();
      await capture('ask-uncertain');
      await run(page, 'questionBatchAsk', 'replied');
      await group.locator('.question-line').nth(2).click();
      await expect(
        card(page).getByRole('button', { name: 'Open reply · Q1', exact: true }),
      ).toBeVisible();
      await capture('noncanonical-replied');
      await card(page).getByRole('button', { name: 'Open reply · Q1', exact: true }).click();
      await expect(card(page).getByTestId('ask-entry').first()).toBeFocused();
      await capture('canonical-replied');
      await group.locator('.question-line').nth(1).click();
      await card(page).getByRole('button', { name: 'Change answer', exact: true }).click();
      await expect(card(page).getByLabel('Your answer', { exact: true })).toBeFocused();
      await expect(card(page).locator('.question-answer > p')).toHaveText(
        'Make the purpose clear in the first paragraph.',
      );
      await card(page).getByLabel('Your answer', { exact: true }).fill('A revised introduction');
      await capture('change-answer');
      expect((await run(page, 'proof')).sends).toHaveLength(3);
      await remount({ count: 3, darkPage }, { failAnswerAt: 1 });
      await answerThree(page, group);
      await group.getByRole('button', { name: /^Send 3 answers/ }).click();
      await recorded(page, 2);
      await group.locator('.question-line').nth(0).click();
      await expect(card(page).getByRole('alert')).toHaveText(
        'Could not confirm this answer. Your draft is kept for the next Send.',
      );
      await expect(card(page).locator('.question-answer')).toHaveCount(0);
      await expect(group.locator('.question-line').first()).toContainText('Waiting for you');
      await capture('comment-unconfirmed');
      await remount({ count: 3, darkPage }, { prepareFailures: 1 });
      await answerThree(page, group);
      await group.getByRole('button', { name: /^Send 3 answers/ }).click();
      await expect(group.getByTestId('recipient-failure')).toHaveCount(1);
      await openRecorded(group, 2);
      await capture('ask-failed');
      await group
        .getByTestId('recipient-failure')
        .getByRole('button', { name: 'Ask again', exact: true })
        .click();
      await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
      await expect(group.getByTestId('recipient-failure')).toHaveCount(0);
      await capture('ask-retried');
      await remount(
        { count: 9, darkPage, agents: agents.map((agent) => agent.agentName) },
        { agents },
      );
      await expect(group.locator('.tmt-ui-chip')).toHaveCount(8);
      await expect(
        group.getByRole('button', { name: 'Add recipient', exact: true }),
      ).toBeDisabled();
      await expect(group.getByText('At most 8', { exact: true })).toBeVisible();
      await capture('recipient-cap');
      await remount({ count: 3, darkPage }, { agents: [] });
      await expect(group.locator('.tmt-ui-chip')).toHaveCount(0);
      await card(page).getByLabel('Keep it', { exact: true }).check();
      await expect(
        group.getByRole('button', { name: 'Send 1 answer · to thread only', exact: true }),
      ).toBeEnabled();
      await capture('no-recipients');
      await remount({ count: 1, darkPage });
      await expect(group.locator('.question-line')).toHaveCount(1);
      await expect(group.locator('.question-compact-heading')).toHaveCount(0);
      await card(page).getByLabel('Keep it', { exact: true }).check();
      await expect(group.getByRole('button', { name: /^Send answer/ })).toBeEnabled();
      await capture('one-question');
      await remount({ count: 3, darkPage }, { checking: true });
      await expect(group.getByText('Checking for agents…', { exact: true })).toBeVisible();
      await expect(
        group.getByRole('button', { name: 'Add recipient', exact: true }),
      ).toBeDisabled();
      await capture('directory-loading');
      await remount({ count: 3, darkPage }, { agentsUnavailable: true });
      await expect(group.getByText('Agents are unavailable.', { exact: true })).toBeVisible();
      await expect(group.getByRole('button', { name: 'Try again', exact: true })).toBeEnabled();
      await expect(group.locator('.tmt-ui-chip')).toHaveCount(0);
      await capture('directory-failed');
      await remount({ count: 3, darkPage }, { agents: [{ ...agents[0], online: 'offline' }] });
      await card(page).getByLabel('Keep it', { exact: true }).check();
      await expect(group.getByRole('button', { name: /^Send 1 answer/ })).toBeDisabled();
      await capture('recipient-offline');
      expect(await run(page, 'discussionProof')).toHaveLength(0);
      expect((await run(page, 'proof')).sends).toHaveLength(0);
    });
  }
}
