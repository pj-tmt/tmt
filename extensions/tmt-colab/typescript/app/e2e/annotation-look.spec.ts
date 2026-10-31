import { mkdirSync } from 'node:fs';
import { expect, test, type Locator, type Page } from '@playwright/test';
import { captureDirectory, capturePath } from './captures.js';
import { readFrameStyle } from './frame-style.js';
import { destination } from '../test/ask-fixtures.js';
import { commentsClose, commentsRegion } from '../test/comments-region.js';
import {
  pageAction,
  showResolvedOn,
  chooseThreadFilter,
  selectDiscussion,
} from '../test/page-actions.js';
import { text } from '../src/strings.js';
import { rgb, tokens } from './tokens.js';

const fixture = '/test/ask-page-browser.tsx';
async function run(page: Page, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
const captures = captureDirectory();
const rule = {
  light: rgb(tokens.browser.surface.rule.light),
  dark: rgb(tokens.browser.surface.rule.dark),
};
/** The panel's own edge: 1px solid --tmt-ui-surface-rule on all four sides, once (no child repeats it). */
async function expectRuleEdge(panel: Locator, theme: 'light' | 'dark') {
  for (const side of ['top', 'right', 'bottom', 'left']) {
    await expect(panel).toHaveCSS(`border-${side}-width`, '1px');
    await expect(panel).toHaveCSS(`border-${side}-style`, 'solid');
    await expect(panel).toHaveCSS(`border-${side}-color`, rule[theme]);
  }
  // Inner dividers are fine; a child must not repeat the edge on a side that touches the panel's own.
  // A composer's suggestion list floats inside its dialog with its own edge; it is not part of the panel's frame.
  const children = panel.locator(':scope > :not(.tmt-listbox-options)');
  const count = await children.count();
  for (const side of ['left', 'right'])
    for (const child of await children.all())
      await expect(child).toHaveCSS(`border-${side}-width`, '0px');
  await expect(children.first()).toHaveCSS('border-top-width', '0px');
  await expect(children.nth(count - 1)).toHaveCSS('border-bottom-width', '0px');
}
/** The floating Comments panel draws the leaf edge on all four sides. */
async function expectCommentsEdge(panel: Locator, theme: 'light' | 'dark') {
  for (const side of ['top', 'right', 'bottom', 'left']) {
    await expect(panel).toHaveCSS(`border-${side}-width`, '1px');
    await expect(panel).toHaveCSS(`border-${side}-color`, rgb(tokens.browser.color.edge[theme]));
  }
}
for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`annotation design matrix ${width} ${theme}`, async ({ page }) => {
      mkdirSync(captures, { recursive: true });
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      // The matrix lists every thread, resolved ones included.
      await showResolvedOn(page);
      await page.goto('/');
      await run(page, 'mount', {
        creator: true,
        agents: [{ ...destination(), agentName: 'Atlas', presence: 'active' }],
      });
      await run(page, 'annotationLook');
      const app = page.locator('#ask-page-fixture');
      const frame = page.frameLocator('#ask-page-fixture iframe');
      // The two notes on one line share the right lane as one `+2` marker (#4489).
      const markers = frame.locator('.annotation-number');
      await expect(markers).toHaveCount(3);
      await expect(markers).toHaveText(['1', '2', '+2']);
      for (const marker of await markers.all()) {
        // A number has no border; the `+2` pill's target is a transparent one.
        const pill = (await marker.getAttribute('data-colab-group')) !== null;
        await expect(marker).toHaveCSS('border-top-width', pill ? '4px' : '0px');
        await expect(marker).toHaveCSS('border-top-color', pill ? 'rgba(0, 0, 0, 0)' : /.*/);
        await expect(marker).toHaveCSS('box-shadow', 'none');
        await expect(marker).toHaveCSS(
          'color',
          rgb(tokens.browser.color['annotation-on-action'][theme]),
        );
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
      // An icon-only round control: its name is the shared tooltip and the accessible name.
      const annotate = page.getByRole('button', { name: 'Annotate', exact: true });
      await expect(annotate).toHaveText('');
      await expect(annotate).toHaveAttribute('aria-keyshortcuts', 'c Alt+Enter');
      await expect(annotate.locator('svg')).toBeVisible();
      await expect(annotate.locator('kbd')).toHaveCount(0);
      const control = (await annotate.boundingBox())!;
      expect([control.width, control.height]).toEqual([34, 34]);
      await expect(annotate).toHaveCSS('border-radius', '999px');
      if (width > 640) {
        // No connector, and the control hangs from the selection's lower right corner.
        const frameBox = (await page.locator('#ask-page-fixture iframe').boundingBox())!;
        const text = await range.evaluate((node) => {
          const text = node.ownerDocument.createRange();
          text.selectNodeContents(node);
          const { x, y, width, height } = text.getBoundingClientRect();
          return { x, y, width, height };
        });
        const selected = { ...text, x: frameBox.x + text.x, y: frameBox.y + text.y };
        expect(Math.abs(control.y - (selected.y + selected.height))).toBeLessThanOrEqual(8);
        expect(Math.abs(control.x - (selected.x + selected.width - 6))).toBeLessThanOrEqual(2);
      }
      await page.screenshot({ path: `${captures}/annotation-affordance-${width}-${theme}.png` });
      await annotate.click();
      const window = page.getByTestId('annotation-window');
      await expect(window).toBeVisible();
      await expectRuleEdge(page.locator('.annotation-popover'), theme);
      const editor = window.getByRole('combobox', { name: 'Message', exact: true });
      await editor.fill('Could @Atlas check this at 390px?');
      // Who is asked is the pill in the field's toolbar, a small round-edged tag that names the agent.
      const pill = window.getByRole('button', { name: /^Who is asked/ });
      await expect(pill).toHaveText('Ask Atlas');
      await expect(pill).toHaveCSS('border-top-left-radius', '999px');
      await pill.focus();
      await expect(pill).toHaveCSS('outline-style', 'solid');
      await editor.focus();
      await page.screenshot({ path: `${captures}/annotation-new-${width}-${theme}.png` });
      if (width === 390) {
        const box = (await page.locator('.annotation-popover').boundingBox())!;
        expect(box.y + box.height).toBeGreaterThan(800);
        await expect(page.locator('.annotation-popover')).not.toHaveAttribute('data-leader');
      }
      await page.keyboard.press('Escape');
      // Collapse the browser selection too, so its Annotate control does not float over the sheet the
      // marker opens below.
      await range.evaluate((node) => node.ownerDocument.getSelection()!.removeAllRanges());
      // The passage at rest is not selected; the one whose thread window opens takes the selected look
      // (#4490) and returns to rest when the window closes.
      const selectedPassages = () =>
        frame.locator('html').evaluate(() => {
          const selected = CSS.highlights.get('colab-selected');
          return selected ? [...selected].map((range) => range.toString()) : null;
        });
      expect(await selectedPassages()).toBeNull();
      await markers.filter({ hasText: /^1$/ }).click();
      const thread = page.getByTestId('comment-thread');
      await expect(thread).toBeVisible();
      await expect.poll(selectedPassages).toEqual(['Check the mobile layout before release.']);
      // The open thread's marker, and only that one, wears the ring: 2 px in the selected rule colour,
      // drawn inside the 24 px target.
      const ringed = frame.locator('.annotation-number[data-colab-selected]');
      await expect(ringed).toHaveText(['1']);
      await expect(ringed).toHaveCSS('outline-width', '2px');
      await expect(ringed).toHaveCSS('outline-style', 'solid');
      await expect(ringed).toHaveCSS('outline-offset', '-2px');
      await expect(ringed).toHaveCSS(
        'outline-color',
        rgb(tokens.browser.color['annotation-rule-selected'][theme]),
      );
      // A marker opens the thread's Discussion view in the panel, which draws its mode's edge.
      await expectCommentsEdge(commentsRegion(page), theme);
      await expect(thread.locator('.annotation-number')).toHaveText('1');
      const human = thread.locator('.conversation-turn[data-turn-role="user"]');
      const agent = thread.locator('.conversation-turn[data-turn-role="agent"]');
      // The avatar carries the kind: an agent's mark is drawn apart from a person's at a glance.
      const personMark = human.locator('header > .tmt-ui-avatar');
      const agentMark = agent.locator('header > .tmt-ui-avatar');
      await expect(personMark).toBeVisible();
      await expect(agentMark).toBeVisible();
      await expect(personMark).toHaveAttribute('data-kind', 'person');
      await expect(agentMark).toHaveAttribute('data-kind', 'agent');
      const fill = (mark: Locator) =>
        mark.evaluate((node) => getComputedStyle(node).backgroundColor);
      expect(await fill(agentMark)).not.toBe(await fill(personMark));
      await expect(agent.locator('.comment-byline .lucide-bot')).toHaveCount(0);
      await expect(agent.locator('.conversation-agent-label')).toHaveCount(0);
      await expect(agent).toHaveCSS('border-left-width', '0px');
      await expect(human).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
      await expect(human.locator('.conversation-body .agent-chip')).toHaveCount(2);
      await page.screenshot({ path: `${captures}/annotation-thread-${width}-${theme}.png` });
      await commentsClose(page, 'discussion').click();
      await expect.poll(selectedPassages).toBeNull();
      await expect(frame.locator('.annotation-number[data-colab-selected]')).toHaveCount(0);
      const comments = await pageAction(app, 'Comments');
      await comments.click();
      const panel = page.getByTestId('comments-panel');
      await expectCommentsEdge(commentsRegion(page), theme);
      await chooseThreadFilter(page, 'All');
      await expect(commentsRegion(page).locator('.discussion-position')).toHaveText('1 / 5');
      await selectDiscussion(page, 'old decision text');
      const resolved = panel.getByTestId('comment-thread');
      await expect(resolved.locator('.conversation-history')).toBeVisible();
      await expect(resolved.locator('.resolved-summary')).toHaveCount(0);
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
      await expect(resolved.locator('.conversation-history')).toBeVisible();
      await expect(resolved.locator('.attachment-pending')).toHaveCount(0);
      await resolved.scrollIntoViewIfNeeded();
      await page.screenshot({
        path: `${captures}/annotation-resolved-expanded-${width}-${theme}.png`,
      });
      await run(page, 'annotationLookDelete');
      await expect(commentsRegion(page).locator('.discussion-position')).toContainText('/ 4');
      await expect(markers).toHaveText(['1', '+2']);
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

/** Chat, replies and the annotation window send with the same field: one box on the sheet surface
    with its toolbar inside it and the round arrow Send at the toolbar's end. */
async function sharedComposer(window: Locator, surface: 'annotation' | 'chat') {
  const compose = window.locator('.annotation-compose');
  await expect(compose).toHaveCount(1);
  await expect(compose.locator('.rich-input-field')).toHaveCount(1);
  const send = compose.getByRole('button', { name: 'Send', exact: true });
  await expect(send.locator('.lucide-arrow-up')).toHaveCount(1);
  await expect(send).toHaveText('');
  const geometry = await compose.evaluate((node) => {
    const box = node.querySelector('.rich-input-surface')!;
    const send = node.querySelector('.message-toolbar-end .tmt-ui-icon-action button')!;
    const tools = node.querySelector('.message-toolbar-tools')!;
    const a = box.getBoundingClientRect(),
      b = tools.getBoundingClientRect(),
      c = send.getBoundingClientRect();
    const style = getComputedStyle(box);
    const composer = getComputedStyle(node.parentElement!);
    return {
      // The toolbar sits 6px in from the box's edges.
      left: b.left - a.left,
      right: a.right - c.right,
      square: c.width - c.height,
      border: style.borderTopWidth,
      background: style.backgroundColor,
      padding: composer.padding,
      rule: composer.borderTopWidth,
    };
  });
  expect(Math.abs(geometry.left - 7)).toBeLessThanOrEqual(1);
  expect(Math.abs(geometry.right - 7)).toBeLessThanOrEqual(1);
  expect(Math.abs(geometry.square)).toBeLessThanOrEqual(1);
  expect(geometry).toMatchObject({
    border: '1px',
    padding: '12px 16px 16px',
    rule: surface === 'annotation' ? '1px' : '0px',
  });
  expect(geometry.background).not.toBe('rgba(0, 0, 0, 0)');
}
/**
 * The agent byline is the shared chip's name with no boxed role badge. A turn leads with the shared
 * avatar, in a thread and in Chat alike, so the chip drops its icon.
 */
async function sharedAgentByline(window: Locator) {
  const byline = window.locator('[data-turn-role="agent"] .comment-byline');
  await expect(byline.locator('.conversation-agent-label')).toHaveCount(0);
  const chip = byline.locator('.conversation-author .agent-chip');
  await expect(chip).toHaveCount(1);
  await expect(chip.locator('.lucide-bot')).toHaveCount(0);
  await expect(window.locator('[data-turn-role="agent"] .tmt-ui-avatar')).toHaveCount(1);
  await expect(chip).toHaveCSS('border-top-width', '0px');
  const geometry = await byline.evaluate((node) => {
    const chip = node.querySelector('.conversation-author')!.getBoundingClientRect();
    const time = node.querySelector('time')!.getBoundingClientRect();
    return { chipRight: chip.right, timeLeft: time.left };
  });
  expect(geometry.timeLeft).toBeGreaterThan(geometry.chipRight);
  return chip.locator('.agent-chip-name').evaluate((node) => {
    const style = getComputedStyle(node);
    return { weight: style.fontWeight, size: style.fontSize, letterSpacing: style.letterSpacing };
  });
}
for (const width of [1440, 1024, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`annotation and Chat share circles composer and agent byline ${width} ${theme}`, async ({
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
      const frame = page.frameLocator('#ask-page-fixture iframe');
      // The annotation window (a new selection) is the composer the other surfaces match.
      await frame.locator('#selected').evaluate((node) => {
        const range = node.ownerDocument.createRange();
        range.selectNodeContents(node);
        const selection = node.ownerDocument.getSelection()!;
        selection.removeAllRanges();
        selection.addRange(range);
      });
      await page.getByTestId('selection-ask').click();
      const annotationWindow = page.getByTestId('annotation-window');
      await expect(annotationWindow).toBeVisible();
      const popover = page.locator('.annotation-popover');
      await expect(popover).not.toHaveAttribute('data-leader');
      expect(await popover.evaluate((node) => getComputedStyle(node, '::before').content)).toBe(
        'none',
      );
      await sharedComposer(annotationWindow, 'annotation');
      await page.keyboard.press('Escape');
      await expect(annotationWindow).toHaveCount(0);
      // A marker opens the thread's Discussion view; its number circle and agent byline are shared.
      const marker = frame.locator('[data-colab-thread]').filter({ hasText: /^1$/ });
      await marker.click();
      const thread = page.getByTestId('comment-thread');
      await expect(thread).toBeVisible();
      const circle = thread.locator('.annotation-number');
      const metrics = await numberMetrics(marker);
      const panelMetrics = await numberMetrics(circle);
      // One number circle in two sizes (#4489): the page's marker is a 16 px circle in a 24 px
      // target, the Discussion title's number an 18 px circle. They share typeface, weight and alignment.
      for (const key of [
        'font-family',
        'font-weight',
        'align-items',
        'justify-items',
        'letter-spacing',
        'border-radius',
      ])
        expect(panelMetrics[key], key).toBe(metrics[key]);
      expect(panelMetrics).toMatchObject({
        width: '18px',
        height: '18px',
        'font-size': '11px',
        'font-weight': '600',
        'align-items': 'center',
        'justify-items': 'center',
      });
      expect(metrics).toMatchObject({
        width: '24px',
        height: '24px',
        padding: '4px',
        'font-size': '10px',
        'font-weight': '600',
        'align-items': 'center',
        'justify-items': 'center',
      });
      expect(
        await readFrameStyle(page.frameLocator('#ask-page-fixture iframe'), (body) => {
          const node = [...body.ownerDocument.querySelectorAll('[data-colab-thread]')].find(
            (candidate) => candidate.textContent?.trim() === '1',
          );
          if (!node?.isConnected) return undefined;
          return {
            children: node.children.length,
            before: getComputedStyle(node, '::before').content,
            after: getComputedStyle(node, '::after').content,
          };
        }),
      ).toEqual({ children: 0, before: 'none', after: 'none' });
      const annotationByline = await sharedAgentByline(thread);
      await page.screenshot({ path: `${captures}/unified-annotation-${width}-${theme}.png` });
      await page.keyboard.press('Escape');
      await run(page, 'mount', { ...options, creator: false });
      const app = page.locator('#ask-page-fixture');
      await (await pageAction(app, 'Chat')).click();
      const chat = page.getByTestId('chat-panel');
      await expect(chat.getByText(text.chatEmpty, { exact: true })).toBeVisible();
      await expect(chat.locator('.agent-chip[data-variant="chip"]')).toHaveCount(0);
      await sharedComposer(chat, 'chat');
      await page.screenshot({ path: `${captures}/unified-chat-empty-${width}-${theme}.png` });
      await run(page, 'mount', options);
      await (await pageAction(app, 'Chat')).click();
      await expect(chat.locator('.agent-chip[data-variant="chip"]')).toHaveCount(1);
      await expect(chat.getByRole('button', { name: /^Who is asked/ })).toHaveText('Ask Atlas');
      await page.screenshot({ path: `${captures}/unified-chat-recipient-${width}-${theme}.png` });
      await run(page, 'conversation', { surface: 'chat', state: 'replied' });
      const chatByline = await sharedAgentByline(chat);
      // The same name in the same weight; the Discussion view's is the 12px of the design, Chat's its 11px.
      expect({ ...chatByline, size: undefined }).toEqual({ ...annotationByline, size: undefined });
      expect([annotationByline.size, chatByline.size]).toEqual(['12px', '11px']);
      await page.screenshot({ path: `${captures}/unified-chat-agent-${width}-${theme}.png` });
      expect(await run(page, 'proof')).toMatchObject({ sends: [], actions: [] });
    });
  }

for (const width of [390, 1440])
  for (const theme of ['light', 'dark'] as const)
    test(`a new annotation retains its send geometry at ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.goto('/');
      await run(page, 'mount', {
        agents: [{ ...destination(), agentName: 'alpha' }],
        observeSend: true,
      });
      await page
        .frameLocator('#ask-page-fixture iframe')
        .locator('#selected')
        .evaluate((node) => {
          const range = node.ownerDocument.createRange();
          range.selectNodeContents(node);
          const selection = node.ownerDocument.getSelection()!;
          selection.removeAllRanges();
          selection.addRange(range);
          node.ownerDocument.dispatchEvent(new Event('selectionchange'));
        });
      await page.getByRole('button', { name: 'Annotate', exact: true }).click();
      const dialog = page.getByRole('dialog', { name: 'Annotate selection', exact: true });
      const input = dialog.getByRole('combobox', { name: 'Message', exact: true });
      await input.pressSequentially('@alpha Keep this turn in place.');
      await run(page, 'holdReply');
      await dialog.getByRole('button', { name: /^Send(?: · to .+)?$/ }).click();
      await expect(dialog.getByTestId('comment-echo')).toBeVisible();
      const before = (await dialog.getByTestId('comment-echo').boundingBox())!;
      const composer = (await dialog.locator('.annotation-compose').boundingBox())!;
      await page.screenshot({ path: capturePath(`send-annotation-pending-${width}-${theme}.png`) });
      // Closing while pending is covered by the annotation lifecycle cases (#5816).
      await run(page, 'releaseReply');
      await expect(dialog.getByTestId('comment-entry')).toBeVisible();
      await expect(dialog.getByTestId('comment-echo')).toHaveCount(0);
      await page.screenshot({ path: capturePath(`send-annotation-settled-${width}-${theme}.png`) });
      const after = (await dialog.getByTestId('comment-entry').boundingBox())!;
      expect(after.y).toBeCloseTo(before.y, 0);
      expect(after.height).toBeCloseTo(before.height, 0);
      expect((await dialog.locator('.annotation-compose').boundingBox())!.y).toBeCloseTo(
        composer.y,
        0,
      );
    });
