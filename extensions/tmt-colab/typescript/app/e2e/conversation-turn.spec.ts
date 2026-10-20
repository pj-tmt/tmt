import { pageAction } from '../test/page-actions.js';
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
/** Infinite animations running in a thread: the marks that turn while an agent works (#3978). */
const turning = (thread: Locator) =>
  thread.evaluate(
    (node) =>
      node
        .getAnimations({ subtree: true })
        .filter((animation) => animation.effect?.getComputedTiming().iterations === Infinity)
        .length,
  );
async function mount(page: Page, surface: 'thread' | 'chat', state = 'waiting') {
  await page.goto('/');
  await run(page, 'mount');
  await run(page, 'conversation', { surface, state });
  const host = page.locator('#ask-page-fixture');
  await expect(host.locator('.status')).toContainText('Live');
  if (surface === 'thread') {
    await page.frameLocator('#ask-page-fixture iframe').locator('[data-colab-thread]').click();
    await expect(page.getByTestId('comment-thread')).toHaveAttribute('data-layout', 'anchored');
  } else {
    const toggle = await pageAction(host, 'Chat');
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
  const chat = await message.evaluate((node) => !!node.closest('.chat-panel'));
  const agent = (await message.getAttribute('data-turn-role')) === 'agent';
  await expect(message).toHaveCSS('border-radius', '0px');
  await expect(message).toHaveCSS('box-shadow', 'none');
  await expect(message).toHaveCSS('border-bottom-width', '0px');
  if (chat || !agent) await expect(message).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
  else await expect(message).not.toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
  await expect(message).toHaveCSS('padding-top', chat || agent ? '8px' : '0px');
  expect(await message.getAttribute('data-turn-layout')).toBeNull();
  const bounds = await message.evaluate((node) => {
    const history = node.closest('.conversation-messages')!;
    const a = node.getBoundingClientRect(),
      b = history.getBoundingClientRect();
    return { left: a.left - b.left, right: a.right - b.right };
  });
  expect(bounds.left).toBeGreaterThanOrEqual(0);
  expect(bounds.right).toBeLessThanOrEqual(1);
}
async function shell(window: Locator) {
  const chat = (await window.getAttribute('data-testid')) === 'chat-panel';
  await expect(
    chat
      ? window.page().locator('.tmt-ui-drawer[data-panel=chat][open] > .tmt-ui-drawer-header')
      : window.locator(':scope > .conversation-window-bar'),
  ).toHaveCount(1);
  if (chat) await expect(window.locator(':scope > .conversation-window-bar')).toHaveCount(0);
  await expect(window.locator(':scope > .conversation-composer')).toHaveCount(1);
  await expect(window.locator('.conversation-composer .annotation-compose')).toHaveCount(1);
  await expect(window.locator('.conversation-composer')).toHaveCSS('padding', '12px 16px 16px');
  await expect(window.locator('.conversation-composer')).toHaveCSS('border-top-width', '1px');
  for (const button of await window.locator('.conversation-composer button').all())
    await expect(button).toBeInViewport();
  const bounds = await window.evaluate((node) => {
    const drawerHeader = node.closest('.tmt-ui-drawer')?.querySelector('.tmt-ui-drawer-header');
    const header = (drawerHeader ??
      node.querySelector('.conversation-window-bar'))!.getBoundingClientRect();
    const actions = (drawerHeader?.querySelector('button') ??
      node.querySelector('.thread-bar-actions'))!.getBoundingClientRect();
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
          await expect(
            page.locator('.tmt-ui-drawer[data-panel="chat"][open] > .tmt-ui-drawer-header'),
          ).toHaveCount(1);
          const header = page.locator(
            '.tmt-ui-drawer[data-panel=chat][open] > .tmt-ui-drawer-header',
          );
          await expect(header.locator('.tmt-ui-drawer-caption')).toHaveText(text.chatVisible);
          await expect(header.getByRole('heading', { name: 'Chat', exact: true })).toBeVisible();
          await expect(
            header.getByRole('button', { name: 'Close Chat', exact: true }),
          ).toBeVisible();
        }
        const user = window.locator('.conversation-turn[data-turn-role="user"]');
        const agent = window.locator('.conversation-turn[data-turn-role="agent"]');
        await expect(user).toHaveCount(1);
        await expect(user.locator('.conversation-author')).toHaveText('You');
        await expect(user.locator('.conversation-body')).toHaveText(request);
        await flat(user);
        await expect(user).toHaveCSS('padding-left', surface === 'chat' ? '16px' : '0px');
        computed[`${surface}-user`] = await styles(user);
        for (const [state, label] of [
          ['waiting', 'Sent'],
          ['held', 'Waiting for approval'],
          ['replied', ''],
          ['failed', 'Not delivered'],
          ['uncertain', 'Delivery unconfirmed'],
        ]) {
          await run(page, 'conversation', { surface, state });
          if (state === 'replied') {
            await expect(agent).toHaveCount(1);
            await flat(agent);
            await expect(agent).toHaveCSS('border-left-width', surface === 'chat' ? '3px' : '0px');
            await expect(agent).toHaveCSS('padding-left', surface === 'chat' ? '13px' : '8px');
            await expect(agent.getByTestId('ask-reply')).toHaveText(reply);
            await expect(agent.getByTestId('ask-reply-attribution')).toHaveText('Atlas · 5m ago');
            computed[`${surface}-agent`] = await styles(agent);
            await expect(window.getByTestId('ask-state')).toHaveCount(0);
            await expect(user.locator('.conversation-turn-status')).toHaveCount(0);
            await expect(window.getByRole('button', { name: text.askRecheck })).toHaveCount(0);
          } else {
            await expect(agent).toHaveCount(0);
            const status = user.getByTestId('ask-state');
            await expect(status).toHaveText(
              surface === 'chat' && state === 'failed' ? `✗${label}` : label,
            );
            if (surface === 'chat' && state === 'failed')
              await expect(status.locator('.ask-state-mark')).toHaveText('✗');
            else if (state === 'waiting')
              await expect(status.locator('.ask-state-mark svg.lucide')).toBeHidden();
            else await expect(status.locator('.ask-state-mark svg.lucide')).toBeVisible();
            await expect(user.locator('header').getByTestId('ask-state')).toHaveCount(0);
            const placement = await user.evaluate((node) => {
              const main = node.querySelector('.conversation-main')!;
              const byline = main.querySelector('header')!;
              const body = main.querySelector('.conversation-body')!;
              const status = main.querySelector('.conversation-turn-status')!;
              const children = Array.from(main.children);
              return {
                order: [byline, body, status].map((child) => children.indexOf(child)),
                body: body.getBoundingClientRect().toJSON(),
                status: status.getBoundingClientRect().toJSON(),
              };
            });
            expect(placement.order[0]).toBeLessThan(placement.order[1]);
            expect(placement.order[1]).toBeLessThan(placement.order[2]);
            expect(placement.status.top).toBeGreaterThanOrEqual(placement.body.bottom);
            expect(placement.status.left).toBe(placement.body.left);
            if (state === 'waiting' || state === 'held' || state === 'uncertain') {
              const group = user.locator('.conversation-status');
              const check = group.getByRole('button', { name: text.askRecheck });
              await expect(check).toBeEnabled();
              const s = (await status.boundingBox())!,
                c = (await check.boundingBox())!;
              expect(Math.abs(s.y + s.height / 2 - c.y - c.height / 2)).toBeLessThan(1);
              const u = (await user.boundingBox())!;
              expect(c.x + c.width).toBeLessThanOrEqual(u.x + u.width);
              await expect(check.locator('svg.lucide-rotate-cw')).toBeVisible();
              await expect(check).toHaveText('');
              if (width === 390) {
                expect(c.width).toBeGreaterThanOrEqual(44);
                expect(c.height).toBeGreaterThanOrEqual(44);
              }
            }
            if (state === 'uncertain')
              await expect(user.locator('.ask-supporting')).toHaveText(text.askUncertain);
          }
          await expect(window.locator('script,img')).toHaveCount(0);
          // The person's avatar is the only one left; an agent turn shows its chip.
          await expect(window.locator('.conversation-avatar')).toHaveCount(
            surface === 'chat' ? 0 : 1,
          );
          await expect(
            window.locator('[data-turn-role="agent"] .conversation-author .agent-chip'),
          ).toHaveCount(state === 'replied' ? 1 : 0);
          expect(await run(page, 'proof')).toEqual({ sends: [], actions: [] });
          await capture(page, window, `${surface}-${width}-${theme}-${state}`);
        }
      }
      expect(computed['chat-user'].row['border-bottom']).toContain('0px');
      expect(computed['thread-user'].row['border-bottom']).toContain('0px');
      expect(computed['chat-agent'].row['border-left']).toContain('3px');
      expect(computed['thread-agent'].row['border-left']).toContain('0px');
      await annotate(page);
      await shell(page.getByTestId('annotation-window'));
      await capture(page, page.getByTestId('annotation-window'), `annotate-${width}-${theme}`);
    });
  }
}

