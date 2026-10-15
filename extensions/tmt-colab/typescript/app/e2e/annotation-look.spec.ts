import { mkdirSync } from 'node:fs';
import { expect, test, type Locator, type Page } from '@playwright/test';
import { captureDirectory } from './captures.js';
import { destination } from '../test/ask-fixtures.js';
import { pageAction } from '../test/page-actions.js';
import { text } from '../src/strings.js';

const fixture = '/test/ask-page-browser.tsx';
async function run(page: Page, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
const captures = captureDirectory();
for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`annotation design matrix ${width} ${theme}`, async ({ page }) => {
      mkdirSync(captures, { recursive: true });
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      await page.goto('/');
      await run(page, 'mount', {
        creator: true,
        agents: [{ ...destination(), agentName: 'Atlas', presence: 'active' }],
      });
      await run(page, 'annotationLook');
      const app = page.locator('#ask-page-fixture');
      const frame = page.frameLocator('#ask-page-fixture iframe');
      const markers = frame.locator('[data-colab-thread]');
      await expect(markers).toHaveCount(4);
      await expect(markers).toHaveText(['1', '2', '3', '4']);
      for (const marker of await markers.all()) {
        await expect(marker).toHaveCSS('border-top-width', '0px');
        await expect(marker).toHaveCSS('box-shadow', 'none');
        await expect(marker).toHaveCSS('color', 'rgb(255, 255, 255)');
      }
      const authored = await frame.locator('body').evaluate((body) => {
        const clone = body.cloneNode(true) as HTMLElement;
        clone.querySelectorAll('[data-colab-highlight]').forEach((node) => node.remove());
        return clone.innerHTML;
      });
      mkdirSync(captures, { recursive: true });
      await page.screenshot({ path: `${captures}/annotation-ranges-${width}-${theme}.png` });
      const range = frame.locator('#selected');
      await range.evaluate((node) => {
        const range = node.ownerDocument.createRange();
        range.selectNodeContents(node);
        const selection = node.ownerDocument.getSelection()!;
        selection.removeAllRanges();
        selection.addRange(range);
      });
      const annotate = page.getByTestId('selection-ask');
      await expect(annotate).toHaveText('Annotate');
      await expect(annotate).toHaveAttribute('aria-keyshortcuts', 'c Alt+Enter');
      await expect(annotate.locator('svg')).toBeVisible();
      await expect(annotate.locator('kbd')).toHaveCount(0);
      await page.screenshot({ path: `${captures}/annotation-affordance-${width}-${theme}.png` });
      await annotate.click();
      const window = page.getByTestId('annotation-window');
      await expect(window).toBeVisible();
      const editor = window.getByRole('combobox', { name: 'Message', exact: true });
      await editor.fill('Could @Atlas check this at 390px?');
      const creator = window.getByRole('button', { name: 'Send to @Atlas', exact: true });
      await expect(creator).toHaveAttribute('aria-pressed', 'true');
      await expect(creator).toHaveCSS('border-top-width', '0px');
      await expect(creator.locator('.tmt-ui-toggle-indicator')).toHaveCSS(
        'border-top-width',
        '0px',
      );
      await creator.focus();
      await expect(creator).toHaveCSS('outline-style', 'solid');
      await editor.focus();
      await page.screenshot({ path: `${captures}/annotation-new-${width}-${theme}.png` });
      if (width === 390) {
        const box = (await page.locator('.annotation-popover').boundingBox())!;
        expect(box.y + box.height).toBeGreaterThan(800);
        await expect(page.locator('.annotation-popover')).not.toHaveAttribute('data-leader');
      }
      await page.keyboard.press('Escape');
      await markers.filter({ hasText: /^1$/ }).click();
      const thread = page.getByTestId('comment-thread');
      await expect(thread).toBeVisible();
      await expect(thread.locator('.annotation-number')).toHaveText('1');
      const human = thread.locator('.conversation-turn[data-turn-role="user"]');
      const agent = thread.locator('.conversation-turn[data-turn-role="agent"]');
      await expect(human.locator('.lucide-user-round')).toBeVisible();
      await expect(agent.locator('.lucide-bot')).toBeVisible();
      await expect(agent.locator('.conversation-agent-label')).toContainText('AGENT');
      await expect(agent).toHaveCSS('border-left-width', '0px');
      await expect(human).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
      await expect(
        human.locator('.conversation-body .message-mention[data-kind="agent"]'),
      ).toHaveCount(2);
      await page.screenshot({ path: `${captures}/annotation-thread-${width}-${theme}.png` });
      await page.keyboard.press('Escape');
      const comments = await pageAction(app, 'Comments');
      await comments.click();
      const panel = page.getByTestId('comments-panel');
      await expect(panel.locator('.annotation-row .annotation-number')).toHaveText([
        '1',
        '2',
        '3',
        '4',
        '5',
      ]);
      await panel.locator('.annotation-row').filter({ hasText: 'old decision text' }).click();
      const resolved = panel.getByTestId('comment-thread');
      await expect(
        resolved.getByRole('button', { name: 'Show conversation', exact: true }),
      ).toBeVisible();
      await expect(resolved.locator('.conversation-history')).toBeHidden();
      await expect(resolved.locator('.resolved-summary')).toContainText(
        '1 messages · 0 attachments',
      );
      await resolved.scrollIntoViewIfNeeded();
      const rejectedDrop = await resolved.evaluate((element) => {
        const dataTransfer = new DataTransfer();
        dataTransfer.items.add(new File(['draft'], 'hidden.txt', { type: 'text/plain' }));
        const event = new DragEvent('drop', { bubbles: true, cancelable: true, dataTransfer });
        element.dispatchEvent(event);
        return event.defaultPrevented;
      });
      expect(rejectedDrop).toBe(true);
      await page.screenshot({ path: `${captures}/annotation-resolved-${width}-${theme}.png` });
      await resolved.getByRole('button', { name: 'Show conversation', exact: true }).click();
      await expect(resolved.locator('.conversation-history')).toBeVisible();
      await expect(resolved.locator('.attachment-pending')).toHaveCount(0);
      await resolved.scrollIntoViewIfNeeded();
      await page.screenshot({
        path: `${captures}/annotation-resolved-expanded-${width}-${theme}.png`,
      });
      await run(page, 'annotationLookDelete');
      await expect(panel.locator('.annotation-row .annotation-number')).toHaveText([
        '1',
        '2',
        '3',
        '4',
      ]);
      await expect(markers).toHaveText(['1', '2', '3']);
      expect(
        await frame.locator('body').evaluate((body) => {
          const clone = body.cloneNode(true) as HTMLElement;
          clone.querySelectorAll('[data-colab-highlight]').forEach((node) => node.remove());
          return clone.innerHTML;
        }),
      ).toBe(authored);
      expect(await run(page, 'proof')).toMatchObject({ sends: [], actions: [] });
    });
  }

