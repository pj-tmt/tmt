import { expect, test, type Page } from '@playwright/test';
const fixture = '/test/chat-input-browser.tsx';
async function run(page: Page, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
async function composer(page: Page, options: object) {
  await page.goto('/');
  await run(page, 'mountComposer', options);
  return page.getByRole('combobox', { name: 'Message to agent', exact: true });
}

test('the composer starts with the agent that replied last, else the publisher, else the only agent', async ({
  page,
}) => {
  let input = await composer(page, {
    agents: ['Alpha', 'Beta'],
    publisher: 'Alpha',
    replier: 'Beta',
  });
  await expect(input).toHaveValue('@Beta ');
  input = await composer(page, { agents: ['Alpha', 'Beta'], publisher: 'Alpha' });
  await expect(input).toHaveValue('@Alpha ');
  input = await composer(page, { agents: ['Solo'] });
  await expect(input).toHaveValue('@Solo ');
  // Two reachable agents and nothing to choose by: no guess.
  input = await composer(page, { agents: ['Alpha', 'Beta'] });
  await expect(input).toHaveValue('');
  // A name that is not reachable falls through to the next source.
  input = await composer(page, { agents: ['Alpha', 'Beta'], publisher: 'Alpha', replier: 'Gone' });
  await expect(input).toHaveValue('@Alpha ');
});

test('Enter without a recipient says what to add, opens the list and sends nothing', async ({
  page,
}) => {
  const input = await composer(page, { agents: ['Alpha', 'Beta'] });
  await input.fill('hello there');
  await input.press('Enter');
  await expect(page.getByRole('alert')).toHaveText('Add @agent to choose who gets this');
  await expect(input).toHaveAttribute('aria-expanded', 'true');
  expect((await run(page, 'proof')).createChats).toBe(0);
  await input.fill('@Alpha ');
  await expect(page.getByRole('alert')).toHaveCount(0);
  await input.press('Enter');
  await expect(page.getByRole('alert')).toHaveText('Write a message for @Alpha');
  expect((await run(page, 'proof')).createChats).toBe(0);
});

test('Enter that ends an IME composition commits it and only the next Enter sends', async ({
  page,
}) => {
  const input = await composer(page, { agents: ['Alpha'] });
  await input.focus();
  const client = await page.context().newCDPSession(page);
  await client.send('Input.imeSetComposition', {
    text: '你好',
    selectionStart: 2,
    selectionEnd: 2,
  });
  await input.press('Enter');
  await client.send('Input.insertText', { text: '你好' });
  expect((await run(page, 'proof')).createChats).toBe(0);
  await expect(input).toHaveValue(/^@Alpha 你好/);
  await input.press('Enter');
  await expect.poll(async () => (await run(page, 'proof')).createChats).toBe(1);
});

test('the row menu appears on hover and focus without layout space, and keeps focus predictable', async ({
  page,
}) => {
  await page.goto('/');
  await run(page, 'mountMenu');
  const row = page.getByTestId('row');
  const trigger = page.getByRole('button', { name: 'Message actions' });
  const before = await row.boundingBox();
  await expect(trigger).toHaveCSS('opacity', '0');
  await row.hover();
  await expect(trigger).toHaveCSS('opacity', '1');
  expect(await row.boundingBox()).toEqual(before);
  // The trigger sits at the row's right edge, out of the text flow and clear of the header line.
  const box = (await trigger.boundingBox())!;
  expect(box.x + box.width).toBeGreaterThan(before!.x + before!.width - 24);
  expect(box.y).toBeGreaterThan(before!.y + before!.height / 2);
  await trigger.click();
  const items = page.getByRole('menuitem');
  await expect(items).toHaveText(['Edit', 'Delete']);
  await expect(items.first()).toBeFocused();
  await page.keyboard.press('ArrowDown');
  await expect(items.nth(1)).toBeFocused();
  await page.keyboard.press('ArrowDown');
  await expect(items.first()).toBeFocused();
  await page.keyboard.press('End');
  await expect(items.nth(1)).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(items).toHaveCount(0);
  await expect(trigger).toBeFocused();
  await trigger.press('ArrowDown');
  await items.nth(1).click();
  await expect(items).toHaveCount(0);
  await expect(trigger).toBeFocused();
  expect((await run(page, 'proof')).selected).toEqual(['delete']);
});
