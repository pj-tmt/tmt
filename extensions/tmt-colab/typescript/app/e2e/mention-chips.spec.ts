import { expect, test, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { destination, id } from '../test/ask-fixtures.js';
import { pageAction } from '../test/page-actions.js';

type State = 'online' | 'offline' | 'not-found' | 'same-name';
const fixture = '/test/message-composer-browser.html';
async function mount(page: Page, state: State) {
  await page.goto(fixture);
  await page.evaluate(async (state) => {
    const path = '/test/message-composer-browser.tsx';
    (await import(path)).mountMentions(state);
  }, state);
  const chip = page.locator('.message-mention');
  await expect(chip).toHaveCount(1);
  await expect(chip).toHaveAttribute('data-state', state === 'same-name' ? 'online' : state);
  return page.getByRole('combobox', { name: 'Message', exact: true });
}

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`page chip captures: ${width}px ${theme}`, async ({ page }) => {
      const captures = process.env.COLAB_MENTION_CAPTURE_DIR;
      test.skip(!captures, 'Set COLAB_MENTION_CAPTURE_DIR for the UX evidence matrix.');
      mkdirSync(captures!, { recursive: true });
      await page.setViewportSize({ width, height: 844 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      for (const state of ['online', 'offline', 'not-found', 'same-name'] as const) {
        await page.goto('/');
        const agent = { ...destination(), agentName: 'astra', presence: 'active' as const };
        const agents =
          state === 'not-found'
            ? []
            : state === 'same-name'
              ? [agent, { ...agent, machine: id(12), machineName: 'Office desktop' }]
              : [{ ...agent, presence: state === 'offline' ? 'offline' : 'active' }];
        await page.evaluate(async (agents) => {
          const path = '/test/ask-page-browser.tsx';
          await (await import(path)).mount({ creator: true, agents });
        }, agents);
        const app = page.locator('#ask-page-fixture');
        const toggle = await pageAction(app, 'Chat');
        await toggle.click();
        const chip = page.getByTestId('chat-panel').locator('.message-mention');
        await expect(chip).toHaveAttribute('data-state', state === 'same-name' ? 'online' : state);
        await page.screenshot({ path: `${captures}/page-${width}-${theme}-${state}.png` });
      }
    });
async function proof(page: Page) {
  return page.evaluate(async () => {
    const path = '/test/annotation-browser.tsx';
    const module = await import(path);
    return { ...module.proof(), ...module.editingProof() };
  });
}

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    for (const state of ['online', 'offline', 'not-found', 'same-name'] as const)
      test(`creator chip ${state}: ${width}px ${theme}`, async ({ page }) => {
        await page.setViewportSize({ width, height: 844 });
        await page.emulateMedia({ colorScheme: theme });
        await page.addInitScript((theme) => {
          document.documentElement.dataset.theme = theme;
        }, theme);
        const input = await mount(page, state);
        const chip = page.locator('.message-mention');
        const name = state === 'not-found' ? '@Creator' : '@astra';
        expect((await proof(page)).draft).toBe(`${name} `);
        await expect(chip.locator('.message-mention-name')).toHaveText(name);
        if (state === 'same-name')
          await expect(chip.locator('.message-mention-machine')).toHaveText(' · My machine');
        const geometry = await chip.evaluate((node) => {
          const style = getComputedStyle(node);
          const dot = node.firstElementChild!.getBoundingClientRect();
          const remove = node.querySelector('button')!.getBoundingClientRect();
          const rect = node.getBoundingClientRect();
          return {
            border: style.borderTopWidth,
            radius: style.borderRadius,
            shadow: style.boxShadow,
            borderStyle: style.borderTopStyle,
            ordered: dot.right < remove.left,
            inside: rect.right <= innerWidth && remove.right <= rect.right,
          };
        });
        expect(geometry).toMatchObject({
          border: '1px',
          radius: '0px',
          shadow: 'none',
          ordered: true,
          inside: true,
          borderStyle: state === 'not-found' ? 'dashed' : 'solid',
        });
        const remove = chip.getByRole('button', { name: /^Remove mention:/ });
        await remove.focus();
        const tooltip = page.locator('.tmt-ui-icon-action-tooltip:popover-open');
        await expect(tooltip).toContainText(name);
        await expect(tooltip).toContainText(
          state === 'offline' ? 'Offline' : state === 'not-found' ? 'Not found' : 'Online',
        );
        await input.focus();
        const captures = process.env.COLAB_MENTION_CAPTURE_DIR;
        if (captures) {
          mkdirSync(captures, { recursive: true });
          await page.screenshot({ path: `${captures}/${width}-${theme}-${state}.png` });
        }
        const send = page.getByRole('button', { name: 'Send', exact: true });
        if (state === 'not-found') {
          await expect(send).toBeDisabled();
          await input.press('Enter');
          expect(await proof(page)).toMatchObject({ writes: 0, preparations: 0, sends: [] });
          await remove.click();
          await expect(chip).toHaveCount(0);
          await input.pressSequentially('Plain comment');
          await send.click();
          await expect.poll(async () => (await proof(page)).writes).toBe(1);
          expect(await proof(page)).toMatchObject({ preparations: 0, sends: [] });
        } else {
          await send.click();
          await expect.poll(async () => (await proof(page)).preparations).toBe(1);
          const sent = await proof(page);
          expect(sent.writes).toBe(1);
          expect(sent.captured[0]).toMatchObject({ body: `${name} ` });
          expect(sent.captured[1]).toMatchObject({ comment: `${name} ` });
          await expect(page.getByTestId('ask-entry')).toContainText('Waiting for approval');
        }
      });

test('removing the creator preserves surrounding text, undo UUID binding and no reseed on replacement', async ({
  page,
}) => {
  const input = await mount(page, 'online');
  await input.press('End');
  await input.pressSequentially('Keep exact café.');
  await page.getByRole('button', { name: /^Remove mention:/ }).click();
  await expect.poll(async () => (await proof(page)).draft).toBe(' Keep exact café.');
  await input.press('ControlOrMeta+Z');
  await expect(page.locator('.message-mention')).toHaveCount(1);
  expect((await proof(page)).draft).toBe('@astra Keep exact café.');
  await page.getByRole('button', { name: /^Remove mention:/ }).click();
  await page.evaluate(async () => {
    const path = '/test/annotation-browser.tsx';
    (await import(path)).reconnectAsk();
  });
  await expect.poll(async () => (await proof(page)).draft).toBe(' Keep exact café.');
  await expect(page.locator('.message-mention')).toHaveCount(0);
  expect(await proof(page)).toMatchObject({ writes: 0, preparations: 0, sends: [] });
});

test('an open list chooses on Shift+Enter and does not send', async ({ page }) => {
  await page.goto(fixture);
  const input = page.getByRole('combobox', { name: 'Message', exact: true });
  await expect(input).toHaveText('');
  await input.pressSequentially('@O');
  await expect(page.getByRole('listbox')).toBeVisible();
  await input.press('Shift+Enter');
  await expect(page.getByRole('listbox')).toBeHidden();
  expect((await proof(page)).draft).toBe('@Other agent ');
  expect(await proof(page)).toMatchObject({ writes: 0, preparations: 0, sends: [] });
});

test('keyboard activation removes a chip without sending, and a missing directory fences its key', async ({
  page,
}) => {
  const input = await mount(page, 'online');
  await page.evaluate(async () => {
    const path = '/test/annotation-browser.tsx';
    const module = await import(path);
    module.setAgents([]);
    module.reconnectAsk();
  });
  await expect(page.locator('.message-mention')).toHaveAttribute('data-state', 'not-found');
  await expect(page.getByRole('button', { name: 'Send', exact: true })).toBeDisabled();
  const remove = page.getByRole('button', { name: /^Remove mention:/ });
  await remove.focus();
  await expect(remove).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.locator('.message-mention')).toHaveCount(0);
  expect((await proof(page)).draft).toBe(' ');
  expect(await proof(page)).toMatchObject({ writes: 0, preparations: 0, sends: [] });
  await expect(input).toBeFocused();
});

test('same-name choices keep the chosen machine UUID; chip controls do not enter copied text', async ({
  page,
}) => {
  const input = await mount(page, 'same-name');
  await input.press('ControlOrMeta+A');
  await input.press('Backspace');
  await input.pressSequentially('@astra');
  await page.getByRole('option', { name: '@astra · Office desktop', exact: true }).click();
  await expect(input.locator('.message-mention-machine')).toHaveText(' · Office desktop');
  await page.context().grantPermissions(['clipboard-read', 'clipboard-write']);
  await input.press('ControlOrMeta+A');
  await input.press('ControlOrMeta+C');
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe('@astra ');
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect.poll(async () => (await proof(page)).preparations).toBe(1);
  const { captured } = await proof(page);
  expect(captured[1]).toMatchObject({
    comment: '@astra ',
    destination: { machine: '00000000-0000-4000-8000-000000000012' },
  });
});

test('a failed first directory read seeds only the admitted creator key and explicit retry restores admission', async ({
  page,
}) => {
  await page.goto(fixture);
  await page.evaluate(async () => {
    const path = '/test/annotation-browser.tsx';
    const fixtures = '/test/ask-fixtures.ts';
    const module = await import(path);
    module.setCreator(true);
    module.setAgents([
      { ...(await import(fixtures)).destination(), agentName: 'astra', presence: 'active' },
    ]);
    module.setDirectory('fail');
    module.mount();
  });
  const input = page.getByRole('combobox', { name: 'Message', exact: true });
  await expect(input.locator('.message-mention-name')).toHaveText('@Creator');
  await expect(page.getByRole('button', { name: 'Send', exact: true })).toBeDisabled();
  await input.press('Enter');
  expect(await proof(page)).toMatchObject({ writes: 0, preparations: 0, sends: [] });
  await page.evaluate(async () => {
    const path = '/test/annotation-browser.tsx';
    (await import(path)).setDirectory('ok');
  });
  await page.getByRole('button', { name: 'Try again', exact: true }).click();
  await expect(input.locator('.message-mention')).toHaveAttribute('data-state', 'online');
  await expect(input.locator('.message-mention')).toHaveAttribute(
    'title',
    '@astra · My machine · Online',
  );
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect.poll(async () => (await proof(page)).preparations).toBe(1);
  expect((await proof(page)).captured[1]).toMatchObject({
    comment: '@Creator ',
    destination: { agentName: 'astra' },
  });
});

test('IME composition with the list open neither navigates, chooses nor sends', async ({
  page,
}) => {
  for (const key of ['ArrowDown', 'ArrowUp', 'Home', 'End', 'Enter', 'Shift+Enter', 'Tab']) {
    // Start every key with a real open list and active composition: Enter/Tab can end native IME input.
    await page.goto(fixture);
    const input = page.getByRole('combobox', { name: 'Message', exact: true });
    await expect(input).toHaveText('');
    await input.pressSequentially('@');
    await expect(page.getByRole('listbox')).toBeVisible();
    const active = await input.getAttribute('aria-activedescendant');
    await page.evaluate(() => {
      Object.assign(window, { compositionKey: undefined });
      document.addEventListener(
        'keydown',
        (event) => {
          if (event.key !== 'Shift')
            Object.assign(window, {
              compositionKey: {
                trusted: event.isTrusted,
                composing: event.isComposing,
                key: event.key,
              },
            });
        },
        true,
      );
    });
    const client = await page.context().newCDPSession(page);
    await client.send('Input.imeSetComposition', {
      text: '@',
      selectionStart: 1,
      selectionEnd: 1,
      replacementStart: 0,
      replacementEnd: 1,
    });
    await page.keyboard.press(key);
    expect(
      await page.evaluate(
        () =>
          (
            window as unknown as {
              compositionKey: { trusted: boolean; composing: boolean; key: string };
            }
          ).compositionKey,
      ),
    ).toEqual({ trusted: true, composing: true, key: key === 'Shift+Enter' ? 'Enter' : key });
    if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(key)) {
      await expect(input).toHaveAttribute('aria-activedescendant', active!);
      await expect(page.getByRole('listbox')).toBeVisible();
    }
    await expect(page.locator('.message-mention')).toHaveCount(0);
    expect(await proof(page)).toMatchObject({ writes: 0, preparations: 0, sends: [] });
    await client.detach();
  }
  // A non-composing control proves that the same keys can choose, then explicitly send.
  await page.goto(fixture);
  const input = page.getByRole('combobox', { name: 'Message', exact: true });
  await expect(input).toHaveText('');
  await input.pressSequentially('@');
  await expect(page.getByRole('listbox')).toBeVisible();
  const active = await input.getAttribute('aria-activedescendant');
  await input.press('ArrowDown');
  await expect(input).not.toHaveAttribute('aria-activedescendant', active!);
  await input.press('Enter');
  await expect(page.getByRole('listbox')).toBeHidden();
  await expect(page.locator('.message-mention')).toHaveCount(1);
  expect(await proof(page)).toMatchObject({ writes: 0, preparations: 0, sends: [] });
  await input.press('Enter');
  await expect.poll(async () => (await proof(page)).writes).toBe(1);
});
