import { mkdirSync } from 'node:fs';
import { expect, test, type Page } from '@playwright/test';
import { text } from '../src/strings.js';
const fixture = '/test/ask-page-browser.tsx';
const captureDir = process.env.COLAB_TURN_CAPTURE_DIR ?? '/private/tmp/ux-1-1707-captures';
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
  const toggle = host.getByTestId(surface === 'chat' ? 'chat-toggle' : 'comments-toggle');
  if (!(await toggle.isVisible()))
    await host.getByRole('button', { name: 'More page actions' }).click();
  await toggle.click();
  if (surface === 'thread') await page.getByTestId('annotation-row').click();
}
for (const width of [1440, 390]) {
  for (const theme of ['light', 'dark']) {
    for (const surface of ['thread', 'chat'] as const) {
      test(`${surface} has one shared turn layout at ${width} ${theme} through waiting, held, replied and failed`, async ({
        page,
      }) => {
        await page.setViewportSize({ width, height: 900 });
        await mount(page, surface);
        await page.evaluate((theme) => {
          document.documentElement.dataset.theme = theme;
        }, theme);
        const user = page.locator('.conversation-turn[data-turn-role="user"]');
        const agent = page.locator('.conversation-turn[data-turn-role="agent"]');
        await expect(user).toHaveCount(1);
        await expect(user.locator('.comment-byline')).toContainText('You ·');
        await expect(user.locator('.conversation-body')).toHaveText(request);
        if (surface === 'thread')
          await expect(user.locator('.conversation-avatar svg.lucide-user')).toBeVisible();
        for (const state of ['waiting', 'held', 'replied', 'failed']) {
          await run(page, 'conversation', { surface, state });
          if (state === 'replied') {
            await expect(agent).toHaveCount(1);
            await expect(agent.getByTestId('ask-reply')).toHaveText(reply);
            await expect(agent.getByTestId('ask-reply-attribution')).toHaveText(
              `Atlas · ${text.conversationAgent} · 5m ago`,
            );
            await expect(agent.getByTestId('ask-reply-attribution')).not.toContainText('You');
            await expect(agent.locator('svg.lucide-bot')).toBeVisible();
            await expect(agent.locator('.conversation-body')).toHaveCSS(
              'border-left-width',
              surface === 'thread' ? '3px' : '0px',
            );
            await expect(page.getByTestId('ask-state')).toHaveCount(0);
            await expect(page.getByRole('button', { name: 'Re-check delivery' })).toHaveCount(0);
            const a = (await agent.boundingBox())!,
              u = (await user.boundingBox())!;
            if (surface === 'thread') {
              expect(Math.round(a.x)).toBe(Math.round(u.x));
              expect(Math.round(a.width)).toBe(Math.round(u.width));
            } else {
              expect(a.x).toBeLessThan(u.x);
              expect(a.x + a.width).toBeLessThan(u.x + u.width);
              await expect(user).not.toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
              await expect(agent).not.toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
            }
          } else {
            await expect(agent).toHaveCount(0);
            await expect(user.getByTestId('ask-state')).toContainText(
              state === 'failed' ? 'This send failed.' : state,
            );
            await expect(user.getByTestId('ask-state').locator('svg.lucide')).toBeVisible();
          }
          await expect(page.getByText('Reply from', { exact: false })).toHaveCount(0);
          await expect(
            page.locator('.conversation-turn script,.conversation-turn img'),
          ).toHaveCount(0);
          await expect(
            page.frameLocator('#ask-page-fixture iframe').locator('body'),
          ).not.toContainText(request);
          await expect(
            page.frameLocator('#ask-page-fixture iframe').locator('body'),
          ).not.toContainText(reply);
          await expect(user).toHaveCSS('border-radius', '0px');
          if (surface === 'thread') {
            await expect(user).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
            const bar = page.locator('.thread-bar');
            await expect(bar.getByRole('button', { name: text.threadResolve })).toHaveText(
              text.threadResolve,
            );
            await expect(bar.getByRole('button', { name: text.threadClose })).toHaveText(
              text.threadClose,
            );
            const actions = (await bar.locator('.thread-bar-actions').boundingBox())!,
              bounds = (await bar.boundingBox())!;
            expect(actions.x).toBeGreaterThanOrEqual(bounds.x);
            expect(actions.x + actions.width).toBeLessThanOrEqual(bounds.x + bounds.width + 1);
          }
          expect(await run(page, 'proof')).toEqual({ sends: [], actions: [] });
          mkdirSync(captureDir, { recursive: true });
          await page.screenshot({
            path: `${captureDir}/${surface}-${width}-${theme}-${state}.png`,
          });
        }
      });
    }
  }
}

test('thread delivery actions remain explicit, empty replies end the status, and visible action labels resolve/reopen/close', async ({
  page,
}) => {
  await mount(page, 'thread', 'uncertain');
  await page.getByRole('button', { name: 'Re-check delivery' }).click();
  await page.getByRole('button', { name: 'Abandon tracking' }).click();
  expect((await run(page, 'proof')).actions).toEqual([
    'recheck:00000000-0000-4000-8000-000000000043',
    'abandon:00000000-0000-4000-8000-000000000043',
  ]);
  await run(page, 'conversation', { surface: 'thread', state: 'empty' });
  await expect(page.getByTestId('ask-reply')).toHaveAttribute('data-empty', 'true');
  await expect(page.getByText('The agent returned an empty reply.')).toBeVisible();
  await expect(page.getByTestId('ask-state')).toHaveCount(0);
  const resolve = page.getByRole('button', { name: text.threadResolve, exact: true });
  await expect(resolve).toHaveAttribute('title', text.threadResolve);
  await expect(resolve).toHaveText(text.threadResolve);
  await expect(resolve.locator('svg.lucide-check')).toBeVisible();
  await resolve.click();
  await expect(page.locator('.thread-state')).toHaveText(text.threadResolved);
  await expect(page.locator('.thread-state svg.lucide-circle-check')).toBeVisible();
  const reopen = page.getByRole('button', { name: text.threadReopen, exact: true });
  await expect(reopen).toHaveAttribute('title', text.threadReopen);
  await expect(reopen).toHaveText(text.threadReopen);
  await expect(reopen.locator('svg.lucide-rotate-ccw')).toBeVisible();
  await reopen.click();
  await expect(page.locator('.thread-state')).toHaveText(text.threadOpen);
  await expect(page.locator('.thread-state svg.lucide-circle-dot')).toBeVisible();
  const close = page.getByRole('button', { name: text.threadClose, exact: true });
  await expect(close).toHaveAttribute('title', text.threadClose);
  await expect(close).toHaveText(text.threadClose);
  await close.click();
  await expect(page.getByTestId('comment-thread')).toHaveCount(0);
  expect((await run(page, 'proof')).sends).toEqual([]);
});
