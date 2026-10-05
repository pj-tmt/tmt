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
  return page.getByRole('combobox', { name: 'Message', exact: true });
}

test('the composer starts with the agent that replied last, else the publisher, else the only agent', async ({
  page,
}) => {
  let input = await composer(page, {
    agents: ['Alpha', 'Beta'],
    publisher: 'Alpha',
    replier: 'Beta',
  });
  await expect(input).toHaveText('', { useInnerText: true });
  await expect(page.locator('.annotation-hint')).toContainText(['Enter sends', 'Recipient: Beta']);
  input = await composer(page, { agents: ['Alpha', 'Beta'], publisher: 'Alpha' });
  await expect(input).toHaveText('', { useInnerText: true });
  await expect(page.locator('.annotation-hint').last()).toContainText('Recipient: Alpha');
  input = await composer(page, { agents: ['Solo'] });
  await expect(input).toHaveText('', { useInnerText: true });
  await expect(page.locator('.annotation-hint').last()).toContainText('Recipient: Solo');
  // Two reachable agents and nothing to choose by: no guess.
  input = await composer(page, { agents: ['Alpha', 'Beta'] });
  await expect(input).toHaveText('', { useInnerText: true });
  // A name that is not reachable falls through to the next source.
  input = await composer(page, { agents: ['Alpha', 'Beta'], publisher: 'Alpha', replier: 'Gone' });
  await expect(input).toHaveText('', { useInnerText: true });
  await expect(page.locator('.annotation-hint').last()).toContainText('Recipient: Alpha');
});

test('an ambiguous recipient requires selection without mandatory mention insertion', async ({
  page,
}) => {
  const input = await composer(page, { agents: ['Alpha', 'Beta'] });
  await input.fill('hello there');
  await input.press('Enter');
  await expect(page.getByRole('alert')).toHaveText('Choose a recipient to ask an agent.');
  expect((await run(page, 'proof')).createChats).toBe(0);
  await input.press('ArrowDown');
  await input.press('Enter');
  await expect(input).toHaveText('hello there', { useInnerText: true });
  await expect(page.getByRole('alert')).toHaveCount(0);
  expect((await run(page, 'proof')).createChats).toBe(0);
  await input.press('Enter');
  await expect.poll(async () => (await run(page, 'proof')).createChats).toBe(1);
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
  await expect(input).toHaveText(/^你好/, { useInnerText: true });
  await input.press('Enter');
  await expect.poll(async () => (await run(page, 'proof')).createChats).toBe(1);
});

test('the row menu sits in the byline row, appears on hover and focus, and keeps focus predictable', async ({
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
  // The trigger sits in the byline row at the right edge, after the status.
  const box = (await trigger.boundingBox())!;
  const status = (await page.getByRole('status').boundingBox())!;
  expect(box.x + box.width).toBeGreaterThan(before!.x + before!.width - 24);
  expect(box.x).toBeGreaterThan(status.x + status.width);
  expect(box.y).toBeLessThan(before!.y + 40);
  await trigger.click();
  const items = page.getByRole('menuitem');
  await expect(items).toHaveText(['Edit', 'Delete']);
  // Opens below the trigger with right edges aligned, never over its own message.
  const list = (await page.getByRole('menu').boundingBox())!;
  expect(list.y).toBeGreaterThan(box.y + box.height - 1);
  expect(Math.round(list.x + list.width)).toBe(Math.round(box.x + box.width));
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

test('the row menu flips above its trigger only when it would leave a short scrolling container', async ({
  page,
}) => {
  await page.goto('/');
  await run(page, 'mountMenu', { tight: true });
  const trigger = page.getByRole('button', { name: 'Message actions' });
  await page.getByTestId('row').hover();
  await trigger.click();
  const list = (await page.getByRole('menu').boundingBox())!;
  const box = (await trigger.boundingBox())!;
  expect(list.y + list.height).toBeLessThanOrEqual(box.y + 1);
  await expect(page.getByRole('menu')).toHaveAttribute('data-flip', 'true');
});
