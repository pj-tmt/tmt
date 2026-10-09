import { mkdirSync } from 'node:fs';
import { expect, test, type Locator, type Page } from '@playwright/test';
import { text } from '../src/strings.js';
import { annotationInput } from '../acceptance/harness/ask.js';

const fixture = '/test/ask-page-browser.tsx';
const captureDir = process.env.COLAB_UPDATE_CAPTURE_DIR ?? '/tmp/colab-live-update-captures';
const source = `<style>body{margin:0;padding:24px;font:18px/1.5 sans-serif}.before{height:1400px}.after{height:1800px}</style>
<div class="before"></div><p id="selected">Exact selected text</p><div class="after"></div>`;
const revised = source.replace('Exact selected text', 'New source without the captured quote');

async function run(page: Page, method: string, argument?: string) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
async function change(page: Page, html: string) {
  const frame = page.locator('#ask-page-fixture iframe');
  const before = await frame.getAttribute('data-render-id');
  await run(page, 'change', html);
  await expect(frame).not.toHaveAttribute('data-render-id', before!);
  await expect(page.locator('#ask-page-fixture .status')).toContainText('Live preview');
  await expect.poll(async () => (await frame.boundingBox())?.height ?? 0).toBeGreaterThan(3200);
}
async function changeWhileTyping(
  page: Page,
  html: string,
  input: Locator,
  addition: string,
  surface: string,
) {
  const value = await input.innerText();
  const caret = value.length - 1;
  await input.focus();
  await input.evaluate((node, caret) => {
    const walker = node.ownerDocument.createTreeWalker(node, NodeFilter.SHOW_TEXT);
    let remaining = caret;
    for (let text = walker.nextNode(); text; text = walker.nextNode()) {
      if (remaining <= text.textContent!.length) {
        const range = node.ownerDocument.createRange();
        range.setStart(text, remaining);
        range.collapse(true);
        const selection = node.ownerDocument.getSelection()!;
        selection.removeAllRanges();
        selection.addRange(range);
        return;
      }
      remaining -= text.textContent!.length;
    }
    throw new Error('Caret is outside the retained message');
  }, caret);
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.route('**/renderer.html', async (route) => {
    await gate;
    await route.continue();
  });
  try {
    const update = change(page, html);
    await expect(page.locator('#ask-page-fixture .status')).toContainText(text.loading);
    await expect(input).toHaveAttribute('contenteditable', 'true');
    await expect(input).toBeFocused();
    expect(
      await input.evaluate((node) => {
        const selection = node.ownerDocument.getSelection()!;
        const before = node.ownerDocument.createRange();
        before.selectNodeContents(node);
        before.setEnd(selection.focusNode!, selection.focusOffset);
        return before.toString().length;
      }),
    ).toBe(caret);
    // Unicode fixture text exercises the caret path used by IME-produced text.
    await page.keyboard.insertText(addition);
    const expected = value.slice(0, caret) + addition + value.slice(caret);
    await expect(input).toHaveText(expected, { useInnerText: true });
    const width = page.viewportSize()!.width;
    const theme = await page.evaluate(() => document.documentElement.dataset.theme);
    await page.screenshot({ path: `${captureDir}/${surface}-${width}-${theme}-loading.png` });
    release();
    await update;
    await expect(input).toBeFocused();
    expect(
      await input.evaluate((node) => {
        const selection = node.ownerDocument.getSelection()!;
        const before = node.ownerDocument.createRange();
        before.selectNodeContents(node);
        before.setEnd(selection.focusNode!, selection.focusOffset);
        return before.toString().length;
      }),
    ).toBe(caret + addition.length);
    return expected;
  } finally {
    release();
    await page.unroute('**/renderer.html');
  }
}
async function select(page: Page) {
  await page
    .frameLocator('#ask-page-fixture iframe')
    .locator('#selected')
    .evaluate((node) => {
      const range = node.ownerDocument.createRange();
      range.selectNodeContents(node);
      const selection = node.ownerDocument.defaultView!.getSelection()!;
      selection.removeAllRanges();
      selection.addRange(range);
    });
  await expect(page.getByTestId('selection-ask')).toBeVisible();
  await page.getByTestId('selection-ask').click();
}
async function panel(page: Page, name: 'chat' | 'comments') {
  const host = page.locator('#ask-page-fixture');
  const toggle = host.getByTestId(`${name}-toggle`);
  if (!(await toggle.isVisible()))
    await host.getByRole('button', { name: 'More page actions' }).click();
  await toggle.click();
}

