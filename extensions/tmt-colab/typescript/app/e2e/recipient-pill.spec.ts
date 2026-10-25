import { expect, test, type Page } from '@playwright/test';
import { destination, id } from '../test/ask-fixtures.js';
const fixture = '/test/annotation-browser.tsx';
async function run(page: Page, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
const alpha = { ...destination(), agentName: 'alpha' };
const beta = { ...destination(), agent: id(7), agentName: 'beta' };
const gamma = { ...destination(), agent: id(8), agentName: 'gamma' };
const resting = { ...destination(), agent: id(9), agentName: 'delta', online: 'offline' as const };

async function mount(page: Page, surface = 'chat', agents = [alpha, beta, gamma, resting]) {
  await page.goto('/');
  await run(page, 'setSurface', surface);
  await run(page, 'setAgents', agents);
  await run(page, 'setCreator', false);
  await run(page, 'setDirectory', 'ok');
  await run(page, 'mount', 'accepted');
  const input = page.getByRole('combobox', { name: 'Message', exact: true });
  await expect(input).toBeVisible();
  await expect(page.locator('.annotation-hint[role="status"]')).not.toContainText('Checking');
  return input;
}
const pill = (page: Page) => page.getByRole('button', { name: /^Who is asked/ });
const chips = (page: Page) => page.locator('.agent-chip[data-variant="chip"]');
const send = (page: Page) => page.getByRole('button', { name: 'Send', exact: true });
const hint = (page: Page) => page.locator('.annotation-hint[role="status"]');

for (const surface of ['annotation', 'reply', 'chat'])
  test(`${surface}: the pill says who is asked and follows the mention chips`, async ({ page }) => {
    const input = await mount(page, surface);
    await expect(pill(page)).toHaveText('Comment only');
    await expect(hint(page)).toHaveText('Comment only, no agent is asked · Enter to send');
    await input.pressSequentially('@alpha ');
    await expect(chips(page)).toHaveCount(1);
    await expect(pill(page)).toHaveText('Ask alpha');
    await expect(hint(page)).toHaveText('Asks alpha · Enter to send');
    await input.pressSequentially('@beta ');
    await expect(pill(page)).toHaveText('Ask alpha, beta');
    await expect(hint(page)).toHaveText('Asks alpha, beta · Enter to send');
    await input.pressSequentially('@gamma ');
    // Two names, then how many more.
    await expect(pill(page)).toHaveText('Ask alpha, beta +1');
    await expect(hint(page)).toHaveText('Asks alpha, beta, gamma · Enter to send');
    // The remove control takes the chip and the pill follows.
    await chips(page).first().hover();
    await page.getByRole('button', { name: /^Remove mention: @alpha/ }).click();
    await expect(pill(page)).toHaveText('Ask beta, gamma');
    // Backspace right after a chip takes the whole chip.
    await input.evaluate((node) => {
      node.focus();
      document.getSelection()!.selectAllChildren(node);
      document.getSelection()!.collapseToEnd();
    });
    await input.press('Backspace');
    await input.press('Backspace');
    await expect(chips(page)).toHaveCount(1);
    await expect(pill(page)).toHaveText('Ask beta');
  });

test('the pill opens the agent list; a choice adds or takes off the mention chip and keeps the text', async ({
  page,
}) => {
  const input = await mount(page);
  await input.pressSequentially('Please look');
  await pill(page).click();
  const list = page.getByRole('listbox');
  await expect(list).toBeVisible();
  // One row per agent with its presence; an agent that cannot be asked cannot be added.
  await expect(list.getByRole('option')).toHaveCount(4);
  await expect(page.getByRole('option', { name: /@delta/ })).toBeDisabled();
  await expect(page.getByRole('option', { name: /@alpha/ })).toContainText('Online');
  await expect(page.getByRole('option', { name: /@delta/ })).toContainText('Offline');
  await page.getByRole('option', { name: /@beta/ }).click();
  await expect(list).toHaveCount(0);
  await expect(input).toHaveText('@beta Please look', { useInnerText: true });
  await expect(pill(page)).toHaveText('Ask beta');
  // The writer carries on typing.
  await expect(input).toBeFocused();
  await pill(page).click();
  await expect(page.getByRole('option', { name: /@beta/ })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  await page.getByRole('option', { name: /@beta/ }).click();
  await expect(input).toHaveText('Please look', { useInnerText: true });
  await expect(pill(page)).toHaveText('Comment only');
  expect(await run(page, 'proof')).toMatchObject({ writes: 0, preparations: 0, commits: 0 });
});

test('the pill list is keyboard operable: open, move, choose, close', async ({ page }) => {
  await mount(page);
  await pill(page).focus();
  await pill(page).press('Enter');
  await expect(page.getByRole('listbox')).toBeVisible();
  await expect(pill(page)).toHaveAttribute('aria-expanded', 'true');
  await pill(page).press('ArrowDown');
  await pill(page).press('Enter');
  await expect(chips(page)).toHaveCount(1);
  await expect(pill(page)).toHaveText('Ask beta');
  await pill(page).focus();
  await pill(page).press('Enter');
  await expect(page.getByRole('listbox')).toBeVisible();
  await pill(page).press('Escape');
  await expect(page.getByRole('listbox')).toHaveCount(0);
  await expect(chips(page)).toHaveCount(1);
});

test('the Mention button types the @ that opens the same list at the caret', async ({ page }) => {
  const input = await mount(page);
  await input.pressSequentially('Hi ');
  await page.getByRole('button', { name: 'Mention an agent', exact: true }).click();
  await expect(input).toBeFocused();
  await expect(input).toHaveText('Hi @', { useInnerText: true });
  await expect(page.getByRole('listbox')).toBeVisible();
  await input.press('ArrowDown');
  await input.press('Enter');
  await expect(input).toHaveText('Hi @beta ', { useInnerText: true });
  await expect(pill(page)).toHaveText('Ask beta');
  await page.getByRole('button', { name: 'Mention an agent', exact: true }).click();
  await input.press('Escape');
  await expect(page.getByRole('listbox')).toHaveCount(0);
});

test('a name that matches no agent is an unfilled dashed tag, and the pill stays comment only', async ({
  page,
}) => {
  const input = await mount(page);
  await input.pressSequentially('@nobody check this');
  await expect(chips(page)).toHaveCount(0);
  await expect(pill(page)).toHaveText('Comment only');
  await expect(hint(page)).toContainText('No agent named @nobody');
  await expect(hint(page)).toContainText('This posts as a comment.');
  await input.press('Enter');
  await expect.poll(() => run(page, 'proof')).toMatchObject({ writes: 1, preparations: 0 });
});

test('Send rests quiet while empty; Enter sends and Shift+Enter breaks the line', async ({
  page,
}) => {
  const input = await mount(page);
  await expect(send(page)).toBeDisabled();
  const quiet = await send(page).evaluate((node) => getComputedStyle(node).backgroundColor);
  await input.pressSequentially('one');
  await expect(send(page)).toBeEnabled();
  expect(await send(page).evaluate((node) => getComputedStyle(node).backgroundColor)).not.toBe(
    quiet,
  );
  await input.press('Shift+Enter');
  await input.pressSequentially('two');
  await expect(input).toHaveText('one\ntwo', { useInnerText: true });
  expect(await run(page, 'proof')).toMatchObject({ writes: 0 });
  await input.press('Enter');
  await expect.poll(() => run(page, 'proof')).toMatchObject({ writes: 1 });
  await expect(send(page)).toBeDisabled();
});

for (const surface of ['annotation', 'reply', 'chat'])
  test(`${surface}: focus draws a ring and an edge without changing the field's size`, async ({
    page,
  }) => {
    const input = await mount(page, surface);
    await page.locator('body').click({ position: { x: 2, y: 2 } });
    const box = page.locator('.annotation-compose .rich-input-surface');
    const resting = (await box.boundingBox())!;
    const restingEdge = await box.evaluate((node) => getComputedStyle(node).borderTopColor);
    await input.focus();
    const focused = (await box.boundingBox())!;
    expect([focused.width, focused.height]).toEqual([resting.width, resting.height]);
    expect(await box.evaluate((node) => getComputedStyle(node).borderTopColor)).not.toBe(
      restingEdge,
    );
    expect(await box.evaluate((node) => getComputedStyle(node).boxShadow)).toMatch(
      /0px 0px 0px 3px/,
    );
    await expect(box).toHaveCSS('border-top-width', '1px');
  });