async function numberMetrics(number: Locator) {
  let metrics: Record<string, string> | undefined;
  await expect
    .poll(
      async () => {
        metrics = await number.evaluate((node) => {
          const style = getComputedStyle(node);
          const rect = node.getBoundingClientRect();
          const values = Object.fromEntries(
            [
              'width',
              'height',
              'font-family',
              'font-size',
              'font-weight',
              'line-height',
              'align-items',
              'justify-items',
              'padding',
              'border-radius',
              'letter-spacing',
            ].map((key) => [key, style.getPropertyValue(key)]),
          );
          // Capture readiness and metrics together: resolving annotations can replace the node.
          return node.isConnected &&
            rect.width > 0 &&
            rect.height > 0 &&
            (style.display === 'inline-grid' || style.display === 'grid') &&
            Object.values(values).every((value) => value !== '')
            ? values
            : undefined;
        });
        return metrics !== undefined;
      },
      { message: 'Number circle must be connected, styled and laid out' },
    )
    .toBe(true);
  return metrics!;
}
test('number metrics wait for a replacement marker to be styled and laid out', async ({ page }) => {
  await page.setContent('<iframe title="Marker lifecycle"></iframe>');
  const frame = page.frameLocator('iframe');
  await frame.locator('body').evaluate((body) => {
    body.innerHTML =
      '<style>.annotation-number { display: inline-grid; position: absolute; width: 22px; height: 22px; }</style><button class="annotation-number">1</button>';
    const view = body.ownerDocument.defaultView!;
    const computedStyle = view.getComputedStyle.bind(view);
    let replaced = false;
    view.getComputedStyle = (node, pseudo) => {
      if (!replaced && node.matches('.annotation-number')) {
        replaced = true;
        node.replaceWith(node.cloneNode(true));
        body.dataset.replacements = '1';
      }
      return computedStyle(node, pseudo);
    };
  });
  const marker = frame.locator('.annotation-number');
  const metrics = await numberMetrics(marker);
  await expect(marker).toHaveCSS('display', 'grid');
  expect(metrics).toMatchObject({ width: '22px', height: '22px' });
  await expect(frame.locator('body')).toHaveAttribute('data-replacements', '1');
});