async function chatGeometry(window: Locator) {
  return window.evaluate((node) => {
    const rect = (element: Element) => element.getBoundingClientRect().toJSON();
    const find = (selector: string) => node.querySelector<HTMLElement>(selector)!;
    const drawer = node.closest('dialog')!;
    const composer = find('.conversation-composer');
    const history = find('.conversation-messages');
    return {
      drawer: rect(drawer),
      header: rect(drawer.querySelector('.tmt-ui-drawer-header')!),
      composer: rect(composer),
      field: rect(find('[role="combobox"]')),
      status: rect(find('.annotation-status-row')),
      send: rect(find('.annotation-status-row [data-variant="primary"]')),
      history: rect(history),
      historyTop: history.scrollTop,
      historyOverflow: history.scrollHeight - history.clientHeight,
      outer: [drawer, node.parentElement!, node, composer].map((element) => ({
        top: element.scrollTop,
        overflow: element.scrollHeight - element.clientHeight,
      })),
      pageScroll: scrollY,
      viewport: { width: innerWidth, height: innerHeight },
    };
  });
}
function visibleChatComposer(geometry: Awaited<ReturnType<typeof chatGeometry>>) {
  for (const bounds of [
    geometry.header,
    geometry.composer,
    geometry.field,
    geometry.status,
    geometry.send,
  ]) {
    expect(bounds.top).toBeGreaterThanOrEqual(geometry.drawer.top);
    expect(bounds.bottom).toBeLessThanOrEqual(geometry.drawer.bottom);
    expect(bounds.left).toBeGreaterThanOrEqual(geometry.drawer.left);
    expect(bounds.right).toBeLessThanOrEqual(geometry.drawer.right);
    expect(bounds.top).toBeGreaterThanOrEqual(0);
    expect(bounds.bottom).toBeLessThanOrEqual(geometry.viewport.height);
    expect(bounds.left).toBeGreaterThanOrEqual(0);
    expect(bounds.right).toBeLessThanOrEqual(geometry.viewport.width);
  }
  for (const bounds of [geometry.field, geometry.status, geometry.send]) {
    expect(bounds.top).toBeGreaterThanOrEqual(geometry.composer.top);
    expect(bounds.bottom).toBeLessThanOrEqual(geometry.composer.bottom);
  }
  expect(geometry.history.top).toBeGreaterThanOrEqual(geometry.header.bottom - 1);
  expect(geometry.history.bottom).toBeLessThanOrEqual(geometry.composer.top + 1);
  for (const element of geometry.outer) {
    expect(element.overflow).toBeLessThanOrEqual(1);
    expect(element.top).toBe(0);
  }
}
for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`long Chat keeps its whole composer visible at ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      await mount(page, 'chat');
      const window = page.getByTestId('chat-panel');
      const short = await chatGeometry(window);
      await run(page, 'scrollHistory');
      await expect(window.getByTestId('comment-entry')).toHaveCount(40);
      const history = window.locator('.conversation-messages');
      await expect
        .poll(() =>
          history.evaluate((node) => node.scrollHeight - node.clientHeight - node.scrollTop),
        )
        .toBeLessThanOrEqual(1);
      const long = await chatGeometry(window);
      visibleChatComposer(long);
      expect(long.composer.height).toBeCloseTo(short.composer.height, 1);
      expect(long.historyOverflow).toBeGreaterThan(0);
      mkdirSync(captureDir, { recursive: true });
      await capture(page, window, `chat-composer-${width}-${theme}`);
      await history.evaluate((node) => {
        node.scrollTop = 150;
      });
      await expect.poll(() => history.evaluate((node) => node.scrollTop)).toBe(150);
      const scrolled = await chatGeometry(window);
      visibleChatComposer(scrolled);
      for (const key of ['header', 'composer', 'field', 'status', 'send'] as const)
        expect(scrolled[key]).toEqual(long[key]);
      expect(scrolled.outer).toEqual(long.outer);
      expect(scrolled.pageScroll).toBe(long.pageScroll);
      await page.setViewportSize({ width, height: 500 });
      const resized = await chatGeometry(window);
      visibleChatComposer(resized);
      expect(resized.composer.height).toBeCloseTo(long.composer.height, 1);
      expect(resized.history.height).toBeLessThan(long.history.height);
      expect(resized.historyOverflow).toBeGreaterThan(0);
      if (width === 390) {
        // Include the visible page header, gutters and 1 px drawer border before measuring stationary controls.
        // With room for only those controls, history yields all its height.
        await page.setViewportSize({
          width,
          height: Math.ceil(
            resized.drawer.top +
              (resized.viewport.height - resized.drawer.bottom) +
              resized.header.height +
              resized.composer.height +
              2,
          ),
        });
        const minimum = await chatGeometry(window);
        visibleChatComposer(minimum);
        expect(minimum.composer.height).toBeCloseTo(long.composer.height, 1);
        expect(minimum.history.height).toBeLessThanOrEqual(1);
      }
    });

for (const surface of ['chat', 'thread'] as const)
  test(`${surface}: Check again has hover/focus detail and trusted keyboard activation below the message`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: 390, height: 900 });
    await mount(page, surface, 'waiting');
    const window = page.getByTestId(surface === 'chat' ? 'chat-panel' : 'comment-thread');
    const check = window.getByRole('button', { name: text.askRecheck, exact: true });
    const tooltip = window.locator('.conversation-status .tmt-ui-icon-action-tooltip');
    await check.hover();
    await expect(tooltip).toBeVisible();
    await expect(tooltip).toHaveText(text.askRecheck);
    await page.mouse.move(389, 899);
    await expect(tooltip).toBeHidden();
    await check.focus();
    await check.press('Tab');
    await page.keyboard.press('Shift+Tab');
    await expect(check).toBeFocused();
    await expect(tooltip).toBeVisible();
    await expect(tooltip).toHaveText(text.askRecheck);
    await check.press('Enter');
    expect((await run(page, 'proof')).actions).toEqual([
      'recheck:00000000-0000-4000-8000-000000000043',
    ]);
    expect((await run(page, 'proof')).sends).toEqual([]);
  });

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
    ['dispatching', 'sending…', ''],
    ['accepted', 'Sent', ''],
    ['held', 'Waiting for approval', text.askHeld],
    ['timeout', 'Sent', ''],
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
    if (['dispatching', 'accepted', 'timeout'].includes(state))
      await expect(
        window.getByTestId('ask-state').locator('.ask-state-mark svg.lucide'),
      ).toBeHidden();
    else
      await expect(
        window.getByTestId('ask-state').locator('.ask-state-mark svg.lucide'),
      ).toBeVisible();
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
  await window.getByRole('button', { name: text.threadResolveAction, exact: true }).click();
  await expect(page.getByTestId('comment-thread')).toHaveCount(0);
  expect((await run(page, 'proof')).sends).toEqual([]);
});

test('editable mention chips send only their original text; annotation keeps its composer instance', async ({
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
  const token = input.locator('.agent-chip');
  await expect(token.locator('.agent-chip-name')).toHaveText('@Agent 1');
  await expect(token.getByRole('button', { name: /^Remove mention:/ })).toBeVisible();
  await expect(token).toHaveCSS('border-top-width', '1px');
  await expect(token).toHaveCSS('border-radius', '0px');
  await input.press('Enter');
  // The double publishes the admitted Ask attribution separately, like the signed own stream.
  await run(page, 'windowReply');
  const thread = page.getByTestId('comment-thread');
  await expect(thread).toHaveAttribute('data-layout', 'anchored');
  await expect(input).toHaveAttribute('id', id!);
  await expect(thread.locator('.comment-body')).toHaveText('Agent 1 Explain this. someone');
  const sent = thread.locator('.comment-body .agent-chip');
  await expect(sent).toHaveCount(2);
  await expect(sent).toHaveText(['Agent 1', 'someone']);
  await expect(sent.getByRole('button')).toHaveCount(0);
  expect((await run(page, 'proof')).sends).toHaveLength(1);
});

test('a long agent label wraps below the message without clipping or hiding Check again', async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 900 });
  await mount(page, 'chat');
  const name = 'LongAgentLabel'.repeat(8);
  await run(page, 'conversation', { surface: 'chat', state: 'waiting', agentName: name });
  const user = page.locator('.conversation-turn[data-turn-role="user"]');
  await run(page, 'conversationActivity', { kind: 'unknown' });
  await expect(user.getByTestId('ask-state')).toHaveText(
    `✓ sent · ${name}'s activity is not reported`,
  );
  const geometry = await user.locator('.conversation-status').evaluate((node) => {
    const style = getComputedStyle(node);
    const body = node
      .closest('.conversation-main')!
      .querySelector('.conversation-body')!
      .getBoundingClientRect();
    return {
      width: node.clientWidth,
      scroll: node.scrollWidth,
      top: node.getBoundingClientRect().top,
      bodyBottom: body.bottom,
      overflow: style.textOverflow,
    };
  });
  expect(geometry.scroll).toBeLessThanOrEqual(geometry.width + 1);
  expect(geometry.top).toBeGreaterThanOrEqual(geometry.bodyBottom);
  expect(geometry.overflow).not.toBe('ellipsis');
  await expect(user.getByRole('button', { name: text.askRecheck })).toBeInViewport();
  expect((await run(page, 'proof')).sends).toEqual([]);
});

