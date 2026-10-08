import { expect, test, type Page } from '@playwright/test';
import { destination, id } from '../test/ask-fixtures.js';
const fixture = '/test/annotation-browser.tsx';
async function run(page: Page, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
const agents = [
  { ...destination(), agentName: 'alpha' },
  { ...destination(), agent: id(9), agentName: 'beta', presence: 'offline' as const },
];
async function mount(
  page: Page,
  options: {
    surface?: string;
    mode?: string;
    creator?: boolean;
    directory?: string;
    agents?: typeof agents;
  } = {},
) {
  await page.goto('/');
  await run(page, 'setSurface', options.surface ?? 'annotation');
  await run(page, 'setAgents', options.agents ?? agents);
  await run(page, 'setCreator', options.creator ?? false);
  await run(page, 'setDirectory', options.directory ?? 'ok');
  await run(page, 'mount', options.mode ?? 'accepted');
  const input = page.getByRole('combobox', { name: 'Message', exact: true });
  await expect(input).toBeVisible();
  if (!options.directory)
    await expect(page.locator('.annotation-status-row')).not.toContainText('Checking');
  return input;
}
for (const surface of ['annotation', 'reply', 'chat']) {
  test(`${surface}: no mention posts exactly one comment and zero Asks`, async ({ page }) => {
    const input = await mount(page, { surface });
    await input.pressSequentially('Plain <script>note</script>.');
    await input.press('Enter');
    await expect
      .poll(() => run(page, 'proof'))
      .toMatchObject({ writes: 1, preparations: 0, commits: 1, sends: [] });
  });
}
for (const ending of [' ', ',', 'Enter']) {
  test(`typed exact name binds on ${ending} and dispatches once`, async ({ page }) => {
    const input = await mount(page);
    await input.pressSequentially('@alpha');
    if (ending === 'Enter') await input.press('Enter');
    else {
      await input.pressSequentially(`${ending}Explain.`);
      await page.getByRole('button', { name: 'Send', exact: true }).click();
    }
    await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
    expect(await run(page, 'proof')).toMatchObject({ writes: 1, preparations: 1, commits: 1 });
  });
}
test('one Send fans out once per UUID, including offline, with one recorded comment', async ({
  page,
}) => {
  const input = await mount(page);
  await input.pressSequentially('@alpha @beta @alpha explain');
  await expect(page.locator('.annotation-status-row')).toContainText(
    'Asks @alpha, @beta (offline).',
  );
  await input.press('Enter');
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(2);
  const proof = await run(page, 'proof');
  expect(proof).toMatchObject({ writes: 1, preparations: 2, commits: 1 });
  expect(new Set(proof.sends.map((send: { operationId: string }) => send.operationId)).size).toBe(
    2,
  );
  expect(proof.sends.map((send: { agentId: string }) => send.agentId)).toEqual(
    agents.map((agent) => agent.agent),
  );
  expect((await run(page, 'directoryProof')).reads).toBe(1);
});
test('unknown name stays text with a comment hint and Enter is allowed', async ({ page }) => {
  const input = await mount(page);
  await input.pressSequentially('@missing note');
  await expect(page.locator('.annotation-status-row')).toContainText(
    'No agent named @missing. This posts as a comment.',
  );
  await input.press('Enter');
  await expect.poll(() => run(page, 'proof')).toMatchObject({ writes: 1, sends: [] });
});
test('ambiguous name opens its list without binding or publishing', async ({ page }) => {
  const input = await mount(page, { agents: [agents[0], { ...agents[1], agentName: 'alpha' }] });
  await input.pressSequentially('@alpha');
  await input.press('Enter');
  await expect(input).toHaveAttribute('aria-expanded', 'true');
  await expect(page.getByRole('option')).toHaveCount(2);
  await expect(page.getByRole('button', { name: 'Send', exact: true })).toBeDisabled();
  expect((await run(page, 'proof')).writes).toBe(0);
  await page.getByRole('option').first().click();
  await input.pressSequentially('note');
  await input.press('Enter');
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
});
test('creator default is removable, never reinserted on directory replacement, and caret/draft remain', async ({
  page,
}) => {
  const input = await mount(page, { creator: true });
  await expect(input).toHaveText('@alpha ', { useInnerText: true });
  await input.press('ControlOrMeta+A');
  await input.press('Backspace');
  await input.pressSequentially('Kept draft');
  const before = await input
    .evaluate((node) => ({ node, offset: document.getSelection()?.anchorOffset }))
    .then((value) => value.offset);
  await input.evaluate((node) => Object.assign(window, { retainedComposer: node }));
  await run(page, 'setDirectory', 'park');
  await run(page, 'reconnectAsk');
  await expect(page.locator('.annotation-status-row')).toContainText('Checking for agents…');
  await expect(input).toHaveText('Kept draft', { useInnerText: true });
  expect(
    await input.evaluate(
      (node) => (window as unknown as { retainedComposer: Element }).retainedComposer === node,
    ),
  ).toBe(true);
  expect(await input.evaluate(() => document.getSelection()?.anchorOffset)).toBe(before);
  await run(page, 'releaseDirectory');
  await expect(page.locator('.annotation-status-row')).toContainText('Posts as a comment.');
  await input.press('Enter');
  await expect.poll(() => run(page, 'proof')).toMatchObject({ writes: 1, sends: [] });
});
test('loading and failed directory fence mention sends, retaining a draft through explicit retry', async ({
  page,
}) => {
  const input = await mount(page, { directory: 'park' });
  await input.pressSequentially('@alpha note');
  await expect(page.getByRole('button', { name: 'Send', exact: true })).toBeDisabled();
  await input.press('Enter');
  expect((await run(page, 'proof')).writes).toBe(0);
  await run(page, 'releaseDirectory', 'fail');
  await expect(page.locator('.annotation-status-row')).toContainText('Agents are unavailable.');
  await run(page, 'setDirectory', 'ok');
  await page.getByRole('button', { name: 'Try again', exact: true }).click();
  await expect(page.locator('.annotation-status-row')).toContainText('Asks @alpha.');
  await expect(input).toHaveText('@alpha note', { useInnerText: true });
  await input.press('Enter');
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
});
test('a ninth distinct mention blocks before comment or effect and repeated mentions count once', async ({
  page,
}) => {
  const nine = Array.from({ length: 9 }, (_, index) => ({
    ...agents[0],
    agent: id(20 + index),
    agentName: `agent-${index}`,
  }));
  const input = await mount(page, { agents: nine });
  await input.pressSequentially(nine.map((agent) => `@${agent.agentName}`).join(' '));
  await expect(page.locator('.annotation-status-row')).toContainText(
    'Mention up to 8 agents per message.',
  );
  await input.press('Enter');
  expect((await run(page, 'proof')).writes).toBe(0);
});
test('synthetic submits and an IME composition Enter cannot publish', async ({ page }) => {
  const input = await mount(page);
  await input.pressSequentially('@alpha note');
  await input.evaluate((node) =>
    node.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })),
  );
  await page
    .getByRole('button', { name: 'Send', exact: true })
    .evaluate((node) => (node as HTMLButtonElement).click());
  expect((await run(page, 'proof')).writes).toBe(0);
  const client = await page.context().newCDPSession(page);
  await client.send('Input.imeSetComposition', { text: 'x', selectionStart: 1, selectionEnd: 1 });
  await input.press('Enter');
  expect((await run(page, 'proof')).writes).toBe(0);
});
test('comment write failure retains draft and prepares no Ask; preparation failure is per recipient', async ({
  page,
}) => {
  const input = await mount(page, { mode: 'write-failure' });
  await input.pressSequentially('@alpha note');
  await input.press('Enter');
  await expect(page.getByRole('alert')).toContainText('could not be recorded');
  await expect(input).toHaveText('@alpha note', { useInnerText: true });
  expect((await run(page, 'proof')).preparations).toBe(0);
  await run(page, 'mount', 'prepare-failure');
  await input.pressSequentially('@alpha @beta note');
  await input.press('Enter');
  await expect(page.getByTestId('recipient-failure')).toHaveCount(2);
  expect(await run(page, 'proof')).toMatchObject({
    writes: 1,
    preparations: 2,
    commits: 1,
    sends: [],
  });
});

