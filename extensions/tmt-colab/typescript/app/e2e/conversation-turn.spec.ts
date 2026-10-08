import { mkdirSync, writeFileSync } from 'node:fs';
import { expect, test, type Locator, type Page } from '@playwright/test';
import { captureDirectory } from './captures.js';
import { text } from '../src/strings.js';
const fixture = '/test/ask-page-browser.tsx';
const captureDir = process.env.COLAB_TURN_CAPTURE_DIR ?? captureDirectory();
const request = 'Explain <img src=x onerror=alert(1)> in this selection.\nKeep the exact text.';
const reply = 'Keep <script>reply</script> as text.\nThis is Atlas’s answer.';
async function run(page: Page, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
async function mount(page: Page, surface: 'thread' | 'chat', state = 'waiting') {
  await page.goto('/');
  await run(page, 'mount');
  await run(page, 'conversation', { surface, state });
  const host = page.locator('#ask-page-fixture');
  await expect(host.locator('.status')).toContainText('Live preview');
  if (surface === 'thread') {
    await page.frameLocator('#ask-page-fixture iframe').locator('[data-colab-thread]').click();
    await expect(page.getByTestId('comment-thread')).toHaveAttribute('data-layout', 'anchored');
  } else {
    const toggle = host.getByTestId('chat-toggle');
    if (!(await toggle.isVisible()))
      await host.getByRole('button', { name: 'More page actions' }).click();
    await toggle.click();
  }
}
async function annotate(page: Page) {
  await page.goto('/');
  await run(page, 'mount');
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
  await expect(page.getByTestId('annotation-window')).toBeVisible();
}
async function capture(page: Page, window: Locator, name: string) {
  await page.screenshot({ path: `${captureDir}/${name}.png` });
  writeFileSync(`${captureDir}/${name}.json`, JSON.stringify(await window.boundingBox()));
}
async function styles(message: Locator) {
  return message.evaluate((node) => {
    const properties = [
      'display',
      'box-sizing',
      'margin',
      'padding',
      'border-top',
      'border-right',
      'border-bottom',
      'border-left',
      'border-radius',
      'background-color',
      'box-shadow',
      'color',
      'font-family',
      'font-size',
      'font-weight',
      'line-height',
      'text-align',
      'max-width',
      'align-self',
      'white-space',
      'overflow-wrap',
    ];
    const read = (element: Element) => {
      const style = getComputedStyle(element);
      return Object.fromEntries(properties.map((key) => [key, style.getPropertyValue(key)]));
    };
    return {
      row: read(node),
      meta: read(node.querySelector('.comment-byline')!),
      body: read(node.querySelector('.conversation-body')!),
    };
  });
}
async function flat(message: Locator) {
  await expect(message).toHaveCSS('border-radius', '0px');
  await expect(message).toHaveCSS('box-shadow', 'none');
  await expect(message).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
  await expect(message).toHaveCSS('margin', '0px');
  await expect(message).toHaveCSS('padding-top', '10px');
  await expect(message).toHaveCSS('padding-bottom', '10px');
  await expect(message).toHaveCSS('padding-right', '14px');
  await expect(message).toHaveCSS('border-bottom-width', '1px');
  expect(await message.getAttribute('data-turn-layout')).toBeNull();
  const bounds = await message.evaluate((node) => {
    const history = node.closest('.conversation-messages')!;
    const a = node.getBoundingClientRect(),
      b = history.getBoundingClientRect();
    return { left: a.left - b.left, width: a.width - history.clientWidth };
  });
  expect(Math.abs(bounds.left)).toBeLessThan(1);
  expect(Math.abs(bounds.width)).toBeLessThan(1);
}
async function shell(window: Locator) {
  await expect(window.locator(':scope > .conversation-window-bar')).toHaveCount(1);
  await expect(window.locator(':scope > .conversation-composer')).toHaveCount(1);
  await expect(window.locator('.conversation-composer .annotation-compose')).toHaveCount(1);
  await expect(window.locator('.conversation-composer')).toHaveCSS('padding', '10px 14px');
  for (const button of await window.locator('.conversation-composer button').all())
    await expect(button).toBeInViewport();
  const bounds = await window.evaluate((node) => {
    const header = node.querySelector('.conversation-window-bar')!.getBoundingClientRect();
    const actions = node.querySelector('.thread-bar-actions')!.getBoundingClientRect();
    const history = node.querySelector('.conversation-messages')!.getBoundingClientRect();
    const composer = node.querySelector('.conversation-composer')!.getBoundingClientRect();
    return { header, actions, history, composer };
  });
  expect(bounds.actions.top).toBeGreaterThanOrEqual(bounds.header.top);
  expect(bounds.actions.bottom).toBeLessThanOrEqual(bounds.header.bottom);
  expect(bounds.history.bottom).toBeLessThanOrEqual(bounds.composer.top + 1);
}
for (const width of [1440, 390]) {
  for (const theme of ['light', 'dark'] as const) {
    test(`one flat style for Chat, anchored thread and annotate at ${width} ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      mkdirSync(captureDir, { recursive: true });
      const computed: Record<string, Awaited<ReturnType<typeof styles>>> = {};
      for (const surface of ['chat', 'thread'] as const) {
        await mount(page, surface);
        const window = page.getByTestId(surface === 'chat' ? 'chat-panel' : 'comment-thread');
        await shell(window);
        if (surface === 'chat') {
          await expect(page.locator('.chat-visibility')).toHaveCount(0);
          await expect(page.locator('.page-drawer[data-panel="chat"] > .drawer-bar')).toHaveCount(
            0,
          );
          await expect(window.locator('.conversation-caption')).toHaveText(text.chatVisible);
          const caption = (await window.locator('.conversation-caption').boundingBox())!;
          const title = (await window.locator('.conversation-title').boundingBox())!;
          expect(caption.x).toBeGreaterThan(title.x + title.width);
          expect(Math.abs(caption.y - title.y)).toBeLessThan(5);
        }
        const user = window.locator('.conversation-turn[data-turn-role="user"]');
        const agent = window.locator('.conversation-turn[data-turn-role="agent"]');
        await expect(user).toHaveCount(1);
        await expect(user.locator('.comment-byline')).toContainText('You ·');
        await expect(user.locator('.conversation-body')).toHaveText(request);
        await flat(user);
        await expect(user).toHaveCSS('padding-left', '14px');
        computed[`${surface}-user`] = await styles(user);
        for (const [state, label] of [
          ['waiting', 'Waiting for Atlas'],
          ['held', 'Waiting for approval'],
          ['replied', ''],
          ['failed', 'Not delivered'],
          ['uncertain', 'Delivery unconfirmed'],
        ]) {
          await run(page, 'conversation', { surface, state });
          if (state === 'replied') {
            await expect(agent).toHaveCount(1);
            await flat(agent);
            await expect(agent).toHaveCSS('border-left-width', '3px');
            await expect(agent).toHaveCSS('padding-left', '11px');
            await expect(agent.getByTestId('ask-reply')).toHaveText(reply);
            await expect(agent.getByTestId('ask-reply-attribution')).toHaveText(
              `Atlas · ${text.conversationAgent} · 5m ago`,
            );
            computed[`${surface}-agent`] = await styles(agent);
            await expect(window.getByTestId('ask-state')).toHaveCount(0);
            await expect(window.getByRole('button', { name: text.askRecheck })).toHaveCount(0);
          } else {
            await expect(agent).toHaveCount(0);
            const status = user.getByTestId('ask-state');
            await expect(status).toHaveText(label);
            await expect(status.locator('svg.lucide')).toBeVisible();
            await expect(user.locator('header').getByTestId('ask-state')).toHaveCount(1);
            if (state === 'waiting' || state === 'uncertain') {
              const group = user.locator('.conversation-status');
              const check = group.getByRole('button', { name: text.askRecheck });
              await expect(check).toBeEnabled();
              const s = (await status.boundingBox())!,
                c = (await check.boundingBox())!;
              expect(Math.abs(s.y - c.y)).toBeLessThan(3);
              const u = (await user.boundingBox())!;
              expect(c.x + c.width).toBeLessThanOrEqual(u.x + u.width);
            }
            if (state === 'uncertain')
              await expect(user.locator('.ask-supporting')).toHaveText(text.askUncertain);
          }
          await expect(window.locator('script,img,.conversation-avatar')).toHaveCount(0);
          expect(await run(page, 'proof')).toEqual({ sends: [], actions: [] });
          await capture(page, window, `${surface}-${width}-${theme}-${state}`);
        }
      }
      expect(computed['chat-user']).toEqual(computed['thread-user']);
      expect(computed['chat-agent']).toEqual(computed['thread-agent']);
      await annotate(page);
      await shell(page.getByTestId('annotation-window'));
      await capture(page, page.getByTestId('annotation-window'), `annotate-${width}-${theme}`);
    });
  }
}

test('status copy preserves every ledger outcome and explicit trusted tracking actions', async ({
  page,
}) => {
  await mount(page, 'thread', 'uncertain');
  const window = page.getByTestId('comment-thread');
  const check = window.getByRole('button', { name: text.askRecheck });
  await check.evaluate((node) => node.dispatchEvent(new MouseEvent('click', { bubbles: true })));
  expect((await run(page, 'proof')).actions).toEqual([]);
  await check.click();
  await window.getByRole('button', { name: text.askAbandon }).click();
  expect((await run(page, 'proof')).actions).toEqual([
    'recheck:00000000-0000-4000-8000-000000000043',
    'abandon:00000000-0000-4000-8000-000000000043',
  ]);
  for (const [state, label, supporting] of [
    ['dispatching', 'Waiting for Atlas', ''],
    ['accepted', 'Waiting for Atlas', ''],
    ['held', 'Waiting for approval', text.askHeld],
    ['timeout', 'No reply yet from Atlas', ''],
    ['failed', 'Not delivered', text.askOperationFailed],
    ['refused', 'Not delivered', text.askRefused],
    ['cancelled', 'Not delivered', text.askCancelled],
    ['expired', 'Not delivered', text.askExpiredState],
    ['uncertain', 'Delivery unconfirmed', text.askUncertain],
    ['abandoned', 'Tracking abandoned', text.askAbandoned],
    [
      'unavailable',
      'Result unavailable',
      `${text.askDeliveryAccepted} ${text.askFinalUnavailable}`,
    ],
  ]) {
    await run(page, 'conversation', { surface: 'thread', state });
    await expect(window.getByTestId('ask-state')).toHaveText(label);
    await expect(window.getByTestId('ask-state').locator('svg.lucide')).toBeVisible();
    if (supporting) await expect(window.locator('.ask-supporting')).toHaveText(supporting);
    else await expect(window.locator('.ask-supporting')).toHaveCount(0);
    await expect(window.getByRole('button', { name: text.askRecheck })).toHaveCount(
      ['dispatching', 'accepted', 'held', 'timeout', 'uncertain'].includes(state) ? 1 : 0,
    );
  }
  await run(page, 'conversation', { surface: 'thread', state: 'empty' });
  await expect(window.getByTestId('ask-reply')).toHaveAttribute('data-empty', 'true');
  await expect(window.getByText(text.askEmptyReply)).toBeVisible();
  await expect(window.getByTestId('ask-state')).toHaveCount(0);
  await expect(window.getByRole('button', { name: text.askRecheck })).toHaveCount(0);
  await window.getByRole('button', { name: text.threadResolve, exact: true }).click();
  await expect(page.getByTestId('comment-thread')).toHaveCount(0);
  expect((await run(page, 'proof')).sends).toEqual([]);
});

test('mention tokens use the same neutral styling before and after explicit send; annotation keeps its composer instance', async ({
  page,
}) => {
  await annotate(page);
  const window = page.getByRole('dialog', { name: 'Annotate selection' });
  const input = window.getByRole('combobox', { name: 'Message', exact: true });
  const id = await input.getAttribute('id');
  await input.fill('@');
  await page.getByRole('option').first().click();
  await input.press('End');
  await page.keyboard.type('Explain this. @someone');
  const token = input.locator('.message-mention');
  await expect(token).toHaveText('@Agent 1');
  const before = await token.evaluate((node) => {
    const style = getComputedStyle(node);
    return [style.backgroundColor, style.padding, style.border, style.borderRadius];
  });
  await input.press('Enter');
  // The double publishes the admitted Ask attribution separately, like the signed own stream.
  await run(page, 'windowReply');
  const thread = page.getByTestId('comment-thread');
  await expect(thread).toHaveAttribute('data-layout', 'anchored');
  await expect(input).toHaveAttribute('id', id!);
  await expect(thread.locator('.comment-body')).toHaveText('@Agent 1 Explain this. @someone');
  const sent = thread.locator('.comment-body .message-mention');
  await expect(sent).toHaveCount(1);
  await expect(sent).toHaveText('@Agent 1');
  expect(
    await sent.evaluate((node) => {
      const style = getComputedStyle(node);
      return [style.backgroundColor, style.padding, style.border, style.borderRadius];
    }),
  ).toEqual(before);
  expect((await run(page, 'proof')).sends).toHaveLength(1);
});

test('a long agent label wraps the entire status group below the byline without clipping or truncating', async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 900 });
  await mount(page, 'chat');
  const name = 'LongAgentLabel'.repeat(8);
  await run(page, 'conversation', { surface: 'chat', state: 'waiting', agentName: name });
  const user = page.locator('.conversation-turn[data-turn-role="user"]');
  await expect(user.getByTestId('ask-state')).toHaveText(`Waiting for ${name}`);
  const geometry = await user.locator('.conversation-status').evaluate((node) => {
    const style = getComputedStyle(node);
    const byline = node
      .closest('header')!
      .querySelector('.comment-byline')!
      .getBoundingClientRect();
    return {
      width: node.clientWidth,
      scroll: node.scrollWidth,
      top: node.getBoundingClientRect().top,
      bylineBottom: byline.bottom,
      overflow: style.textOverflow,
    };
  });
  expect(geometry.scroll).toBeLessThanOrEqual(geometry.width + 1);
  expect(geometry.top).toBeGreaterThanOrEqual(geometry.bylineBottom);
  expect(geometry.overflow).not.toBe('ellipsis');
  await expect(user.getByRole('button', { name: text.askRecheck })).toBeInViewport();
  expect((await run(page, 'proof')).sends).toEqual([]);
});
