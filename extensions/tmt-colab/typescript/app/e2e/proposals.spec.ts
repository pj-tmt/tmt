import { mkdirSync } from 'node:fs';
import { expect, test, type Page } from '@playwright/test';
import { pageAction } from '../test/page-actions.js';
const fixture = '/test/ask-page-browser.tsx';
async function run(page: Page, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
async function mount(page: Page, placement = 'inline') {
  await page.goto('/');
  await run(page, 'mount');
  await run(page, 'proposal', placement);
  await expect(page.locator('#ask-page-fixture .status')).toContainText('Live');
}
const captureDir = process.env.COLAB_PROPOSAL_CAPTURE_DIR;
async function capture(page: Page, name: string) {
  if (!captureDir) return;
  mkdirSync(captureDir, { recursive: true });
  await page.screenshot({ path: `${captureDir}/${name}.png` });
}
for (const width of [1440, 390])
  for (const theme of ['light', 'dark']) {
    test(`proposal lifecycle ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await mount(page);
      if (theme === 'dark')
        await (await pageAction(page.locator('#ask-page-fixture'), 'Theme: Dark')).click();
      const inline = page.locator('[data-inline-proposal]').first();
      await expect(inline).not.toHaveAttribute('data-detached');
      const card = inline.locator('.proposal-card');
      await expect(card).toHaveCSS('box-shadow', 'none');
      await expect(card).toHaveCSS('border-top-width', '1px');
      const after = page.frameLocator('#ask-page-fixture iframe').locator('#after-proposal');
      const top = await after.evaluate((node) => node.getBoundingClientRect().top);
      const box = await card.boundingBox();
      expect(top).toBeGreaterThan(box!.height);
      await capture(page, `open-${width}-${theme}`);
      // Author messages and synthetic DOM activations have no decision/send authority.
      await card
        .getByRole('button', { name: 'Approve', exact: true })
        .evaluate((node) => (node as HTMLButtonElement).click());
      expect((await run(page, 'proof')).sends).toHaveLength(0);
      await card.getByRole('button', { name: 'Approve', exact: true }).click();
      await expect(card.locator('.proposal-state')).toHaveText('Approved');
      await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
      const threads = await run(page, 'discussionProof');
      expect(threads[0].decision.decision).toBe('approved');
      expect(threads[0].comments[0].body).toBe('Approved: Use a clearer project heading');
      await capture(page, `approved-${width}-${theme}`);
      await card.getByRole('button', { name: 'Resolve', exact: true }).click();
      await expect(card).toHaveAttribute('data-resolved', 'true');
      await expect(card.locator('.proposal-body')).toHaveCount(0);
      await expect
        .poll(async () => {
          const below = await after.boundingBox(),
            slot = await inline.boundingBox();
          return below!.y - slot!.y - slot!.height;
        })
        .toBeLessThanOrEqual(32);
      await capture(page, `resolved-${width}-${theme}`);
      await card.getByRole('button', { name: 'Reopen', exact: true }).click();
      await expect(card.locator('.proposal-state')).toHaveText('Approved');
      await expect(card.getByRole('button', { name: 'Approve', exact: true })).toHaveCount(0);
      await capture(page, `reopened-${width}-${theme}`);
      await card.getByRole('button', { name: 'Follow up', exact: true }).click();
      const composer = page.getByTestId('comment-thread').getByRole('combobox');
      await expect(composer).toBeFocused();
      await expect(composer).toContainText('@Agent 1');
      await capture(page, `follow-up-${width}-${theme}`);
      expect((await run(page, 'proof')).sends).toHaveLength(1);
    });
  }
for (const placement of ['missing', 'duplicate'])
  test(`proposal ${placement} is detached in Comments`, async ({ page }) => {
    await mount(page, placement);
    await expect(page.locator('[data-inline-proposal]')).toHaveAttribute('data-detached', 'true');
    await (await pageAction(page.locator('#ask-page-fixture'), 'Comments')).click();
    await page.locator('[data-testid="annotation-row"]').click();
    const card = page.locator('.page-drawer[data-panel="comments"][open] .proposal-card');
    await expect(card).toBeVisible();
    await expect(card.locator('.proposal-author')).toContainText('Detached');
    expect((await run(page, 'proof')).sends).toHaveLength(0);
  });
test('vanished spacer detaches without another publication or send', async ({ page }) => {
  await mount(page);
  const inline = page.locator('[data-inline-proposal]');
  await expect(inline).not.toHaveAttribute('data-detached');
  await page
    .frameLocator('#ask-page-fixture iframe')
    .locator('[data-colab-proposal-slot]')
    .evaluate((node) => node.remove());
  await expect(inline).toHaveAttribute('data-detached', 'true');
  expect((await run(page, 'proof')).sends).toHaveLength(0);
});

for (const width of [1440, 390])
  for (const theme of ['light', 'dark']) {
    test(`declined and notification states ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await mount(page);
      if (theme === 'dark')
        await (await pageAction(page.locator('#ask-page-fixture'), 'Theme: Dark')).click();
      const card = page.locator('[data-inline-proposal] .proposal-card');
      await expect(card).toBeVisible();
      await card.getByRole('button', { name: 'Decline', exact: true }).click();
      await expect(card.locator('.proposal-state')).toHaveText('Declined');
      await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
      await capture(page, `declined-${width}-${theme}`);
      for (const state of ['held', 'uncertain', 'refused', 'replied']) {
        await run(page, 'proposalAsk', state);
        if (state === 'replied') await expect(card.getByTestId('ask-reply')).toBeVisible();
        else await expect(card.getByTestId('ask-state')).toHaveAttribute('data-state', state);
        if (state === 'refused') {
          const retry = card.getByRole('button', { name: 'Ask again', exact: true });
          await expect(retry).toBeVisible();
          await retry.evaluate((node) => (node as HTMLButtonElement).click());
          expect((await run(page, 'proof')).sends).toHaveLength(1);
        }
        await capture(page, `${state}-${width}-${theme}`);
      }
      expect((await run(page, 'proof')).sends).toHaveLength(1);
    });
    test(`detached and unavailable ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto('/');
      await run(page, 'mount', { agents: [] });
      await run(page, 'proposal', 'missing');
      if (theme === 'dark')
        await (await pageAction(page.locator('#ask-page-fixture'), 'Theme: Dark')).click();
      await (await pageAction(page.locator('#ask-page-fixture'), 'Comments')).click();
      await page.getByTestId('annotation-row').click();
      const card = page.locator('.page-drawer[data-panel="comments"][open] .proposal-card');
      await expect(card).toBeVisible();
      await capture(page, `detached-${width}-${theme}`);
      await card.getByRole('button', { name: 'Approve', exact: true }).click();
      await expect(card.getByRole('alert')).toHaveText(
        'Decision saved. The proposer could not be notified.',
      );
      await expect(card.locator('.proposal-state')).toHaveText('Approved');
      await capture(page, `unavailable-${width}-${theme}`);
      expect((await run(page, 'proof')).sends).toHaveLength(0);
    });
  }