test('a preparation failure leaves the sibling delivered once and never retries it', async ({
  page,
}) => {
  const input = await mount(page, { mode: 'partial-preparation' });
  await input.pressSequentially('@alpha @beta explain');
  await input.press('Enter');
  await expect(page.getByTestId('recipient-failure')).toHaveCount(1);
  await expect(page.getByTestId('recipient-failure')).toContainText('@alpha · Not delivered');
  await expect(page.getByTestId('recipient-failure')).toContainText(
    'Mention @alpha in a new message to ask again.',
  );
  await expect(page.getByTestId('ask-entry')).toHaveCount(1);
  const proof = await run(page, 'proof');
  expect(proof).toMatchObject({ writes: 1, preparations: 2, commits: 1 });
  expect(proof.sends).toHaveLength(1);
  expect(proof.sends[0].agentId).toBe(agents[1].agent);
  await input.press('Enter');
  expect((await run(page, 'proof')).sends).toHaveLength(1);
});

test('an ambiguous token in the middle is replaced in place without losing surrounding text', async ({
  page,
}) => {
  const input = await mount(page, { agents: [agents[0], { ...agents[1], agentName: 'alpha' }] });
  await input.pressSequentially('Before @alpha after.');
  await expect(page.getByRole('option')).toHaveCount(2);
  await page.getByRole('option').first().click();
  await expect(input).toHaveText('Before @alpha  after.', { useInnerText: true });
  await input.press('Enter');
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
});

