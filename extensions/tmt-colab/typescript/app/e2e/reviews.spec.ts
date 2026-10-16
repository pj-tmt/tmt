import { expect, test, type Page } from '@playwright/test';

const fixture = '/test/ask-page-browser.tsx';
async function run(page: Page, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
const frame = (page: Page) => page.frameLocator('#ask-page-fixture iframe');
const section = (page: Page, index = 1) =>
  frame(page)
    .locator('tmt-review')
    .filter({ hasText: `Content of section ${index}.` });

async function mount(page: Page, options = {}) {
  await page.goto('/');
  await run(page, 'mount');
  await run(page, 'review', options);
  await expect(page.locator('[data-inline-review]').first()).not.toHaveAttribute('data-detached');
  return page.locator('.review-card').first();
}
async function sectionThread(page: Page) {
  const threads = await run(page, 'discussionProof');
  return threads.filter((thread: { anchor: { exact: string } | null }) =>
    thread.anchor?.exact.startsWith('Section'),
  );
}

test('a waiting section offers Confirm and Needs change, and the head counts it', async ({
  page,
}) => {
  const card = await mount(page, { sections: 2 });
  await expect(card).toHaveAttribute('data-review-state', 'waiting');
  await expect(card.getByText('Waiting for your confirmation')).toBeVisible();
  await expect(card.getByRole('button', { name: 'Confirm and close', exact: true })).toBeEnabled();
  await expect(card.getByRole('button', { name: 'Needs change…', exact: true })).toBeEnabled();
  await expect(card.getByText('Goes to @Agent 1')).toBeVisible();
  const chip = page.getByRole('status', { name: '0 of 2 review sections closed' });
  await expect(chip).toHaveText('✓ 0 / 2 closed');
  await expect(section(page)).toHaveAttribute('data-colab-review-closed', 'false');
  expect(await sectionThread(page)).toHaveLength(0);
});

test('Confirm and close records one comment, resolves the thread and fades the section in place', async ({
  page,
}) => {
  const card = await mount(page, { sections: 2 });
  // Synthetic activation has no authority.
  await card
    .getByRole('button', { name: 'Confirm and close', exact: true })
    .evaluate((node) => (node as HTMLButtonElement).click());
  expect(await sectionThread(page)).toHaveLength(0);
  expect((await run(page, 'proof')).actions).toEqual([]);

  await card.getByRole('button', { name: 'Confirm and close', exact: true }).click();
  await expect(card).toHaveAttribute('data-review-state', 'closed');
  const threads = await sectionThread(page);
  expect(threads).toHaveLength(1);
  expect(threads[0].anchor.exact).toBe('Section 1');
  expect(threads[0].comments).toHaveLength(1);
  expect(threads[0].comments[0].body).toBe(
    'Confirmed “Section 1”\nPage: Live shared page\n@Agent 1',
  );
  expect((await run(page, 'proof')).actions).toEqual(['status:resolve']);
  await expect(page.getByRole('status', { name: '1 of 2 review sections closed' })).toHaveText(
    '✓ 1 / 2 closed',
  );
  // Faded, never collapsed or hidden: its content stays visible and selectable.
  const closed = section(page);
  await expect(closed).toHaveAttribute('data-colab-review-closed', 'true');
  await expect(closed.getByText('Content of section 1.')).toBeVisible();
  expect(await closed.evaluate((node) => Number(getComputedStyle(node).opacity))).toBeLessThan(1);
  expect(await closed.evaluate((node) => getComputedStyle(node).display)).toBe('block');
  await expect(section(page, 2)).toHaveAttribute('data-colab-review-closed', 'false');
  // The closed band offers only Reopen.
  await expect(card.getByRole('button', { name: 'Confirm and close' })).toHaveCount(0);
  await card.getByRole('button', { name: 'Reopen', exact: true }).click();
  await expect(card).toHaveAttribute('data-review-state', 'waiting');
  await expect(section(page)).toHaveAttribute('data-colab-review-closed', 'false');
  expect((await run(page, 'proof')).actions).toEqual(['status:resolve', 'status:reopen']);
  await expect(page.getByRole('status', { name: '0 of 2 review sections closed' })).toBeVisible();
});

test('Confirm keeps an optional one-line note in the closing comment', async ({ page }) => {
  const card = await mount(page);
  await card.getByRole('button', { name: 'Add a note', exact: true }).click();
  await card.getByLabel('Note (optional)', { exact: true }).fill('Looks right to me');
  await card.getByRole('button', { name: 'Confirm and close', exact: true }).click();
  await expect(card).toHaveAttribute('data-review-state', 'closed');
  await expect(card.getByText('Looks right to me')).toBeVisible();
  expect((await sectionThread(page))[0].comments[0].body).toBe(
    'Confirmed “Section 1”: Looks right to me\nPage: Live shared page\n@Agent 1',
  );
});

test('Needs change adds a comment and an Ask, keeps the thread open, and a reply returns it to waiting', async ({
  page,
}) => {
  const card = await mount(page);
  await card.getByRole('button', { name: 'Needs change…', exact: true }).click();
  const send = card.getByRole('button', { name: 'Send', exact: true });
  await expect(send).toBeDisabled();
  await card.getByLabel('What needs to change', { exact: true }).fill('Say what the page is for.');
  await send.click();
  await expect(card).toHaveAttribute('data-review-state', 'changes');
  await expect(card.getByText('Changes requested')).toBeVisible();
  await expect(card.locator('.review-note')).toHaveText('Say what the page is for.');
  // The composer empties once the Ask settled.
  await expect(card.getByLabel('What needs to change', { exact: true })).toHaveCount(0);
  const threads = await sectionThread(page);
  expect(threads).toHaveLength(1);
  expect(threads[0].comments[0].body).toBe(
    'Needs change to “Section 1”: Say what the page is for.\nPage: Live shared page\n@Agent 1',
  );
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
  // No status action: the thread stays open and the count does not move.
  expect((await run(page, 'proof')).actions).toEqual([]);
  await expect(page.getByRole('status', { name: '0 of 1 review sections closed' })).toBeVisible();
  await expect(section(page)).toHaveAttribute('data-colab-review-closed', 'false');
  await run(page, 'questionAsk', 'replied');
  await expect(card).toHaveAttribute('data-review-state', 'waiting');
  await expect(card.getByRole('button', { name: 'Confirm and close', exact: true })).toBeEnabled();
});

test('an agent resolving the thread does not close the section; Confirm reopens, then resolves', async ({
  page,
}) => {
  const card = await mount(page);
  await card.getByRole('button', { name: 'Needs change…', exact: true }).click();
  await card.getByLabel('What needs to change', { exact: true }).fill('Fix the heading.');
  await card.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(card).toHaveAttribute('data-review-state', 'changes');
  await run(page, 'agentResolves');
  // The thread is resolved with an agent label: the section stays open and the count is 0.
  await expect(card).not.toHaveAttribute('data-review-state', 'closed');
  await expect(page.getByRole('status', { name: '0 of 1 review sections closed' })).toBeVisible();
  await expect(section(page)).toHaveAttribute('data-colab-review-closed', 'false');
  await card.getByRole('button', { name: 'Confirm and close', exact: true }).click();
  await expect(card).toHaveAttribute('data-review-state', 'closed');
  expect((await run(page, 'proof')).actions).toEqual(['status:reopen', 'status:resolve']);
  await expect(page.getByRole('status', { name: '1 of 1 review sections closed' })).toBeVisible();
});

test('page HTML cannot close a section, count or draw controls', async ({ page }) => {
  await page.goto('/');
  await run(page, 'mount');
  await run(page, 'review', { forged: true });
  const card = page.locator('.review-card').first();
  await expect(card).toHaveAttribute('data-review-state', 'waiting');
  // The page authored `data-colab-review-closed="true"`; the frame follows the parent's state.
  await expect(section(page)).toHaveAttribute('data-colab-review-closed', 'false');
  // Authored text that looks like a status changes nothing in the parent.
  await expect(frame(page).getByText('✓ Closed by the author')).toBeVisible();
  await expect(page.getByRole('status', { name: '0 of 1 review sections closed' })).toBeVisible();
  // Page-authored buttons in the frame cannot reach the parent's action.
  await frame(page).getByRole('button', { name: 'Confirm and close' }).click();
  expect(await sectionThread(page)).toHaveLength(0);
  expect((await run(page, 'proof')).actions).toEqual([]);
});

test('duplicate and nested sections supply no controls', async ({ page }) => {
  await page.goto('/');
  await run(page, 'mount');
  await run(page, 'review', { placement: 'duplicate' });
  await expect(page.locator('.review-card')).toHaveCount(0);
  await expect(page.getByRole('status', { name: /review sections closed/ })).toHaveCount(0);
  await run(page, 'review', { placement: 'nested' });
  await expect(page.locator('.review-card')).toHaveCount(1);
  await expect(page.getByRole('status', { name: '0 of 1 review sections closed' })).toBeVisible();
});

test('answering the questions inside a section closes it in the same Send', async ({ page }) => {
  const card = await mount(page, { question: true });
  await expect(card).toHaveAttribute('data-review-state', 'waiting');
  const question = page.locator('.question-group-card');
  await question.getByLabel('Your answer', { exact: true }).fill('Use a clearer heading.');
  await question.getByRole('button', { name: /^Send answer/ }).press('Enter');
  await expect(card).toHaveAttribute('data-review-state', 'closed');
  const threads = await run(page, 'discussionProof');
  const answers = threads.filter((thread: { anchor: { exact: string } }) =>
    thread.anchor.exact.startsWith('Which heading'),
  );
  const sections = threads.filter((thread: { anchor: { exact: string } }) =>
    thread.anchor.exact.startsWith('Section'),
  );
  expect(answers).toHaveLength(1);
  expect(sections).toHaveLength(1);
  expect(sections[0].comments[0].body).toBe('Answered “Section 1”\nPage: Live shared page');
  expect((await run(page, 'proof')).actions).toEqual(['status:resolve']);
  await expect(page.getByRole('status', { name: '1 of 1 review sections closed' })).toBeVisible();
});

test('an archived page shows section state and count but no controls', async ({ page }) => {
  const card = await mount(page);
  await card.getByRole('button', { name: 'Confirm and close', exact: true }).click();
  await expect(card).toHaveAttribute('data-review-state', 'closed');
  await run(page, 'archive');
  await expect(card.getByRole('button', { name: 'Reopen' })).toHaveCount(0);
  await expect(card.locator('.review-state strong')).toHaveText('Closed');
  await expect(page.getByRole('status', { name: '1 of 1 review sections closed' })).toBeVisible();
});

test('a Resolve that did not land leaves the section waiting; the retry publishes only the Resolve', async ({
  page,
}) => {
  const card = await mount(page);
  await run(page, 'failStatusOnce');
  await card.getByRole('button', { name: 'Confirm and close', exact: true }).click();
  await expect(card.getByRole('alert')).toContainText('Your comment is saved');
  await expect(card).toHaveAttribute('data-review-state', 'waiting');
  expect((await run(page, 'proof')).actions).toEqual([]);
  await expect(page.getByRole('status', { name: '0 of 1 review sections closed' })).toBeVisible();
  await card.getByRole('button', { name: 'Confirm and close', exact: true }).click();
  await expect(card).toHaveAttribute('data-review-state', 'closed');
  // The confirmation is recorded once, not twice.
  const threads = await sectionThread(page);
  expect(threads).toHaveLength(1);
  expect(threads[0].comments).toHaveLength(1);
  expect((await run(page, 'proof')).actions).toEqual(['status:resolve']);
});

test('concurrent first actions make sibling threads and every tab agrees on one canonical thread', async ({
  page,
}) => {
  const card = await mount(page);
  await card.getByRole('button', { name: 'Needs change…', exact: true }).click();
  await card.getByLabel('What needs to change', { exact: true }).fill('Fix the heading.');
  await card.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(card).toHaveAttribute('data-review-state', 'changes');
  // Another reader confirmed at the same time: its thread orders first, so it is canonical.
  await run(page, 'reviewSibling', {
    body: 'Confirmed “Section 1”\nPage: Live shared page',
    resolved: 'person',
    writer: 2,
  });
  await expect(card).toHaveAttribute('data-review-state', 'closed');
  await expect(page.getByRole('status', { name: '1 of 1 review sections closed' })).toBeVisible();
  // An agent-labelled sibling never closes anything.
  await run(page, 'reviewSibling', {
    body: 'Confirmed “Section 1”\nPage: Live shared page',
    resolved: 'agent',
    writer: 1,
  });
  await expect(card).not.toHaveAttribute('data-review-state', 'closed');
  await expect(page.getByRole('status', { name: '0 of 1 review sections closed' })).toBeVisible();
});

test('a group inside a section closes it only when every question is answered', async ({
  page,
}) => {
  const card = await mount(page, { group: true });
  const group = page.locator('.question-group-card');
  const active = page.locator('.question-card:visible').first();
  await group.getByRole('tab', { name: /Q1/ }).click();
  await active.getByLabel('Your answer', { exact: true }).fill('First answer.');
  await group.getByRole('button', { name: /^Send/ }).press('Enter');
  await expect(group.locator('.question-state').first()).toContainText('answered');
  // One of two answered: the section stays open and nothing was resolved.
  await expect(card).toHaveAttribute('data-review-state', 'waiting');
  expect((await run(page, 'proof')).actions).toEqual([]);
  await group.getByRole('tab', { name: /Q2/ }).click();
  await active.getByLabel('Your answer', { exact: true }).fill('Second answer.');
  await group.getByRole('button', { name: /^Send/ }).press('Enter');
  await expect(card).toHaveAttribute('data-review-state', 'closed');
  expect((await run(page, 'proof')).actions).toEqual(['status:resolve']);
});

test('the controls wait for the asking agent to resolve, then name it', async ({ page }) => {
  await page.goto('/');
  await run(page, 'mount', { holdQuestionDirectory: true });
  await run(page, 'review');
  const card = page.locator('.review-card').first();
  await expect(card.getByRole('button', { name: 'Confirm and close', exact: true })).toBeDisabled();
  await run(page, 'finishQuestionDirectory');
  await expect(card.getByRole('button', { name: 'Confirm and close', exact: true })).toBeEnabled();
  await expect(card.getByText('Goes to @Agent 1')).toBeVisible();
});

test('an unknown asking agent keeps the comment on the page and says so', async ({ page }) => {
  const card = await mount(page, { agent: 'Missing agent' });
  await expect(card.getByText('Stays on the page, no one is asked')).toBeVisible();
  await card.getByRole('button', { name: 'Confirm and close', exact: true }).click();
  await expect(card).toHaveAttribute('data-review-state', 'closed');
  expect((await sectionThread(page))[0].comments[0].body).toBe(
    'Confirmed “Section 1”\nPage: Live shared page',
  );
});

test('only a thread on the whole heading is the section thread; a partial note is an ordinary annotation', async ({
  page,
}) => {
  const card = await mount(page);
  await card.getByRole('button', { name: 'Needs change…', exact: true }).click();
  await card.getByLabel('What needs to change', { exact: true }).fill('Fix the heading.');
  await card.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(card).toHaveAttribute('data-review-state', 'changes');
  // A resolved confirmation-looking thread on part of the heading, and one inside the content.
  await run(page, 'reviewSibling', {
    body: 'Confirmed “Section 1”\nPage: Live shared page',
    resolved: 'person',
    writer: 2,
    anchor: { exact: 'Section', prefix: '', suffix: '' },
  });
  await run(page, 'reviewSibling', {
    body: 'Confirmed “Section 1”\nPage: Live shared page',
    resolved: 'person',
    writer: 1,
    anchor: { exact: 'Content of section 1.', prefix: '', suffix: '' },
  });
  await expect(card).toHaveAttribute('data-review-state', 'changes');
  await expect(page.getByRole('status', { name: '0 of 1 review sections closed' })).toBeVisible();
});
