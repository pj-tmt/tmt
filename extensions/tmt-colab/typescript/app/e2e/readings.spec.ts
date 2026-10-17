import { capturePath } from './captures.js';
import { expect, test, type Page } from '@playwright/test';
const fixture = '/test/ask-page-browser.tsx';
const run = (page: Page, method: string, argument?: unknown) =>
  page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
async function mount(page: Page, options = {}) {
  await page.goto('/');
  await run(page, 'mount', { statusAgents: true });
  await run(page, 'reading', options);
  const band = page.locator('[data-inline-reading]').first();
  await expect(band).not.toHaveAttribute('data-detached');
  return band;
}
test('reading bands record a person read once, keep content, and survive reload without notification retries', async ({
  page,
}) => {
  const band = await mount(page, { forged: true });
  await expect(band.getByText('Read', { exact: true })).toBeVisible();
  await expect(band).toHaveAttribute('data-reading-state', 'unread');
  const mark = band.getByRole('button', { name: 'Mark read', exact: true });
  await mark.evaluate((node) => (node as HTMLButtonElement).click());
  expect((await run(page, 'proof')).actions).toEqual([]);
  await mark.click();
  await expect(band).toHaveAttribute('data-reading-state', 'read');
  await expect(band).toContainText('unconfirmed');
  const threads = await run(page, 'discussionProof');
  expect(threads).toHaveLength(1);
  expect(threads[0].comments).toHaveLength(1);
  expect(threads[0].comments[0].body).toContain('Reading: ');
  expect((await run(page, 'proof')).actions).toEqual(['status:resolve', 'notify:Agent 1']);
  const content = page.frameLocator('#ask-page-fixture iframe').locator('tmt-read').first();
  await expect(content.getByText(/Content of reading 1/)).toBeVisible();
  await expect(band.getByRole('button', { name: 'Mark read', exact: true })).toHaveCount(0);
  await run(page, 'saveQuestionPage');
  await page.reload();
  await run(page, 'mount', { statusAgents: true, restoreQuestionPage: true });
  const restored = page.locator('[data-inline-reading]').first();
  await expect(restored).toHaveAttribute('data-reading-state', 'read');
  expect((await run(page, 'proof')).actions).toEqual([]);
});

test('only admitted accepted delivery says notified; refusals keep the read and never retry', async ({
  page,
}) => {
  const band = await mount(page);
  await band.getByRole('button', { name: 'Mark read', exact: true }).click();
  await expect(band).toHaveAttribute('data-reading-state', 'read');
  await run(page, 'readingDelivery', 'accepted');
  await expect(band).toContainText('Agent 1 notified');
  for (const state of ['held', 'refused', 'unavailable']) {
    await run(page, 'readingDelivery', state);
    await expect(band).toContainText(`Read, but Agent 1 was not notified: ${state}.`);
    await expect(band).toHaveAttribute('data-reading-state', 'read');
    await expect(band.getByRole('button', { name: 'Mark read', exact: true })).toHaveCount(0);
  }
  await run(page, 'readingDelivery', 'uncertain');
  await expect(band).toContainText('notification to Agent 1 is unconfirmed');
  expect((await run(page, 'proof')).actions).toEqual(['status:resolve', 'notify:Agent 1']);
});
test('Next is cosmetic and an archived page has no reading actions', async ({ page }) => {
  const band = await mount(page);
  await band.getByRole('button', { name: 'Next ↓', exact: true }).click();
  await expect
    .poll(async () =>
      page
        .frameLocator('#ask-page-fixture iframe')
        .locator('h2')
        .nth(1)
        .evaluate((node) => {
          const highlight = (
            CSS as unknown as { highlights: Map<string, { size: number }> }
          ).highlights.get('colab-reading');
          return { text: node.textContent, size: highlight?.size };
        }),
    )
    .toEqual({ text: 'Decision 2', size: 1 });
  expect((await run(page, 'proof')).actions).toEqual([]);
  expect(await run(page, 'discussionProof')).toHaveLength(0);
  await run(page, 'archive');
  await expect(page.locator('[data-inline-reading] button')).toHaveCount(0);
});
test('a failed Resolve keeps one recorded comment and an explicit retry completes it', async ({
  page,
}) => {
  const band = await mount(page);
  await run(page, 'failStatusOnce');
  await band.getByRole('button', { name: 'Mark read', exact: true }).click();
  await expect(band.getByRole('alert')).toContainText('comment is saved');
  expect((await run(page, 'discussionProof'))[0].comments).toHaveLength(1);
  await band.getByRole('button', { name: 'Mark read', exact: true }).click();
  await expect(band).toHaveAttribute('data-reading-state', 'read');
  expect((await run(page, 'discussionProof'))[0].comments).toHaveLength(1);
  expect((await run(page, 'proof')).actions).toEqual(['status:resolve', 'notify:Agent 1']);
});
for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`reading bands in context ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
      const band = await mount(page, { count: 3, darkPage: theme === 'dark' });
      await expect(band).toBeVisible();
      if (process.env.COLAB_CAPTURE_DIR)
        await page.screenshot({
          path: capturePath(`reading-open-${width}-${theme}.png`),
          fullPage: true,
        });
      await band.getByRole('button', { name: 'Mark read', exact: true }).click();
      await run(page, 'readingDelivery', 'accepted');
      await expect(band).toContainText('Agent 1 notified');
      if (process.env.COLAB_CAPTURE_DIR)
        await page.screenshot({
          path: capturePath(`reading-done-${width}-${theme}.png`),
          fullPage: true,
        });
      await run(page, 'readingDelivery', 'unavailable');
      await expect(band).toContainText('was not notified: unavailable');
      if (process.env.COLAB_CAPTURE_DIR)
        await page.screenshot({
          path: capturePath(`reading-warning-${width}-${theme}.png`),
          fullPage: true,
        });
    });
  }