test('removing and undoing multiple tokens changes the visible recipients without a hidden selection', async ({
  page,
}) => {
  const input = await mount(page);
  await input.pressSequentially('@alpha @beta note');
  await expect(page.locator('.annotation-status-row')).toContainText(
    'Asks @alpha, @beta (offline).',
  );
  await input.press('ControlOrMeta+A');
  await input.press('Backspace');
  await expect(page.locator('.annotation-status-row')).toContainText('Posts as a comment.');
  await input.press('ControlOrMeta+Z');
  await expect(input).toHaveText('@alpha @beta note', { useInnerText: true });
  await expect(page.locator('.annotation-status-row')).toContainText(
    'Asks @alpha, @beta (offline).',
  );
  await input.press('Escape');
  expect((await run(page, 'proof')).writes).toBe(0);
});

test('keyboard selection, multiline text and Escape preserve the input focus without effects', async ({
  page,
}) => {
  const input = await mount(page);
  await input.pressSequentially('@be');
  await input.press('ArrowDown');
  await input.press('Enter');
  await expect(input).toHaveText('@beta ', { useInnerText: true });
  await expect(input).toBeFocused();
  expect((await run(page, 'proof')).writes).toBe(0);
  await input.pressSequentially('First line');
  await input.press('Shift+Enter');
  await input.pressSequentially('Second line');
  await expect(input).toHaveText('@beta First line\nSecond line', { useInnerText: true });
  await input.press('Enter');
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
});

test('directory retry keeps keyboard focus and ignores a late result from the replaced binding', async ({
  page,
}) => {
  const input = await mount(page, { directory: 'fail' });
  await input.fill('@alpha Kept draft.');
  const retry = page.getByRole('button', { name: 'Try again', exact: true });
  await run(page, 'setDirectory', 'park');
  await retry.focus();
  await retry.press('Enter');
  await expect(retry).toBeDisabled();
  await expect(retry).toHaveAttribute('aria-busy', 'true');
  await run(page, 'releaseDirectory', 'fail');
  await expect(retry).toBeFocused();
  await run(page, 'setDirectory', 'park');
  await retry.press('Enter');
  await run(page, 'setDirectory', 'ok');
  await run(page, 'reconnectAsk');
  await expect(page.locator('.annotation-status-row')).toContainText('Asks @alpha.');
  await expect(input).toBeFocused();
  await run(page, 'releaseDirectory', 'fail');
  await expect(page.locator('.annotation-status-row')).toContainText('Asks @alpha.');
  await expect(input).toHaveText('@alpha Kept draft.', { useInnerText: true });
  expect((await run(page, 'directoryProof')).reads).toBe(4);
  expect(await run(page, 'proof')).toMatchObject({ writes: 0, preparations: 0, sends: [] });
});

test('Escape closes suggestions before cancelling without recording or dispatching', async ({
  page,
}) => {
  const input = await mount(page);
  await input.pressSequentially('@');
  await expect(input).toHaveAttribute('aria-expanded', 'true');
  await input.press('Escape');
  await expect(input).toHaveAttribute('aria-expanded', 'false');
  await expect(input).toBeVisible();
  await input.press('Escape');
  expect(await run(page, 'proof')).toMatchObject({
    writes: 0,
    preparations: 0,
    closes: 1,
    sends: [],
  });
});

