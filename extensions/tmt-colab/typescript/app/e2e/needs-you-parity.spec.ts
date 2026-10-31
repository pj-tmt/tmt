import { expect, test } from '@playwright/test';
import vectors from '../../../contracts/vectors/needs-you-v1.json' with { type: 'json' };

for (const vector of vectors.cases) {
  test(`native/browser legacy pending parity: ${vector.name}`, async ({ page }) => {
    await page.goto('/');
    await page.evaluate(async (vector) => {
      const fixture = '/test/ask-page-browser.tsx';
      const app = await import(/* @vite-ignore */ fixture);
      await app.mount();
      app.pendingVector(vector);
    }, vector);
    const host = page.locator('#ask-page-fixture');
    // #5814: this owner corpus case retains a proposal record without an authored target.
    const expected =
      vector.name === 'standalone question counts once'
        ? vector.expected.filter((item) => item.kind !== 'proposal')
        : vector.expected;
    const count = expected.length;
    if (count) {
      const trigger = host.getByRole('button', { name: `Needs you: ${count}`, exact: true });
      await expect(trigger).toBeVisible();
      await trigger.click();
      const items = page.getByRole('menu').locator('[role=menuitem][data-needs-you-kind]');
      await expect(items).toHaveCount(count);
      // #5814 adds one command, not another pending target or another header count.
      await expect(page.getByRole('menu').getByRole('menuitem')).toHaveCount(count + 1);
      await expect(page.getByRole('menuitem', { name: 'Clear all', exact: true })).toHaveCount(1);
      expect(
        await items.evaluateAll((nodes) =>
          nodes.map((node) => ({
            kind: (node as HTMLElement).dataset.needsYouKind,
            id: (node as HTMLElement).dataset.needsYouId,
          })),
        ),
      ).toEqual(expected);
    } else {
      await expect(host.getByLabel('✓ Nothing waits', { exact: true })).toBeVisible();
      await expect(host.locator('.needs-you-header button')).toHaveCount(0);
    }
  });
}

for (const detached of [false, true]) {
  test(`proposal navigation keeps records unchanged (${detached ? 'omitted without target' : 'inline'})`, async ({
    page,
  }) => {
    const vector = structuredClone(vectors.cases[0]);
    if (detached) vector.source = vector.source.replace(/<tmt-proposal[^>]*><\/tmt-proposal>/, '');
    await page.goto('/');
    await page.evaluate(async (vector) => {
      const fixture = '/test/ask-page-browser.tsx';
      const app = await import(/* @vite-ignore */ fixture);
      await app.mount();
      app.pendingVector(vector);
    }, vector);
    const proof = () =>
      page.evaluate(async () => {
        const fixture = '/test/ask-page-browser.tsx';
        const app = await import(/* @vite-ignore */ fixture);
        return { records: app.discussionProof(), sends: app.proof() };
      });
    const count = detached ? 3 : 4;
    const trigger = page.getByRole('button', { name: `Needs you: ${count}`, exact: true });
    await expect(trigger).toBeVisible();
    const before = await proof();
    await trigger.click();
    const proposal = page.getByRole('menuitem', { name: 'Choose release · Agent 1' });
    if (detached) {
      await expect(proposal).toHaveCount(0);
      await expect(page.getByRole('menu').getByText('Please decide · 1')).toHaveCount(0);
      await page.keyboard.press('Escape');
    } else {
      await expect(page.getByRole('menu').getByText('Please decide · 1')).toBeVisible();
      await proposal.click();
      await expect(page.getByRole('menu')).toHaveCount(0);
      const target = page.locator('[data-inline-proposal]');
      await expect(target).toBeVisible();
      await expect
        .poll(() => target.evaluate((node) => node.contains(document.activeElement)))
        .toBe(true);
    }
    expect(await proof()).toEqual(before);
  });
}