test('clearing an annotation immediately before Escape does not restore the removed text', async ({
  page,
}) => {
  await page.goto('/');
  await run(page, 'mount');
  await expect(page.locator('#ask-page-fixture .status')).toContainText('Live preview');
  await change(page, source);
  await page.evaluate(() => window.scrollTo(0, 1100));
  await select(page);
  const composer = page.getByRole('dialog', { name: 'Annotate selection' });
  const input = composer.getByRole('combobox', { name: 'Message', exact: true });
  await expect(input).toBeFocused();
  await page.keyboard.insertText('@');
  await expect(input).toHaveAttribute('aria-expanded', 'true');
  await input.press('Escape');
  await input.press('ControlOrMeta+A');
  await input.press('Backspace');
  await input.press('Escape');
  await expect(composer).toHaveCount(0);
  await select(page);
  await expect(input).toHaveText('', { useInnerText: true });
  await expect(input).toBeFocused();
  await page.keyboard.insertText('Typed immediately after reopening.');
  await expect(input).toHaveText('Typed immediately after reopening.', { useInnerText: true });
  expect((await run(page, 'proof')).sends).toHaveLength(0);
});

for (const width of [1440, 390]) {
  for (const theme of ['light', 'dark']) {
    test(`source revisions preserve frozen annotation, thread and Chat drafts and scroll at ${width} ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto('/');
      await run(page, 'mount');
      await expect(page.locator('#ask-page-fixture .status')).toContainText('Live preview');
      await change(page, source);
      await page.evaluate((theme) => {
        document.documentElement.dataset.theme = theme;
        window.scrollTo(0, 1100);
      }, theme);
      await select(page);
      const composer = page.getByRole('dialog', { name: 'Annotate selection' });
      const input = await annotationInput(composer, 'Agent 1');
      await expect(input).toHaveText('', { useInnerText: true });
      mkdirSync(captureDir, { recursive: true });
      await page.screenshot({ path: `${captureDir}/annotation-${width}-${theme}-empty.png` });
      const draft = '@Agent 1 Keep this exact unsent draft.';
      await input.fill(draft);
      const position = (await composer.boundingBox())!;
      const scroll = await page.evaluate(() => window.scrollY);
      // Keeping the value alone is insufficient: the real input must survive loading.
      await input.evaluate((node) => Object.assign(window, { retainedComposer: node }));
      await page.screenshot({ path: `${captureDir}/annotation-${width}-${theme}-before.png` });
      const continuedDraft = await changeWhileTyping(page, revised, input, '新', 'annotation');
      await expect(input).toHaveText(continuedDraft, { useInnerText: true });
      expect(
        await input.evaluate(
          (node) => (window as unknown as { retainedComposer: Element }).retainedComposer === node,
        ),
      ).toBe(true);
      await expect(composer.locator('blockquote')).toHaveText('Exact selected text');
      await expect(composer.getByText(text.commentQuoteChanged)).toBeVisible();
      await expect(
        page.frameLocator('#ask-page-fixture iframe').locator('[data-colab-thread]'),
      ).toHaveCount(0);
      const after = (await composer.boundingBox())!;
      console.log(
        JSON.stringify({
          position,
          after,
          scroll,
          updatedScroll: await page.evaluate(() => window.scrollY),
          frame: await page.locator('#ask-page-fixture iframe').boundingBox(),
        }),
      );
      await page.screenshot({ path: `${captureDir}/annotation-${width}-${theme}-updated.png` });
      expect(Math.abs(after.x - position.x)).toBeLessThan(2);
      expect(Math.abs(after.y - position.y)).toBeLessThan(2);
      expect(Math.abs((await page.evaluate(() => window.scrollY)) - scroll)).toBeLessThan(2);
      expect((await run(page, 'proof')).sends).toEqual([]);
      await input.press('Enter');
      await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
      const sent = (await run(page, 'proof')).sends[0];
      expect(sent.message).toContain('Exact selected text');
      expect(sent.message).toContain(continuedDraft);
      expect(sent.message).not.toContain('New source without the captured quote');
      expect((await run(page, 'discussionProof'))[0].anchor.exact).toBe('Exact selected text');

      const thread = page.getByTestId('comment-thread');
      await expect(thread).toHaveAttribute('data-anchor', 'detached');
      await expect(thread.getByText(text.commentQuoteChanged)).toBeVisible();
      const reply = thread.getByRole('combobox', { name: 'Message' });
      const threadDraft = '@Agent 1 A thread reply still being written.';
      await reply.fill(threadDraft);
      const threadScroll = await page.evaluate(() => window.scrollY);
      const continuedThreadDraft = await changeWhileTyping(page, source, reply, '文', 'thread');
      await expect(reply).toHaveText(continuedThreadDraft, { useInnerText: true });
      await expect(thread).toHaveAttribute('data-anchor', 'attached');
      await expect(thread.getByText(text.commentQuoteChanged)).toHaveCount(0);
      await expect(
        page.frameLocator('#ask-page-fixture iframe').locator('[data-colab-thread]'),
      ).toHaveCount(1);
      expect(Math.abs((await page.evaluate(() => window.scrollY)) - threadScroll)).toBeLessThan(2);
      await thread.getByRole('button', { name: text.threadClose, exact: true }).click();
      await panel(page, 'chat');
      const chat = await annotationInput(page.getByTestId('chat-panel'), 'Agent 1');
      const chatDraft = '@Agent 1 A Chat draft during another edit.';
      await chat.fill(chatDraft);
      const chatScroll = await page.evaluate(() => window.scrollY);
      const continuedChatDraft = await changeWhileTyping(page, revised, chat, '續', 'chat');
      await expect(chat).toHaveText(continuedChatDraft, { useInnerText: true });
      expect(Math.abs((await page.evaluate(() => window.scrollY)) - chatScroll)).toBeLessThan(2);
      await page.screenshot({ path: `${captureDir}/chat-${width}-${theme}-updated.png` });
      await page.getByRole('button', { name: 'Close Chat', exact: true }).click();
      await panel(page, 'comments');
      await page.getByTestId('annotation-row').filter({ hasText: 'Exact selected text' }).click();
      await expect(reply).toHaveText(continuedThreadDraft, { useInnerText: true });
      await expect(thread).toHaveAttribute('data-anchor', 'detached');
      await expect(thread.getByText(text.commentQuoteChanged)).toBeVisible();
      await page.screenshot({ path: `${captureDir}/thread-${width}-${theme}-updated.png` });
      expect((await run(page, 'proof')).sends).toHaveLength(1);

      // A different page is the reset boundary, including drafts kept after close.
      await page.getByRole('button', { name: 'Close Comments', exact: true }).click();
      await change(page, source);
      await select(page);
      await input.fill('@Agent 1 This belongs to the previous page only.');
      await page.getByRole('button', { name: 'Close annotation', exact: true }).click();
      await page.evaluate(() => (location.hash = '/pages/notes'));
      await expect(
        page.frameLocator('#ask-page-fixture iframe').locator('#other-page'),
      ).toBeVisible();
      await expect(composer).toHaveCount(0);
      await expect(page.getByTestId('chat-panel')).toHaveCount(0);
      await page.evaluate(() => (location.hash = '/pages/welcome'));
      await expect(page.frameLocator('#ask-page-fixture iframe').locator('#selected')).toHaveText(
        'Exact selected text',
      );
      await expect(page.locator('#ask-page-fixture .status')).toContainText('Live preview');
      await expect
        .poll(
          async () => (await page.locator('#ask-page-fixture iframe').boundingBox())?.height ?? 0,
        )
        .toBeGreaterThan(3200);
      await page.evaluate(() => window.scrollTo(0, 1100));
      await select(page);
      await expect(input).toHaveText('', { useInnerText: true });
      await expect(composer.getByText('Draft kept', { exact: true })).toHaveCount(0);
      expect((await run(page, 'proof')).sends).toHaveLength(1);
      await input.fill('Retain this draft when the connection fails.');
      await run(page, 'block');
      await expect(input).toHaveText('Retain this draft when the connection fails.', {
        useInnerText: true,
      });
      await expect(input).toHaveAttribute('contenteditable', 'false');
      await expect(composer).toBeVisible();
      await expect(composer.locator('blockquote')).toHaveText('Exact selected text');
      await expect(page.locator('#ask-page-fixture iframe')).toHaveCount(0);
      const notice = page.getByRole('alert');
      await expect(notice).toContainText('Page unavailable');
      await expect(notice).not.toContainText(text.limit);
      await expect
        .poll(async () => {
          const card = await notice.boundingBox();
          const draft = await composer.boundingBox();
          return card && draft ? draft.y - (card.y + card.height) : -1;
        })
        .toBeGreaterThan(0);
      await page.screenshot({ path: `${captureDir}/annotation-${width}-${theme}-failure.png` });
      expect((await run(page, 'proof')).sends).toHaveLength(1);
    });
  }
}

for (const width of [1440, 390]) {
  for (const theme of ['light', 'dark']) {
    test(`anchored window keeps one composer, exact association and collapsed draft at ${width} ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto('/');
      await run(page, 'mount');
      await expect(page.locator('#ask-page-fixture .status')).toContainText('Live preview');
      await change(page, source);
      await page.evaluate((theme) => {
        document.documentElement.dataset.theme = theme;
        window.scrollTo(0, 1100);
      }, theme);
      await select(page);
      const dialog = page.getByRole('dialog', { name: 'Annotate selection' });
      const input = dialog.getByRole('combobox', { name: 'Message', exact: true });
      await expect(input).toBeFocused();
      await input.evaluate((node) => Object.assign(window, { anchoredInput: node }));
      const scroll = await page.evaluate(() => window.scrollY);
      const before = (await dialog.boundingBox())!;
      mkdirSync(captureDir, { recursive: true });
      await page.screenshot({ path: `${captureDir}/window-${width}-${theme}-empty.png` });
      await input.fill('First plain turn without an agent.');
      await input.press('Enter');
      const thread = dialog.getByTestId('comment-thread');
      await expect(thread.getByTestId('comment-entry')).toHaveCount(1);
      await expect(input).toHaveText('', { useInnerText: true });
      expect(
        await input.evaluate(
          (node) => (window as unknown as { anchoredInput: Element }).anchoredInput === node,
        ),
      ).toBe(true);
      await expect(page.locator('.page-drawer[open]')).toHaveCount(0);
      expect((await run(page, 'proof')).sends).toHaveLength(0);
      expect(Math.abs((await dialog.boundingBox())!.x - before.x)).toBeLessThan(2);
      expect(Math.abs((await dialog.boundingBox())!.y - before.y)).toBeLessThan(2);
      expect(Math.abs((await page.evaluate(() => window.scrollY)) - scroll)).toBeLessThan(2);
      await annotationInput(dialog, 'Agent 1');
      await input.fill('@Agent 1 Second turn from the same window.');
      await input.press('Enter');
      await expect(thread.getByTestId('comment-entry')).toHaveCount(2);
      await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
      await expect(
        thread.getByText('@Agent 1 Second turn from the same window.', { exact: true }),
      ).toBeInViewport({ ratio: 1 });
      const threadId = await thread.getAttribute('data-thread-id');
      const editorBox = (await input.boundingBox())!;
      const labelBox = (await dialog.locator('.annotation-reply-label').boundingBox())!;
      expect(editorBox.y - labelBox.y - labelBox.height).toBeGreaterThanOrEqual(8);
      await page.screenshot({ path: `${captureDir}/window-${width}-${theme}-sent.png` });
      // Row actions must fit the message area rather than flip under its stationary header.
      const latestTurn = thread.getByTestId('comment-entry').last();
      await latestTurn.hover();
      await latestTurn.getByRole('button', { name: 'Message actions', exact: true }).click();
      const editAction = latestTurn.getByRole('menuitem', { name: 'Edit', exact: true });
      await expect(editAction).toBeInViewport({ ratio: 1 });
      const menuBox = (await latestTurn.getByRole('menu').boundingBox())!;
      const messageBox = (await thread.locator('.thread-messages').boundingBox())!;
      expect(menuBox.y).toBeGreaterThanOrEqual(messageBox.y);
      expect(menuBox.y + menuBox.height).toBeLessThanOrEqual(messageBox.y + messageBox.height);
      await editAction.click();
      await expect(
        latestTurn.getByRole('combobox', { name: 'Edit comment', exact: true }),
      ).toBeFocused();
      await latestTurn.getByRole('button', { name: 'Cancel', exact: true }).click();
      await page.evaluate(async (fixture) => (await import(fixture)).windowReply(24), fixture);
      await expect(thread.getByTestId('ask-reply')).toHaveText('Exact associated agent reply.');
      const messages = thread.locator('.thread-messages');
      await expect(thread.getByTestId('ask-reply')).toBeInViewport({ ratio: 1 });
      expect(await messages.evaluate((node) => node.scrollHeight > node.clientHeight)).toBe(true);
      const header = thread.locator('.thread-bar');
      const headerBefore = (await header.boundingBox())!;
      const inputBefore = (await input.boundingBox())!;
      await messages.evaluate((node) => {
        node.scrollTop = 0;
      });
      expect((await header.boundingBox())!.y).toBe(headerBefore.y);
      expect((await input.boundingBox())!.y).toBe(inputBefore.y);
      await run(page, 'unrelatedWindowUpdate');
      await expect(
        page.getByRole('heading', { name: 'Unrelated live title', exact: true }),
      ).toBeVisible();
      expect(await messages.evaluate((node) => node.scrollTop)).toBe(0);
      await page.screenshot({ path: `${captureDir}/window-${width}-${theme}-history.png` });
      await run(page, 'updateWindowReply');
      await expect(thread.getByTestId('ask-reply')).toHaveText('Updated associated agent reply.');
      await expect(thread.getByTestId('ask-reply')).toBeInViewport({ ratio: 1 });
      expect(await messages.evaluate((node) => node.clientHeight)).toBeGreaterThanOrEqual(240);
      await page.screenshot({ path: `${captureDir}/window-${width}-${theme}-reply.png` });
      await input.fill('@Agent 1 Exact unsent draft retained on collapse.');
      await page.getByRole('heading', { name: 'Unrelated live title', exact: true }).click();
      await expect(dialog).toHaveCount(0);
      // Reopening via the associated marker restores the thread, quote and bound mention.
      const marker = page
        .frameLocator('#ask-page-fixture iframe')
        .locator(`[data-colab-thread$="${threadId}"]`);
      await marker.click();
      await expect(thread).toHaveAttribute('data-thread-id', threadId!);
      await expect(input).toHaveText('@Agent 1 Exact unsent draft retained on collapse.', {
        useInnerText: true,
      });
      await expect(dialog.locator('.annotation-status-row')).toContainText('Asks @Agent 1.');
      await expect(thread.getByTestId('ask-reply')).toBeInViewport({ ratio: 1 });
      await expect(thread.locator('blockquote')).toHaveText('Exact selected text');
      await expect(page.locator('.page-drawer[open]')).toHaveCount(0);
      const box = (await dialog.boundingBox())!;
      expect(box.x).toBeGreaterThanOrEqual(8);
      expect(box.x + box.width).toBeLessThanOrEqual(width - 8);
      expect(box.y + box.height).toBeLessThanOrEqual(892);
      const frameText = await page
        .frameLocator('#ask-page-fixture iframe')
        .locator('body')
        .innerText();
      expect(frameText).not.toContain('First plain turn');
      expect(frameText).not.toContain('Exact associated agent reply');
      expect(frameText).not.toContain('Exact unsent draft');
      await page.screenshot({ path: `${captureDir}/window-${width}-${theme}-reopened.png` });
      const resolve = thread.getByRole('button', { name: text.threadResolve, exact: true });
      await resolve.focus();
      await resolve.press('Tab');
      const close = thread.getByRole('button', { name: text.threadClose, exact: true });
      await expect(close).toBeFocused();
      await expect(close.locator('..').locator('.tmt-ui-icon-action-tooltip')).toBeVisible();
      await page.screenshot({ path: `${captureDir}/window-${width}-${theme}-header-focus.png` });
      await resolve.click();
      await expect(dialog).toHaveCount(0);
      await panel(page, 'comments');
      await page.getByTestId('annotation-row').click();
      const details = page.getByTestId('comment-thread');
      await expect(details).toContainText(text.threadResolved);
      await details.getByRole('button', { name: text.threadReopen, exact: true }).click();
      await expect(details).toContainText(text.threadOpen);
      expect((await run(page, 'proof')).sends).toHaveLength(1);
    });
  }
}
