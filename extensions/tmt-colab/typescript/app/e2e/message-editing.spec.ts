import { expect, test, type Page } from '@playwright/test';
async function proof(page: Page) {
  return page.evaluate(async () => {
    const path = '/test/message-editing-browser.tsx';
    return (await import(path)).proof();
  });
}
async function mount(page: Page) {
  await page.goto('/test/message-editing-browser.html');
  return page.getByRole('combobox', { name: 'Editing fixture' });
}
const undo = process.platform === 'darwin' ? 'Meta+z' : 'Control+z';
const redo = process.platform === 'darwin' ? 'Meta+Shift+z' : 'Control+Shift+z';

test('completion preserves surrounding text, caret, token range and history through parent echoes', async ({
  page,
}) => {
  const input = await mount(page);
  await input.fill('Before @O\nAfter');
  await input.evaluate((node) => {
    const text = document.createTreeWalker(node, NodeFilter.SHOW_TEXT).nextNode()!;
    document.getSelection()!.setBaseAndExtent(text, 9, text, 9);
  });
  await input.press('ArrowDown');
  await input.press('End');
  await input.press('Enter');
  await expect.poll(async () => (await proof(page)).edit.value).toBe('Before @Other agent \nAfter');
  const completed = (await proof(page)).edit;
  expect(completed.mentions[0].range).toEqual({ start: 7, end: 19 });
  await input.pressSequentially('X');
  await expect
    .poll(async () => (await proof(page)).edit.value)
    .toBe('Before @Other agent X\nAfter');
  expect((await proof(page)).edit.mentions).toEqual(completed.mentions);
  await page.getByRole('button', { name: 'Echo', exact: true }).click();
  await input.focus();
  await input.press(undo);
  await expect.poll(async () => (await proof(page)).edit.value).toBe(completed.value);
  await input.press(redo);
  await expect
    .poll(async () => (await proof(page)).edit.value)
    .toBe('Before @Other agent X\nAfter');
  expect((await proof(page)).submitted).toBe(0);
});

test('completion after Shift+Enter accounts for line-break nodes and keeps the caret after the token', async ({
  page,
}) => {
  const input = await mount(page);
  await input.fill('First');
  await input.press('End');
  await input.press('Shift+Enter');
  await input.pressSequentially('@O');
  await input.press('End');
  await input.press('Enter');
  await input.pressSequentially('Follow');
  await expect.poll(async () => (await proof(page)).edit.value).toBe('First\n@Other agent Follow');
  expect((await proof(page)).edit.mentions[0].range).toEqual({ start: 6, end: 18 });
  expect((await proof(page)).submitted).toBe(0);
});

test('close/reopen keeps bound tokens and bytes; token deletion clears recipients and reset clears undo', async ({
  page,
}) => {
  const input = await mount(page);
  await input.fill('@O');
  await input.press('End');
  await input.press('Enter');
  const before = (await proof(page)).edit;
  await page.getByRole('button', { name: 'Close', exact: true }).click();
  await page.getByRole('button', { name: 'Reopen', exact: true }).click();
  expect((await proof(page)).edit).toEqual(before);
  await input.fill('No mention needed');
  expect((await proof(page)).edit.mentions).toEqual([]);
  await page.getByRole('button', { name: 'Reset', exact: true }).click();
  await input.focus();
  await input.press(undo);
  expect((await proof(page)).edit.value).toBe('Reset value');
  expect((await proof(page)).submitted).toBe(0);
});

test('scripted paste chooses plaintext over HTML without dispatch and growth stays bounded', async ({
  page,
}) => {
  const input = await mount(page);
  await input.focus();
  await input.evaluate((node) => {
    const data = new DataTransfer();
    data.setData('text/plain', '<b>literal</b>\n\n😀\n');
    data.setData('text/html', '<b>formatted</b>');
    node.dispatchEvent(
      new ClipboardEvent('paste', { bubbles: true, cancelable: true, clipboardData: data }),
    );
  });
  await expect.poll(async () => (await proof(page)).edit.value).toBe('<b>literal</b>\n\n😀\n');
  await expect(input.locator('b')).toHaveCount(0);
  await input.fill(Array(200).fill('A long message line.').join('\n'));
  const tall = await input.evaluate((node) => ({
    height: node.clientHeight,
    scroll: node.scrollHeight,
    max: parseFloat(getComputedStyle(node).maxHeight),
  }));
  expect(tall.height).toBeLessThanOrEqual(tall.max);
  expect(tall.scroll).toBeGreaterThan(tall.height);
  await input.fill('Short');
  expect(await input.evaluate((node) => node.clientHeight)).toBeLessThan(tall.height);
  expect((await proof(page)).submitted).toBe(0);
});
