import { expect, test, type Page } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';

const fixture = '/test/message-composer-browser.html';
for (const width of [1440, 390]) {
  for (const theme of ['light', 'dark'] as const) {
    test(`mention choices use available viewport outside the popover: ${width}px ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 844 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      await page.goto(fixture);
      const input = page.getByRole('combobox', { name: 'Message', exact: true });
      await expect(input).toHaveText('', { useInnerText: true });
      const evidence = [];
      for (const position of ['Top', 'Bottom']) {
        await page.getByRole('button', { name: position, exact: true }).click();
        // Use trusted editing keys: fill('') selects the DOM before Lexical admits that selection.
        await input.press('ControlOrMeta+A');
        await input.press('Backspace');
        await expect(input).toHaveText('', { useInnerText: true });
        await input.pressSequentially('@');
        const list = page.getByRole('listbox');
        await expect(list).toBeVisible();
        await expect(list.getByRole('option')).toHaveCount(3);
        // A visible full list must also follow the current field, not retain an old position.
        await expect
          .poll(() =>
            list.evaluate((node) => {
              const menu = node.getBoundingClientRect();
              const field = document.querySelector('[contenteditable]')!.getBoundingClientRect();
              return Math.min(
                Math.abs(menu.top - field.bottom - 2),
                Math.abs(field.top - menu.bottom - 2),
              );
            }),
          )
          .toBeLessThanOrEqual(4);
        const geometry = await list.evaluate((node) => {
          const rect = (element: Element) => {
            const r = element.getBoundingClientRect();
            return { x: r.x, y: r.y, width: r.width, height: r.height, bottom: r.bottom };
          };
          const element = node as HTMLElement;
          const popover = document.querySelector('.annotation-popover')!;
          return {
            viewport: { width: innerWidth, height: innerHeight },
            list: rect(node),
            input: rect(document.querySelector('[contenteditable]')!),
            popover: rect(popover),
            portalParent: node.parentElement!.tagName,
            scrollHeight: element.scrollHeight,
            clientHeight: element.clientHeight,
            maxHeight: getComputedStyle(element).maxHeight,
            popoverOverflow: getComputedStyle(popover).overflowY,
            hitTargets: [...node.querySelectorAll('[role=option]')].every((option) => {
              const box = option.getBoundingClientRect();
              return option.contains(
                document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2),
              );
            }),
          };
        });
        evidence.push({ position, ...geometry });
        const directory = process.env.COLAB_COMPOSER_CAPTURE_DIR;
        if (directory) {
          mkdirSync(directory, { recursive: true });
          await page.screenshot({ path: `${directory}/${width}-${theme}-${position}.png` });
        }
        await input.press('Escape');
      }
      const directory = process.env.COLAB_COMPOSER_CAPTURE_DIR;
      if (directory)
        writeFileSync(`${directory}/${width}-${theme}-geometry.json`, JSON.stringify(evidence));
      // Three short options fit within the viewport at both positions; the popover is not their clip.
      for (const geometry of evidence) {
        expect(geometry.clientHeight).toBeGreaterThanOrEqual(geometry.scrollHeight);
        expect(geometry.hitTargets).toBe(true);
        expect(geometry.list.y).toBeGreaterThanOrEqual(8);
        expect(geometry.list.bottom).toBeLessThanOrEqual(geometry.viewport.height - 8);
      }
    });
  }
}

test('recipient selection keeps multiline surrounding text and performs no effects', async ({
  page,
}) => {
  await page.goto(fixture);
  const input = page.getByRole('combobox', { name: 'Message', exact: true });
  await expect(input).toHaveText('', { useInnerText: true });
  await input.fill('Before @O\nAfter');
  await input.evaluate((node) => {
    const text = document.createTreeWalker(node, NodeFilter.SHOW_TEXT).nextNode()!;
    document.getSelection()!.setBaseAndExtent(text, 9, text, 9);
  });
  await input.press('ArrowDown');
  await input.press('End');
  await input.press('Enter');
  const proof = await page.evaluate(async () => {
    const modulePath = '/test/annotation-browser.tsx';
    return (await import(modulePath)).proof();
  });
  expect(proof.writes).toBe(0);
  expect(proof.preparations).toBe(0);
  expect(proof.sends).toHaveLength(0);
  await expect(input).toHaveText('Before @Other agent \nAfter', { useInnerText: true });
});

// Chinese and Japanese text has no spaces, and a CJK IME may type the full-width ＠: the
// recipient list must open in both cases, and choosing from it binds a recipient as usual.
for (const [name, type] of [
  [
    'straight after CJK text',
    async (page: Page) => {
      await page.keyboard.insertText('請問');
      await page.keyboard.type('@');
    },
  ],
  [
    'as the full-width ＠',
    async (page: Page) => {
      await page.keyboard.insertText('＠');
    },
  ],
  [
    'after an IME-committed CJK word',
    async (page: Page) => {
      const client = await page.context().newCDPSession(page);
      await client.send('Input.imeSetComposition', {
        text: 'ㄋ',
        selectionStart: 1,
        selectionEnd: 1,
      });
      await client.send('Input.insertText', { text: '你' });
      await client.send('Input.imeSetComposition', {
        text: '@',
        selectionStart: 1,
        selectionEnd: 1,
      });
      await client.send('Input.insertText', { text: '@' });
    },
  ],
] as const)
  test(`the recipient list opens for @ ${name} and picking binds the recipient`, async ({
    page,
  }) => {
    await page.goto(fixture);
    const input = page.getByRole('combobox', { name: 'Message', exact: true });
    await expect(input).toHaveText('', { useInnerText: true });
    await input.focus();
    await type(page);
    const list = page.getByRole('listbox');
    await expect(list).toBeVisible();
    await page.getByRole('option').filter({ hasText: '@Other agent' }).click();
    await expect(list).toBeHidden();
    // The token is the agent's name; the typed trigger and prefix text are not lost.
    await expect(input).toContainText('@Other agent', { useInnerText: true });
  });

test('shared field retains its root, label association and undo history across access changes', async ({
  page,
}) => {
  await page.goto(`${fixture}?field`);
  await page.evaluate(async () => {
    const path = '/test/message-composer-browser.tsx';
    (await import(path)).mountField();
  });
  const input = page.getByRole('combobox', { name: 'Retained message', exact: true });
  await expect(input).toHaveText('', { useInnerText: true });
  await input.pressSequentially('Draft survives');
  await input.evaluate((node) => Object.assign(window, { retainedField: node }));
  const id = await input.getAttribute('id');
  expect(id).toBeTruthy();
  await expect(page.locator(`[id="${id}-label"]`)).toHaveText('Retained message');
  await expect(input).toHaveAttribute('aria-labelledby', `${id}-label`);
  await page.getByRole('button', { name: 'Toggle access' }).click();
  await expect(input).toHaveAttribute('contenteditable', 'false');
  await expect(input).toHaveAttribute('aria-disabled', 'true');
  await expect(input).toHaveText('Draft survives', { useInnerText: true });
  await page.getByRole('button', { name: 'Toggle access' }).click();
  await expect(input).toHaveAttribute('aria-disabled', 'false');
  expect(
    await input.evaluate(
      (node) => (window as unknown as { retainedField: Element }).retainedField === node,
    ),
  ).toBe(true);
  await expect(input).toHaveAttribute('id', id!);
  await input.press('ControlOrMeta+Z');
  await expect(input).toHaveText('', { useInnerText: true });
});

for (const width of [1440, 390]) {
  for (const theme of ['light', 'dark'] as const) {
    test(`interaction presentation evidence: ${width}px ${theme}`, async ({ page }) => {
      const directory =
        process.env.COLAB_INTERACTION_CAPTURE_DIR ?? '/tmp/colab-interaction-captures';
      mkdirSync(directory, { recursive: true });
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      await page.goto('/');
      const run = (method: string) =>
        page.evaluate(async (method) => {
          const path = '/test/ask-page-browser.tsx';
          return (await import(path))[method]();
        }, method);
      await run('mount');
      await expect(page.locator('#ask-page-fixture .status')).toContainText('Live preview');
      await page
        .frameLocator('#ask-page-fixture iframe')
        .locator('#selected')
        .evaluate((node) => {
          const range = node.ownerDocument.createRange();
          range.selectNodeContents(node);
          const selection = node.ownerDocument.getSelection()!;
          selection.removeAllRanges();
          selection.addRange(range);
        });
      await page.getByTestId('selection-ask').click();
      const dialog = page.getByRole('dialog', { name: 'Annotate selection' });
      const input = dialog.getByRole('combobox', { name: 'Message', exact: true });
      const capture = async (name: string) => {
        const fields = await page
          .locator('.annotation-compose .message-composer-field:visible')
          .evaluateAll((nodes) =>
            nodes.map((control) => {
              const composer = control.closest('.annotation-compose')!;
              const label = composer.querySelector('.tmt-ui-field-label')!;
              const send = composer.querySelector('.annotation-status-row > .tmt-ui-action')!;
              const fieldBox = control.getBoundingClientRect(),
                sendBox = send.getBoundingClientRect();
              return {
                labelWidth: label.getBoundingClientRect().width,
                clearance: sendBox.top - fieldBox.bottom,
                sendRight: sendBox.right,
                fieldRight: fieldBox.right,
                padding: parseFloat(getComputedStyle(send).paddingLeft),
              };
            }),
          );
        expect(fields.length).toBeGreaterThan(0);
        for (const field of fields) {
          expect(field.labelWidth).toBe(1);
          expect(field.clearance).toBeGreaterThanOrEqual(8);
          expect(Math.abs(field.sendRight - field.fieldRight)).toBeLessThan(1);
          expect(field.padding).toBeGreaterThan(0);
        }
        const state = await page.evaluate(() =>
          [...document.querySelectorAll<HTMLButtonElement>('.tmt-ui-action')].map((button) => {
            const style = getComputedStyle(button);
            return {
              label: button.getAttribute('aria-label') ?? button.textContent,
              hovered: button.matches(':hover'),
              focused: button.matches(':focus'),
              focusVisible: button.matches(':focus-visible'),
              disabled: button.disabled,
              variant: button.dataset.variant,
              background: style.backgroundColor,
              color: style.color,
              disabledColor: (() => {
                const probe = document.createElement('span');
                probe.style.color = style.getPropertyValue('--tmt-ui-color-disabled-text');
                return probe.style.color;
              })(),
            };
          }),
        );
        for (const button of state) {
          if (
            button.variant === 'text' &&
            ((!button.hovered && !button.focusVisible) || button.disabled)
          )
            expect(button.background).toBe('rgba(0, 0, 0, 0)');
          if (button.disabled && button.variant === 'text')
            expect(button.color).toBe(button.disabledColor);
        }
        writeFileSync(
          `${directory}/${width}-${theme}-${name}.json`,
          `${JSON.stringify(state, null, 2)}\n`,
        );
        await page.screenshot({ path: `${directory}/${width}-${theme}-${name}.png` });
      };
      await expect(input).toBeFocused();
      await capture('annotation-empty');
      await input.pressSequentially('A plain comment about the selected text.');
      await capture('annotation-comment');
      await input.press('Enter');
      await expect(dialog.getByTestId('comment-entry')).toHaveCount(1);
      await input.pressSequentially('A reply to the original comment.');
      await capture('reply-comment');
      await input.press('End');
      await input.pressSequentially(' @');
      await page.getByRole('option').filter({ hasText: '@Agent 1 ·' }).click();
      await capture('reply-agent');
      await run('pausePrepare');
      await dialog.getByRole('button', { name: 'Send', exact: true }).click();
      await expect(input).toHaveAttribute('contenteditable', 'false');
      await capture('reply-busy');
      await run('resumePrepare');
      await expect(input).toHaveAttribute('contenteditable', 'true');
      await dialog.getByRole('button', { name: 'Close thread', exact: true }).click();
      const toggle = page.locator('#ask-page-fixture').getByTestId('chat-toggle');
      if (!(await toggle.isVisible()))
        await page.getByRole('button', { name: 'More page actions' }).click();
      await toggle.click();
      const chat = page.getByTestId('chat-panel');
      const chatInput = chat.getByRole('combobox', { name: 'Message', exact: true });
      await chatInput.pressSequentially('@');
      await page.getByRole('option').filter({ hasText: '@Agent 1 ·' }).click();
      await chatInput.pressSequentially('Ask about the complete page.');
      await capture('chat-ready');
      await run('pausePrepare');
      await chat.getByRole('button', { name: 'Send', exact: true }).click();
      await expect(chatInput).toHaveAttribute('contenteditable', 'false');
      await capture('chat-busy');
      await run('resumePrepare');
      await expect(chatInput).toHaveAttribute('contenteditable', 'true');
      const close = page.getByRole('button', { name: 'Close Chat', exact: true });
      await close.focus();
      await close.press('Shift+Tab');
      await page.keyboard.press('Tab');
      await expect(close).toBeFocused();
      if (!process.env.COLAB_INTERACTION_BASELINE)
        await expect(page.locator('.tmt-ui-icon-action-tooltip:popover-open')).toHaveText(
          'Close Chat',
        );
      await capture('drawer-focus');
      if (!process.env.COLAB_INTERACTION_BASELINE) {
        await page.keyboard.press('Escape');
        await expect(page.locator('.tmt-ui-icon-action-tooltip:popover-open')).toHaveCount(0);
        await expect(chat).toBeVisible();
      }
      await run('block');
      await expect(chatInput).toHaveAttribute('contenteditable', 'false');
      await capture('chat-disabled');
      if (!process.env.COLAB_INTERACTION_BASELINE) {
        await page.keyboard.press('Escape');
        await expect(chat).not.toBeVisible();
      }
    });
  }
}