async function sharedComposer(window: Locator) {
  const compose = window.locator('.annotation-compose');
  await expect(compose).toHaveCount(1);
  await expect(compose.locator('.rich-input-field')).toHaveCount(1);
  const send = compose.getByRole('button', { name: /^Send(?: · to .+)?$/, exact: true });
  await expect(send.locator('.lucide-arrow-right')).toHaveCount(1);
  const geometry = await compose.evaluate((node) => {
    const field = node.querySelector('.rich-input-field')!;
    const attach = node.querySelector('.attach-labeled')!;
    const send = node.querySelector(
      '.annotation-status-row > .tmt-ui-icon-action:last-child button',
    )!;
    const a = field.getBoundingClientRect(),
      b = attach.getBoundingClientRect(),
      c = send.getBoundingClientRect();
    const style = getComputedStyle(field);
    const composer = getComputedStyle(node.parentElement!);
    return {
      left: b.left - a.left,
      right: c.right - a.right,
      square: c.width - c.height,
      border: style.borderTopWidth,
      background: style.backgroundColor,
      padding: composer.padding,
      rule: composer.borderTopWidth,
    };
  });
  expect(Math.abs(geometry.left)).toBeLessThanOrEqual(1);
  expect(Math.abs(geometry.right)).toBeLessThanOrEqual(1);
  expect(Math.abs(geometry.square)).toBeLessThanOrEqual(1);
  expect(geometry).toMatchObject({
    border: '0px',
    background: 'rgba(0, 0, 0, 0)',
    padding: '12px 16px 16px',
    rule: '1px',
  });
  return geometry;
}
async function sharedAgentTag(window: Locator) {
  const byline = window.locator('[data-turn-role="agent"] .comment-byline');
  const tag = byline.locator('.conversation-agent-label');
  await expect(tag).toHaveCount(1);
  await expect(tag).toHaveText('AGENT');
  await expect(tag).toHaveCSS('border-top-width', '1px');
  const gap = await byline.evaluate((node) => {
    const name = node.querySelector('.conversation-author')!.getBoundingClientRect();
    const tag = node.querySelector('.conversation-agent-label')!.getBoundingClientRect();
    const time = node.querySelector('time')!.getBoundingClientRect();
    return { gap: tag.left - name.right, tagRight: tag.right, timeLeft: time.left };
  });
  expect(gap.gap).toBeCloseTo(6, 0);
  expect(gap.timeLeft).toBeGreaterThan(gap.tagRight);
  return tag.evaluate((node) => {
    const style = getComputedStyle(node);
    return {
      font: style.font,
      padding: style.padding,
      border: style.borderTopWidth,
      letterSpacing: style.letterSpacing,
    };
  });
}
for (const width of [1440, 1024, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`annotation and Chat share circles composer and AGENT tag ${width} ${theme}`, async ({
      page,
    }) => {
      mkdirSync(captures, { recursive: true });
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.goto('/');
      const options = {
        creator: true,
        attachments: true,
        agents: [{ ...destination(), agentName: 'Atlas', presence: 'active' }],
      };
      await run(page, 'mount', options);
      await run(page, 'annotationLook');
      const marker = page
        .frameLocator('#ask-page-fixture iframe')
        .locator('[data-colab-thread]')
        .filter({ hasText: /^1$/ });
      await marker.click();
      const thread = page.getByTestId('comment-thread');
      await expect(thread).toBeVisible();
      const circle = thread.locator('.annotation-number');
      const metrics = await numberMetrics(marker);
      expect(await numberMetrics(circle)).toEqual(metrics);
      expect(metrics).toMatchObject({
        width: '22px',
        height: '22px',
        'font-size': '12px',
        'font-weight': '600',
        'align-items': 'center',
        'justify-items': 'center',
      });
      expect(
        await marker.evaluate((node) => ({
          children: node.children.length,
          before: getComputedStyle(node, '::before').content,
          after: getComputedStyle(node, '::after').content,
        })),
      ).toEqual({ children: 0, before: 'none', after: 'none' });
      const popover = page.locator('.annotation-popover');
      await expect(popover).not.toHaveAttribute('data-leader');
      expect(await popover.evaluate((node) => getComputedStyle(node, '::before').content)).toBe(
        'none',
      );
      const annotationComposer = await sharedComposer(thread);
      const annotationTag = await sharedAgentTag(thread);
      await page.screenshot({ path: `${captures}/unified-annotation-${width}-${theme}.png` });
      await page.keyboard.press('Escape');
      await run(page, 'mount', { ...options, creator: false });
      const app = page.locator('#ask-page-fixture');
      await (await pageAction(app, 'Chat')).click();
      const chat = page.getByTestId('chat-panel');
      await expect(chat.getByText(text.chatEmpty, { exact: true })).toBeVisible();
      await expect(chat.locator('.message-mention-chip')).toHaveCount(0);
      expect(await sharedComposer(chat)).toEqual(annotationComposer);
      await page.screenshot({ path: `${captures}/unified-chat-empty-${width}-${theme}.png` });
      await run(page, 'mount', options);
      await (await pageAction(app, 'Chat')).click();
      await expect(chat.locator('.message-mention-chip')).toHaveCount(1);
      await expect(chat.getByRole('button', { name: 'Send to @Atlas', exact: true })).toHaveCSS(
        'border-top-width',
        '0px',
      );
      await page.screenshot({ path: `${captures}/unified-chat-recipient-${width}-${theme}.png` });
      await run(page, 'conversation', { surface: 'chat', state: 'replied' });
      expect(await sharedAgentTag(chat)).toEqual(annotationTag);
      await page.screenshot({ path: `${captures}/unified-chat-agent-${width}-${theme}.png` });
      expect(await run(page, 'proof')).toMatchObject({ sends: [], actions: [] });
    });
  }