async function windowGeometry(window: Locator) {
  return window.evaluate((node) => {
    const rect = (selector: string) =>
      node.querySelector(selector)!.getBoundingClientRect().toJSON();
    const history = node.querySelector<HTMLElement>('.conversation-messages')!;
    const quote = node.querySelector('.annotation-anchor-note')!;
    return {
      window: node.getBoundingClientRect().toJSON(),
      header: rect('.conversation-window-bar'),
      quote: rect('.annotation-anchor-note'),
      composer: rect('.conversation-composer'),
      field: rect('[role="combobox"]'),
      send: rect('.annotation-status-row [data-variant="primary"]'),
      history: rect('.conversation-messages'),
      scrollHeight: history.scrollHeight,
      clientHeight: history.clientHeight,
      quoteSpacing: parseFloat(getComputedStyle(quote).marginBottom),
      pageScroll: scrollY,
    };
  });
}

async function windowStyles(window: Locator) {
  return window.evaluate((node) => {
    const properties = [
      'display',
      'align-items',
      'gap',
      'padding',
      'margin',
      'border',
      'border-radius',
      'background-color',
      'box-shadow',
      'color',
      'font',
      'min-height',
      'overflow',
    ];
    return Object.fromEntries(
      [
        '.conversation-window-bar',
        '.conversation-composer',
        '.annotation-status-row',
        '[role="combobox"]',
        '.annotation-status-row [data-variant="primary"]',
      ].map((selector) => {
        const style = getComputedStyle(node.querySelector(selector)!);
        return [
          selector,
          Object.fromEntries(properties.map((key) => [key, style.getPropertyValue(key)])),
        ];
      }),
    );
  });
}

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`annotation content fit and shared thread geometry at ${width} ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      const directory = process.env.COLAB_2134_CAPTURE_DIR;
      const record = async (window: Locator, state: string) => {
        await page.mouse.move(width - 1, 899);
        const geometry = await windowGeometry(window);
        if (directory) {
          mkdirSync(directory, { recursive: true });
          const name = `${width}-${theme}-${state}`;
          await page.screenshot({ path: `${directory}/${name}.png` });
          writeFileSync(`${directory}/${name}.json`, JSON.stringify(geometry, null, 2));
        }
        return geometry;
      };
      await annotate(page);
      const annotation = page.getByTestId('annotation-window');
      const short = await record(annotation, 'short-quote');
      const annotationStyles = await windowStyles(annotation);
      const input = page
        .getByRole('dialog', { name: 'Annotate selection' })
        .getByRole('combobox', { name: 'Message', exact: true });
      const inputId = await input.getAttribute('id');
      await input.fill('A plain first turn.');
      await input.press('Enter');
      const continued = page.getByTestId('comment-thread');
      await expect(continued.getByTestId('comment-entry')).toHaveCount(1);
      await expect(input).toHaveAttribute('id', inputId!);
      const firstSend = await windowGeometry(continued);
      const newTurn = (await continued.getByTestId('comment-entry').boundingBox())!;
      // The attached thread has no whole-thread action row: measure the new turn itself.
      await expect(
        continued.locator('.conversation-message-content > .comment-actions'),
      ).toHaveCount(0);
      const turnSpacing = await continued.getByTestId('comment-entry').evaluate((node) => {
        const style = getComputedStyle(node);
        return parseFloat(style.marginTop) + parseFloat(style.marginBottom);
      });
      await expect(continued.locator('.annotation-reply-label')).toBeHidden();

      await annotate(page);
      const longQuote = 'This selected passage must scroll inside the annotation window. '
        .repeat(50)
        .trim();
      await page.getByRole('button', { name: 'Collapse thread', exact: true }).click();
      const frame = page.locator('#ask-page-fixture iframe');
      const render = await frame.getAttribute('data-render-id');
      await run(
        page,
        'change',
        `<p id="selected" style="margin:0;font:14px/1.5 sans-serif">${longQuote}</p>`,
      );
      await expect(frame).not.toHaveAttribute('data-render-id', render!);
      await expect(page.locator('#ask-page-fixture .status')).toContainText('Live');
      await page
        .frameLocator('#ask-page-fixture iframe')
        .locator('#selected')
        .evaluate((node) => {
          const range = node.ownerDocument.createRange();
          range.selectNodeContents(node);
          const selection = node.ownerDocument.getSelection()!;
          selection.removeAllRanges();
          selection.addRange(range);
          node.ownerDocument.defaultView!.focus();
        });
      // The long selection ends below this viewport. Alt+Enter opens a bounded
      // annotation without clicking an offscreen selection affordance.
      await page.keyboard.press('Alt+Enter');
      await expect(annotation).toBeVisible();
      await expect(annotation.locator('.annotation-anchor-note')).toHaveAttribute(
        'title',
        longQuote,
      );
      const long = await record(annotation, 'long-quote');
      const history = annotation.locator('.conversation-messages');
      await history.evaluate((node) => {
        node.scrollTop = node.scrollHeight;
      });
      const scrolled = await windowGeometry(annotation);

      await mount(page, 'thread');
      const thread = page.getByTestId('comment-thread');
      await record(thread, 'anchored-thread');
      expect(await windowStyles(thread)).toEqual(annotationStyles);
      // The quote is a compact footer hint; it never duplicates the long source in history.
      expect(short.history.height).toBe(0);
      expect(firstSend.window.height - short.window.height).toBeLessThanOrEqual(
        newTurn.height + turnSpacing + 8,
      );
      expect(firstSend.field.width).toBe(short.field.width);
      expect(firstSend.field.height).toBe(short.field.height);
      expect(long.scrollHeight).toBeLessThanOrEqual(long.clientHeight + 1);
      expect(long.window.top).toBeGreaterThanOrEqual(48);
      expect(long.window.bottom).toBeLessThanOrEqual(892);
      for (const bounds of [long.field, long.send]) {
        expect(bounds.top).toBeGreaterThanOrEqual(long.window.top);
        expect(bounds.bottom).toBeLessThanOrEqual(long.window.bottom);
      }
      expect(scrolled.header).toEqual(long.header);
      expect(scrolled.field).toEqual(long.field);
      expect(scrolled.send).toEqual(long.send);
      expect(scrolled.pageScroll).toBe(long.pageScroll);
    });

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`anchored annotation retains placement and editor on first Send at ${width} ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      await page.goto('/');
      await run(page, 'mount');
      await expect(page.locator('#ask-page-fixture .status')).toContainText('Live');
      const frame = page.locator('#ask-page-fixture iframe');
      const render = await frame.getAttribute('data-render-id');
      await run(
        page,
        'change',
        '<div style="height:600px"></div><p id="selected">Exact selected text</p>',
      );
      await expect(frame).not.toHaveAttribute('data-render-id', render!);
      await expect(page.locator('#ask-page-fixture .status')).toContainText('Live');
      const selected = page.frameLocator('#ask-page-fixture iframe').locator('#selected');
      await selected.evaluate((node) => {
        const range = node.ownerDocument.createRange();
        range.selectNodeContents(node);
        const selection = node.ownerDocument.getSelection()!;
        selection.removeAllRanges();
        selection.addRange(range);
      });
      await page.getByTestId('selection-ask').click();
      const dialog = page.getByRole('dialog', { name: 'Annotate selection' });
      const window = dialog.locator('.conversation-window');
      const before = await windowGeometry(window);
      const selection = (await selected.boundingBox())!;
      const input = dialog.getByRole('combobox', { name: 'Message', exact: true });
      await input.fill('First turn above the selection.');
      await input.press('Enter');
      await expect(window.getByTestId('comment-entry')).toHaveCount(1);
      const after = await windowGeometry(window);
      const directory = process.env.COLAB_2134_CAPTURE_DIR;
      if (directory) {
        mkdirSync(directory, { recursive: true });
        await page.mouse.move(width - 1, 899);
        const name = `${width}-${theme}-above-first-send`;
        await page.screenshot({ path: `${directory}/${name}.png` });
        writeFileSync(
          `${directory}/${name}.json`,
          JSON.stringify({ before, after, selection }, null, 2),
        );
      }
      for (const geometry of [before, after]) {
        expect(geometry.window.top).toBeGreaterThanOrEqual(48);
        expect(geometry.window.bottom).toBeLessThanOrEqual(892);
      }
      if (width === 390) expect(after.window.bottom).toBe(before.window.bottom);
      else expect(after.window.top).toBe(before.window.top);
      expect(after.field.width).toBe(before.field.width);
      expect(after.field.height).toBe(before.field.height);
      expect(after.pageScroll).toBe(before.pageScroll);
    });

