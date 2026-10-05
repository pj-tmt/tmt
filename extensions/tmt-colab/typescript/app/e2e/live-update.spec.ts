import { mkdirSync } from 'node:fs';
import { expect, test, type Page } from '@playwright/test';

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
      const input = composer.getByRole('combobox', { name: 'Message to agent' });
      await expect(input).toHaveValue('@Agent 1 ');
      const draft = '@Agent 1 Keep this exact unsent draft.';
      await input.fill(draft);
      const position = (await composer.boundingBox())!;
      const scroll = await page.evaluate(() => window.scrollY);
      // Keeping the value alone is insufficient: the real input must survive loading.
      await input.evaluate((node) => Object.assign(window, { retainedComposer: node }));
      mkdirSync(captureDir, { recursive: true });
      await page.screenshot({ path: `${captureDir}/annotation-${width}-${theme}-before.png` });
      await change(page, revised);
      await expect(input).toHaveValue(draft);
      expect(
        await input.evaluate(
          (node) => (window as unknown as { retainedComposer: Element }).retainedComposer === node,
        ),
      ).toBe(true);
      await expect(composer.locator('blockquote')).toHaveText('Exact selected text');
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
      expect(sent.message).toContain('Keep this exact unsent draft.');
      expect(sent.message).not.toContain('New source without the captured quote');
      expect((await run(page, 'discussionProof'))[0].anchor.exact).toBe('Exact selected text');

      const thread = page.getByTestId('comment-thread');
      await expect(thread).toHaveAttribute('data-anchor', 'detached');
      const reply = thread.getByRole('combobox', { name: 'Message to agent' });
      const threadDraft = '@Agent 1 A thread reply still being written.';
      await reply.fill(threadDraft);
      const threadScroll = await page.evaluate(() => window.scrollY);
      await change(page, source);
      await expect(reply).toHaveValue(threadDraft);
      await expect(thread).toHaveAttribute('data-anchor', 'attached');
      await expect(
        page.frameLocator('#ask-page-fixture iframe').locator('[data-colab-thread]'),
      ).toHaveCount(1);
      expect(Math.abs((await page.evaluate(() => window.scrollY)) - threadScroll)).toBeLessThan(2);
      await page.screenshot({ path: `${captureDir}/thread-${width}-${theme}-updated.png` });
      await page.getByRole('button', { name: 'Close Comments', exact: true }).click();
      await panel(page, 'chat');
      const chat = page
        .getByTestId('chat-panel')
        .getByRole('combobox', { name: 'Message to agent' });
      const chatDraft = '@Agent 1 A Chat draft during another edit.';
      await chat.fill(chatDraft);
      const chatScroll = await page.evaluate(() => window.scrollY);
      await change(page, revised);
      await expect(chat).toHaveValue(chatDraft);
      expect(Math.abs((await page.evaluate(() => window.scrollY)) - chatScroll)).toBeLessThan(2);
      await page.screenshot({ path: `${captureDir}/chat-${width}-${theme}-updated.png` });
      await page.getByRole('button', { name: 'Close Chat', exact: true }).click();
      await panel(page, 'comments');
      await expect(reply).toHaveValue(threadDraft);
      await expect(thread).toHaveAttribute('data-anchor', 'detached');
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
      await expect(input).toHaveValue('@Agent 1 ');
      await expect(composer.getByText('Draft kept', { exact: true })).toHaveCount(0);
      expect((await run(page, 'proof')).sends).toHaveLength(1);
    });
  }
}
