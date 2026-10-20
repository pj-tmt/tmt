import { expect, test, type Page } from '@playwright/test';
import { pageAction } from '../test/page-actions.js';
import { capturePath } from './captures.js';

/**
 * A page shows its document before its threads and asks are verified. Until then nothing reads
 * as verified: no comment count, no "Nothing waits", no bands or composers, only a quiet place
 * where the discussion will be. Verification then fills everything in without moving the document.
 */
const fixture = '/test/ask-page-browser.tsx';
const run = (page: Page, method: string, argument?: unknown) =>
  page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
const frame = (page: Page) => page.frameLocator('#ask-page-fixture iframe');

async function open(page: Page, width = 1440) {
  await page.setViewportSize({ width, height: 900 });
  await page.goto('/');
  await run(page, 'mount', { statusAgents: true, discussionPending: true });
  // A thread the viewer will see once it is verified: the fixture holds it back until then.
  await run(page, 'conversation', { surface: 'thread', state: 'replied' });
  await expect(frame(page).locator('#selected')).toBeVisible();
}
const capture = async (page: Page, name: string, width: number) => {
  if (process.env.COLAB_CAPTURE_DIR)
    await page.screenshot({ path: capturePath(`progressive-${name}-${width}.png`) });
};

for (const width of [1440, 390])
  test(`the document is shown at ${width}px with no count, no Needs you and no bands until verified`, async ({
    page,
  }) => {
    await open(page, width);
    // Nothing unverified reads as verified: no "(0)", no "✓ Nothing waits".
    await expect(page.getByRole('button', { name: 'Discussion', exact: true })).toBeVisible();
    const needsYou = page.locator(
      '#ask-page-fixture [aria-label*="Needs you"], #ask-page-fixture [aria-label*="Nothing waits"]',
    );
    await expect(needsYou).toHaveCount(0);
    await expect(page.locator('#ask-page-fixture').getByText('Nothing waits')).toHaveCount(0);
    await expect(
      page.locator('[data-inline-reading], [data-inline-review], .proposal-inline'),
    ).toHaveCount(0);
    await expect(frame(page).locator('[data-colab-thread]')).toHaveCount(0);
    await capture(page, 'pending', width);
    await run(page, 'verifyDiscussion');
    await expect(page.getByRole('button', { name: /^Discussion \(1\)/ })).toBeVisible();
    await expect(frame(page).locator('[data-colab-thread]')).toHaveCount(1);
    await expect(needsYou).toHaveCount(1);
    await capture(page, 'verified', width);
  });

for (const drawer of ['Comments', 'Chat'] as const)
  test(`the ${drawer} drawer says comments are loading, offers no composer, then fills in`, async ({
    page,
  }) => {
    await open(page);
    await (await pageAction(page, drawer)).click();
    const panel = page.locator('.tmt-ui-drawer[open]');
    await expect(panel.getByRole('status')).toHaveText('Loading comments…');
    await expect(panel.getByRole('textbox').or(panel.getByRole('combobox'))).toHaveCount(0);
    await expect(panel.getByTestId('comment-thread')).toHaveCount(0);
    await run(page, 'verifyDiscussion');
    await expect(panel.getByText('Loading comments…')).toHaveCount(0);
    await expect(
      drawer === 'Chat'
        ? panel.getByRole('combobox', { name: 'Message' })
        : panel.getByRole('button', { name: '+ Comment on page' }),
    ).toBeVisible();
  });

test('the document does not move when verification arrives', async ({ page }) => {
  await open(page);
  const before = await page.locator('#ask-page-fixture iframe').boundingBox();
  await run(page, 'verifyDiscussion');
  await expect(frame(page).locator('[data-colab-thread]')).toHaveCount(1);
  expect(await page.locator('#ask-page-fixture iframe').boundingBox()).toEqual(before);
});