for (const width of [1440, 390]) {
  for (const theme of ['light', 'dark'] as const) {
    test(`verified reply treatment updates without remount at ${width} ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      mkdirSync(captureDir, { recursive: true });
      for (const surface of ['chat', 'thread'] as const) {
        await mount(page, surface, 'replied');
        const window = page.getByTestId(surface === 'chat' ? 'chat-panel' : 'comment-thread');
        const agent = window.locator('.conversation-turn[data-turn-role="agent"]');
        const editor = window.locator('[contenteditable="true"]');
        const retained = await agent.elementHandle();
        await expect(agent).not.toHaveAttribute('data-running-driver');
        await expect(agent.locator('.agent-chip-driver')).toHaveCount(0);
        await expect(agent.locator('.agent-chip .lucide-bot')).toHaveCount(1);
        await expect(agent.getByTestId('ask-reply')).toHaveText(reply);
        await editor.focus();
        await capture(page, window, `driver-${surface}-neutral-${width}-${theme}`);
        for (const driver of ['claude', 'codex'] as const) {
          await run(page, 'driverObservation', { driver, held: true });
          await expect(agent).not.toHaveAttribute('data-running-driver');
          await run(page, 'settleDriverObservation');
          await expect(agent).toHaveAttribute('data-running-driver', driver);
          await expect(agent.locator('.agent-chip-driver')).toHaveText(`· ${driver}`);
          await expect(agent.locator('.agent-chip svg')).toHaveCount(1);
          await expect(agent.locator('.agent-chip-name')).toHaveCSS('font-weight', '500');
          const roleColor = await agent.evaluate(
            (node, driver) =>
              getComputedStyle(node)
                .getPropertyValue(
                  driver === 'claude' ? '--tmt-ui-color-review' : '--tmt-ui-color-link',
                )
                .trim(),
            driver,
          );
          // CSS variable may be hexadecimal; resolve the same role through a detached style probe.
          const color = await agent.evaluate((node, value) => {
            const probe = document.createElement('span');
            probe.style.color = value;
            node.append(probe);
            const color = getComputedStyle(probe).color;
            probe.remove();
            return color;
          }, roleColor);
          if (surface === 'chat') await expect(agent).toHaveCSS('border-left-color', color);
          else await expect(agent).toHaveCSS('border-left-width', '0px');
          await flat(agent);
          expect(await agent.evaluate((node, retained) => node === retained, retained)).toBe(true);
          await expect(editor).toBeFocused();
          await capture(page, window, `driver-${surface}-${driver}-${width}-${theme}`);
          await run(page, 'conversation', {
            surface,
            state: 'replied',
            agentName: 'A very long agent name with words and unbrokenSuffix'.repeat(3),
          });
          await expect(agent).toHaveAttribute('data-running-driver', driver);
          await shell(window);
          const overflow = await agent.evaluate((node) => node.scrollWidth - node.clientWidth);
          expect(overflow).toBeLessThanOrEqual(1);
          expect(await agent.evaluate((node, retained) => node === retained, retained)).toBe(true);
          await expect(editor).toBeFocused();
          await capture(page, window, `driver-${surface}-${driver}-long-${width}-${theme}`);
        }
        for (const options of [
          {},
          { driver: 'claude', mismatch: 'machine' },
          { driver: 'codex', mismatch: 'agent' },
          { driver: 'claude', failed: true },
        ]) {
          await run(page, 'driverObservation', options);
          await expect(agent).not.toHaveAttribute('data-running-driver');
          await expect(agent.locator('.agent-chip-driver')).toHaveCount(0);
          await expect(agent.locator('.agent-chip .lucide-bot')).toHaveCount(1);
          expect(await agent.evaluate((node, retained) => node === retained, retained)).toBe(true);
          await expect(editor).toBeFocused();
        }
        // An obsolete pending read cannot recolor a newer operation's observation.
        await run(page, 'driverObservation', { driver: 'claude', held: true });
        await run(page, 'driverObservation', { driver: 'codex' });
        await expect(agent).toHaveAttribute('data-running-driver', 'codex');
        await run(page, 'settleDriverObservation');
        await expect(agent).toHaveAttribute('data-running-driver', 'codex');
        await run(page, 'driverObservation', {});
        await expect(agent).not.toHaveAttribute('data-running-driver');
        await run(page, 'driverObservation', { driver: 'claude' });
        await expect(agent).toHaveAttribute('data-running-driver', 'claude');
        await run(page, 'driverClient', { enabled: false });
        await expect(agent).not.toHaveAttribute('data-running-driver');
        await run(page, 'driverClient', { enabled: true, held: true, driver: 'codex' });
        // Restoring the very same client must await its new observation, not reuse Claude.
        await expect(agent).not.toHaveAttribute('data-running-driver');
        await run(page, 'settleDriverObservation');
        await expect(agent).toHaveAttribute('data-running-driver', 'codex');
        expect(await agent.evaluate((node, retained) => node === retained, retained)).toBe(true);
        await expect(editor).toBeFocused();
        await run(page, 'driverObservation', {});
        await expect(agent).not.toHaveAttribute('data-running-driver');
        // Hostile authored body/name contain driver vocabulary and markup, without authority.
        await run(page, 'conversation', {
          surface,
          state: 'replied',
          agentName: 'claude codex <b data-running-driver="codex">',
          message: 'runningDriver=claude <style>.conversation-turn{color:red}</style>',
          reply:
            'claude codex <b data-running-driver=codex> <style>.conversation-turn{color:red}</style>',
        });
        await expect(agent).not.toHaveAttribute('data-running-driver');
        await expect(agent.locator('.conversation-author')).toHaveText(
          'claude codex <b data-running-driver="codex">',
        );
        await expect(
          window.locator(
            '.conversation-body style, .conversation-body script, .conversation-body img',
          ),
        ).toHaveCount(0);
        await expect(agent.getByTestId('ask-reply')).toHaveText(
          'claude codex <b data-running-driver=codex> <style>.conversation-turn{color:red}</style>',
        );
        expect(await agent.evaluate((node, retained) => node === retained, retained)).toBe(true);
        await expect(editor).toBeFocused();
      }
    });
  }
}

for (const [width, theme] of [
  [1440, 'light'],
  [390, 'dark'],
] as const)
  test(`Chat polish keeps identity, day boundaries and controls at ${width} ${theme}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.emulateMedia({ colorScheme: theme });
    await page.addInitScript((theme) => {
      document.documentElement.dataset.theme = theme;
    }, theme);
    await page.goto('/');
    await run(page, 'mount', { attachments: true, creator: true });
    await run(page, 'chatPolish', 'empty');
    await (await pageAction(page.locator('#ask-page-fixture'), 'Chat')).click();
    const panel = page.getByTestId('chat-panel');
    await expect(page.getByRole('button', { name: 'Close Chat', exact: true })).toBeVisible();
    await expect(panel.locator('.chat-empty h3')).toHaveText(text.chatEmpty);
    await expect(panel.locator('.chat-empty p')).toHaveText(text.chatEmptyHint);
    mkdirSync(captureDir, { recursive: true });
    await capture(page, panel, `chat-polish-empty-${width}-${theme}`);
    await run(page, 'chatPolish', 'messages');
    await expect(panel.locator('.chat-day')).toHaveCount(2);
    await expect(panel.locator('.chat-day').last()).toHaveText(text.chatToday);
    await expect(panel.locator('.conversation-author')).toHaveText(['Pat', 'You', 'Atlas']);
    const agent = panel.locator('[data-turn-role="agent"]');
    await expect(agent).toHaveCSS('border-left-width', '3px');
    await expect(panel.locator('.conversation-turn')).toHaveCount(3);
    for (const row of await panel.locator('.conversation-turn').all()) {
      await expect(row).toHaveCSS('border-bottom-width', '0px');
      const body = await row.locator('.conversation-body').boundingBox();
      const byline = await row.locator('.comment-byline').boundingBox();
      // Ben's shared byline keeps the time beside its separator, not on a
      // separate right edge; the own-message menu keeps its existing owner.
      expect(byline!.width).toBeGreaterThan(body!.width - 50);
      const gap = await row.locator('time').evaluate((time) => {
        const separator = time.previousElementSibling!.getBoundingClientRect();
        return time.getBoundingClientRect().left - separator.right;
      });
      expect(gap).toBeCloseTo(6, 0);
    }
    await expect(panel.getByTestId('ask-state')).toHaveCount(0);
    await capture(page, panel, `chat-polish-messages-${width}-${theme}`);
    await run(page, 'chatPolish', 'failed');
    await expect(panel.getByTestId('ask-state')).toContainText('✗Not delivered');
    await expect(panel.getByRole('button', { name: text.chatRetry, exact: true })).toBeVisible();
    await capture(page, panel, `chat-polish-failed-${width}-${theme}`);
    const input = panel.getByRole('combobox', { name: 'Message', exact: true });
    await input.fill('A comment without a recipient.');
    await expect(panel.locator('.annotation-hint[role="status"]')).toContainText(
      text.messageComment,
    );
    await expect(panel.locator('.creator-toggle')).toHaveCount(0);
    await capture(page, panel, `chat-polish-comment-${width}-${theme}`);
    // Lexical admits the selection through trusted editing keys.
    await input.press('ControlOrMeta+A');
    await input.press('Backspace');
    await expect(input).toHaveText('');
    await input.pressSequentially('@A');
    await page.getByRole('option').first().click();
    await expect(panel.locator('.annotation-hint[role="status"]')).not.toContainText('Asks ');
    await expect(panel.locator('.annotation-hint[role="status"]')).not.toContainText(
      text.messageComment,
    );
    await expect(panel.locator('.creator-toggle')).toBeVisible();
    await capture(page, panel, `chat-polish-recipient-${width}-${theme}`);
    await panel.getByTestId('attachment-input').setInputFiles([
      { name: 'notes.txt', mimeType: 'text/plain', buffer: Buffer.from('Notes') },
      { name: 'plan.txt', mimeType: 'text/plain', buffer: Buffer.from('Plan') },
      { name: 'review.txt', mimeType: 'text/plain', buffer: Buffer.from('Review') },
    ]);
    await expect(panel.getByTestId('attachment-chip')).toHaveCount(3);
    await capture(page, panel, `chat-polish-pending-${width}-${theme}`);
    expect(await run(page, 'proof')).toEqual({ sends: [], actions: [] });
  });

for (const [width, theme] of [
  [1440, 'light'],
  [390, 'dark'],
] as const)
  test(`archived empty Chat has no @ invitation ${width} ${theme}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.emulateMedia({ colorScheme: theme });
    await page.goto('/');
    await run(page, 'mount');
    await run(page, 'chatPolish', 'empty');
    await (await pageAction(page.locator('#ask-page-fixture'), 'Chat')).click();
    const chat = page.getByTestId('chat-panel');
    await expect(chat.getByText(text.chatEmptyHint, { exact: true })).toBeVisible();
    await run(page, 'archive');
    await expect(page.locator('#ask-page-fixture .status')).toContainText(text.archiveReadOnly);
    await expect(chat.locator('.chat-empty h3')).toHaveText(text.chatEmpty);
    await expect(chat.getByText(text.chatEmptyHint, { exact: true })).toHaveCount(0);
    await expect(chat.getByRole('combobox')).toHaveCount(0);
    expect(await run(page, 'proof')).toEqual({ sends: [], actions: [] });
  });

for (const surface of ['chat', 'thread'] as const)
  for (const width of [1440, 390])
    for (const theme of ['light', 'dark'] as const)
      test(`agent activity stays cosmetic and bounded: ${surface} ${width} ${theme}`, async ({
        page,
      }) => {
        await page.setViewportSize({ width, height: 900 });
        await page.emulateMedia({ colorScheme: theme, reducedMotion: 'no-preference' });
        await mount(page, surface);
        await page.evaluate((theme) => {
          document.documentElement.dataset.theme = theme;
        }, theme);
        const window =
          surface === 'chat' ? page.getByTestId('chat-panel') : page.getByTestId('comment-thread');
        const status = window.getByTestId('ask-state');
        await expect(status).toHaveText('Sent');
        await expect(status.locator('.agent-activity-spinner')).toHaveCount(0);
        expect(await turning(window)).toBe(0);
        const before = await run(page, 'proof');
        for (const [kind, label] of [
          ['working', /Atlas is working · 3:0[5-9]/],
          ['queued', /Atlas is finishing an earlier turn · 3m/],
          ['finished', /Atlas finished without replying · 3m ago/],
          ['unknown', /sent · Atlas's activity is not reported/],
        ] as const) {
          await run(page, 'conversationActivity', { kind });
          await expect(status).toContainText(label);
          const colours = await status.locator('.agent-activity').evaluate((line) => {
            const name = line.querySelector('.agent-activity-name, .agent-chip-name')!;
            const spinner = line.querySelector('.agent-activity-spinner');
            const mark = spinner ?? line.querySelector('.agent-activity-mark')!;
            return {
              line: getComputedStyle(line).color,
              name: getComputedStyle(name).color,
              mark: getComputedStyle(mark).color,
            };
          });
          expect(colours.name).toBe(colours.line);
          if (kind === 'working' || kind === 'finished')
            expect(colours.mark).not.toBe(colours.line);
          else expect(colours.mark).toBe(colours.line);
          await expect(status.locator('.agent-activity-spinner')).toHaveCount(
            kind === 'working' ? 1 : 0,
          );
          // The line's mark is the one animation of a working agent; no chip turns beside it.
          expect(await turning(window)).toBe(kind === 'working' ? 1 : 0);
          if (kind === 'working') {
            await expect(status.locator('.agent-activity-spinner')).toHaveCSS(
              'animation-name',
              'agent-activity-spin',
            );
            await page.emulateMedia({ reducedMotion: 'reduce' });
            await expect(status.locator('.agent-activity-spinner')).toHaveCSS(
              'animation-name',
              'none',
            );
            expect(await turning(window)).toBe(0);
            await expect(status).toContainText('Atlas is working');
            await page.emulateMedia({ reducedMotion: 'no-preference' });
            await page.emulateMedia({ forcedColors: 'active' });
            await expect(status.locator('.agent-activity-spinner')).toBeVisible();
            await expect(status.locator('.agent-activity')).toContainText('Atlas is working');
            await page.emulateMedia({ forcedColors: 'none' });
          }
          const captures = process.env.COLAB_ACTIVITY_CAPTURE_DIR;
          if (captures) {
            mkdirSync(captures, { recursive: true });
            await page.screenshot({ path: `${captures}/${surface}-${width}-${theme}-${kind}.png` });
          }
        }
        await run(page, 'conversationActivity', { kind: 'working', ageMs: 120_000 });
        await expect(status).toContainText("sent · Atlas's activity is not reported");
        await expect(status.locator('.agent-activity-spinner')).toHaveCount(0);
        await run(page, 'conversationActivity', { kind: 'working', active: false });
        await expect(status).toHaveText('Sent');
        expect(await run(page, 'proof')).toEqual(before);
        await run(page, 'conversation', { surface, state: 'replied' });
        await expect(window.getByTestId('ask-reply')).toHaveText(reply);
        await expect(status).toHaveCount(0);
      });

for (const surface of ['chat', 'thread'] as const)
  for (const width of [1440, 390])
    for (const theme of ['light', 'dark'] as const)
      test(`all page chips share the existing activity observation: ${surface} ${width} ${theme}`, async ({
        page,
      }) => {
        await page.setViewportSize({ width, height: 900 });
        await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
        await mount(page, surface);
        await page.evaluate((value) => {
          document.documentElement.dataset.theme = value;
        }, theme);
        await run(page, 'conversation', {
          surface,
          state: 'waiting',
          agentName: 'Agent 1',
          message: '@Agent 1 Please check this passage.',
        });
        const window =
          surface === 'chat' ? page.getByTestId('chat-panel') : page.getByTestId('comment-thread');
        const input = window.getByRole('combobox', { name: 'Message', exact: true });
        await input.fill('@');
        await page.getByRole('option').first().click();
        await input.press('End');
        await input.pressSequentially('Keep this draft.');
        const draft = await input.textContent();
        const before = await run(page, 'proof');
        // Earlier-turn working is still agent-wide working, even while this Ask is queued.
        await run(page, 'conversationActivity', { kind: 'queued', chipState: 'working' });
        const chips = window
          .locator('.agent-chip')
          .filter({ has: page.locator('.agent-chip-name').filter({ hasText: /^@?Agent 1$/ }) });
        await expect.poll(() => chips.count()).toBeGreaterThanOrEqual(3);
        await expect(input.locator('.agent-chip-mark[data-working]')).toBeVisible();
        expect(await turning(window)).toBe(0);
        for (const chip of await chips.all()) {
          // A working chip is the robot with a static dot, and says so in words.
          await expect(chip.locator('.agent-chip-mark[data-working]')).toHaveCount(1);
          await expect(chip.locator('.agent-chip-icon')).toBeVisible();
          await expect(chip.locator('.agent-chip-state')).toHaveText(', working');
          expect(
            await chip.evaluate(
              (node) =>
                getComputedStyle(node.querySelector('.agent-chip-name')!).color ===
                getComputedStyle(node).color,
            ),
          ).toBe(true);
        }
        await expect(window.getByTestId('ask-state')).toContainText('finishing an earlier turn');
        const captures = process.env.COLAB_ACTIVITY_CAPTURE_DIR;
        if (captures) {
          mkdirSync(captures, { recursive: true });
          await page.screenshot({ path: `${captures}/all-chips-${surface}-${width}-${theme}.png` });
        }
        // A known working agent with an unclassifiable turn clock keeps plain Sent.
        await run(page, 'conversationActivity', { kind: 'sent', chipState: 'working' });
        await expect(window.getByTestId('ask-state')).toHaveText('Sent');
        await expect(input.locator('.agent-chip-mark[data-working]')).toBeVisible();
        expect(await turning(window)).toBe(0);
        if (captures)
          await page.screenshot({
            path: `${captures}/clock-incomplete-${surface}-${width}-${theme}.png`,
          });
        for (const observation of [
          { chipState: 'idle' },
          { chipState: 'working', ageMs: 120_000 },
          { chipState: 'working', active: false },
        ]) {
          await run(page, 'conversationActivity', { kind: 'queued', ...observation });
          await expect(window.locator('.agent-chip-mark[data-working]')).toHaveCount(0);
          await expect(chips.first().locator('.agent-chip-icon')).toBeVisible();
        }
        await expect(input).toHaveText(draft!);
        expect(await run(page, 'proof')).toEqual(before);
      });
