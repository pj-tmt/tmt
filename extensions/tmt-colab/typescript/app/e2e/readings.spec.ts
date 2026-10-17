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
/** The frame's real spacer acknowledges the measured parent band, including a disclosure. */
async function placed(page: Page, band: import('@playwright/test').Locator) {
  const id = await band.getAttribute('data-inline-reading');
  const spacer = page
    .frameLocator('#ask-page-fixture iframe')
    .locator(`tmt-read[data-id="${id}"] > [data-colab-proposal-slot]`);
  await expect
    .poll(async () => {
      const [height, box] = await Promise.all([
        spacer.evaluate((node) => parseFloat((node as HTMLElement).style.minHeight)),
        band.boundingBox(),
      ]);
      return height - Math.ceil(box!.height);
    })
    .toBe(0);
}
test('reading bands record a person read once, keep content, and survive reload without notification retries', async ({
  page,
}) => {
  const band = await mount(page, { forged: true });
  await expect(band.getByText('Please read', { exact: true })).toBeVisible();
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
  await run(page, 'renameReadingRequester');
  await expect(page.locator('[data-inline-reading]').nth(1)).toContainText('Agent 2');
  await expect(band).toContainText('Agent 1 notified');
  await expect(band).not.toContainText('Agent 2 notified');
  await expect(band.getByRole('button', { name: 'Why', exact: true })).toHaveCount(0);
  for (const [state, reason] of [
    ['held', 'The notice is waiting for approval on this computer.'],
    ['refused', "The agent's side refused the notice."],
    ['unavailable', 'The agent could not be reached.'],
  ]) {
    await run(page, 'readingDelivery', state);
    await expect(band).toContainText('Agent 1 not notified');
    const why = band.getByRole('button', { name: 'Why', exact: true });
    const detail = band.locator('.reading-reason');
    await expect(why).toHaveAttribute('aria-expanded', 'false');
    await expect(detail).toBeHidden();
    await expect(why).toHaveAttribute('aria-controls', (await detail.getAttribute('id'))!);
    await why.click();
    await expect(why).toHaveAttribute('aria-expanded', 'true');
    await expect(detail).toHaveText(reason);
    await expect(detail).toBeVisible();
    await why.press('Enter');
    await expect(why).toHaveAttribute('aria-expanded', 'false');
    await expect(detail).toBeHidden();
    await expect(band).toHaveAttribute('data-reading-state', 'read');
    await expect(band.getByRole('button', { name: 'Mark read', exact: true })).toHaveCount(0);
  }
  await run(page, 'readingDelivery', 'uncertain');
  await expect(band).toContainText('Agent 1 notification unconfirmed');
  await band.getByRole('button', { name: 'Why', exact: true }).press('Space');
  await expect(band.locator('.reading-reason')).toHaveText(
    'TMT could not confirm the notice arrived. Check with the agent before sending again.',
  );
  await expect(band.locator('.reading-reason')).toBeVisible();
  await run(page, 'archive');
  await expect(band.getByRole('button')).toHaveCount(0);
  await expect(band.locator('.reading-reason')).toBeVisible();
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
      await placed(page, band);
      if (process.env.COLAB_CAPTURE_DIR)
        await page.screenshot({
          path: capturePath(`reading-open-${width}-${theme}.png`),
          fullPage: true,
        });
      await band.getByRole('button', { name: 'Mark read', exact: true }).click();
      await run(page, 'readingDelivery', 'accepted');
      await expect(band).toContainText('Agent 1 notified');
      await placed(page, band);
      if (process.env.COLAB_CAPTURE_DIR)
        await page.screenshot({
          path: capturePath(`reading-done-${width}-${theme}.png`),
          fullPage: true,
        });
      await run(page, 'readingDelivery', 'unavailable');
      await expect(band).toContainText('Agent 1 not notified');
      await placed(page, band);
      if (process.env.COLAB_CAPTURE_DIR)
        await page.screenshot({
          path: capturePath(`reading-warning-${width}-${theme}.png`),
          fullPage: true,
        });
      await band.getByRole('button', { name: 'Why', exact: true }).click();
      await expect(band.locator('.reading-reason')).toBeVisible();
      await placed(page, band);
      if (process.env.COLAB_CAPTURE_DIR)
        await page.screenshot({
          path: capturePath(`reading-why-${width}-${theme}.png`),
          fullPage: true,
        });
    });
  }

test('the reading cue stays between its title and passage in unread and done states', async ({
  page,
}) => {
  const band = await mount(page);
  const content = page.frameLocator('#ask-page-fixture iframe').locator('tmt-read').first();
  const beforePassage = async () => {
    const [heading, cue, passage] = await Promise.all([
      content.locator('[data-title]').boundingBox(),
      band.boundingBox(),
      content.locator('p').boundingBox(),
    ]);
    return (
      !!heading &&
      !!cue &&
      !!passage &&
      cue.y >= heading.y + heading.height - 1 &&
      cue.y + cue.height <= passage.y + 1
    );
  };
  await expect.poll(beforePassage).toBe(true);
  await band.getByRole('button', { name: 'Mark read', exact: true }).click();
  await expect(band).toHaveAttribute('data-reading-state', 'read');
  await expect.poll(beforePassage).toBe(true);
});

test('Why stays a plain text action and its reason preserves the passage gap', async ({ page }) => {
  const band = await mount(page);
  await band.getByRole('button', { name: 'Mark read', exact: true }).click();
  await expect(band).toHaveAttribute('data-reading-state', 'read');
  const why = band.getByRole('button', { name: 'Why', exact: true });
  await why.hover();
  await expect(why).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
  const content = page.frameLocator('#ask-page-fixture iframe').locator('tmt-read').first();
  const gap = async () => {
    const [cue, passage] = await Promise.all([
      band.boundingBox(),
      content.locator('p').boundingBox(),
    ]);
    return passage!.y - cue!.y - cue!.height;
  };
  await placed(page, band);
  await expect.poll(gap).toBeGreaterThanOrEqual(0);
  const closedGap = await gap();
  await why.click();
  await expect(why).toHaveAttribute('aria-expanded', 'true');
  await expect(why).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
  await placed(page, band);
  await expect.poll(async () => Math.abs((await gap()) - closedGap)).toBeLessThan(1.01);
  const [line, reason] = await Promise.all([
    band.locator('.needs-you-bar').boundingBox(),
    band.locator('.reading-reason').boundingBox(),
  ]);
  expect(reason!.y - line!.y - line!.height).toBeCloseTo(4, 0);
  await why.press('Enter');
  await expect(why).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
});