test('accepted recipients reach independent observation deadlines without redispatching', async ({
  page,
}) => {
  const input = await mount(page);
  await page.clock.install();
  await input.fill('@alpha First question.');
  await input.press('Enter');
  const status = page.getByTestId('ask-state');
  await expect(status).toHaveText('Waiting for alpha');
  await page.clock.fastForward(60 * 60 * 1000);
  await input.fill('@beta Second question.');
  await input.press('Enter');
  await expect(status).toHaveCount(2);
  const before = await run(page, 'proof');
  await page.clock.fastForward(60 * 60 * 1000 + 1);
  await expect(status).toHaveText(['@alpha · No reply yet from alpha', '@beta · Waiting for beta']);
  await expect(page.getByRole('button', { name: 'Check again', exact: true })).toHaveCount(2);
  await page.clock.fastForward(60 * 60 * 1000);
  await expect(status).toHaveText([
    '@alpha · No reply yet from alpha',
    '@beta · No reply yet from beta',
  ]);
  expect(await run(page, 'proof')).toEqual(before);
});

test('a rate-limited recipient retains its own refused operation and leaves the sibling delivered once', async ({
  page,
}) => {
  const input = await mount(page, { mode: 'partial-refusal' });
  await input.fill('@alpha @beta Explain.');
  await input.press('Enter');
  const entries = page.getByTestId('ask-entry');
  await expect(entries).toHaveCount(2);
  await expect(entries.nth(0)).toHaveAttribute('data-ledger-state', 'refused');
  await expect(entries.nth(0)).toContainText('Mention @alpha in a new message to ask again.');
  await expect(entries.nth(1)).toHaveAttribute('data-ledger-state', 'accepted');
  await expect(page.getByRole('button', { name: 'Retry', exact: true })).toHaveCount(0);
  expect(await run(page, 'proof')).toMatchObject({ writes: 1, preparations: 2, commits: 1 });
  await input.press('Enter');
  expect((await run(page, 'proof')).sends).toHaveLength(2);
});

test('a pre-adoption Send failure has a local Not delivered label and no invented Ask row', async ({
  page,
}) => {
  const input = await mount(page, { mode: 'no-adoption' });
  await input.fill('@alpha Explain.');
  await input.press('Enter');
  await expect(page.getByTestId('recipient-failure')).toContainText('@alpha · Not delivered');
  await expect(page.getByTestId('recipient-failure')).toContainText(
    'Mention @alpha in a new message to ask again.',
  );
  await expect(page.getByTestId('ask-entry')).toHaveCount(0);
  expect(await run(page, 'proof')).toMatchObject({
    writes: 1,
    preparations: 1,
    commits: 1,
    sends: [],
  });
});

test('creator lookup waits for the directory and never replaces text typed while it loads', async ({
  page,
}) => {
  let input = await mount(page, { creator: true, directory: 'park' });
  await expect(input).toHaveText('', { useInnerText: true });
  await run(page, 'releaseDirectory');
  await expect(input).toHaveText('@alpha ', { useInnerText: true });
  input = await mount(page, { creator: true, directory: 'park' });
  await input.pressSequentially('My own draft.');
  await run(page, 'releaseDirectory');
  await expect(input).toHaveText('My own draft.', { useInnerText: true });
  await expect(page.locator('.annotation-status-row')).toContainText('Posts as a comment.');
});

test('unknown adoption shows per-recipient uncertainty without claiming Not delivered or offering another attempt', async ({
  page,
}) => {
  const input = await mount(page, { mode: 'unknown-adoption' });
  await input.fill('@alpha Explain.');
  await input.press('Enter');
  const failure = page.getByTestId('recipient-failure');
  await expect(failure).toContainText('@alpha · Delivery unconfirmed');
  await expect(failure).not.toContainText('Not delivered');
  await expect(failure).not.toContainText('new message');
  await expect(page.getByTestId('ask-entry')).toHaveCount(0);
  expect(await run(page, 'proof')).toMatchObject({
    writes: 1,
    preparations: 1,
    commits: 1,
    sends: [],
  });
});
