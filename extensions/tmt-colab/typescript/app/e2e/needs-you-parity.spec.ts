import { expect, test } from '@playwright/test';
import vectors from '../../../contracts/vectors/needs-you-v1.json' with { type: 'json' };

for (const vector of vectors.cases) {
  test(`native/browser pending parity: ${vector.name}`, async ({ page }) => {
    await page.goto('/');
    await page.evaluate(async (vector) => {
      const fixture = '/test/ask-page-browser.tsx';
      const app = await import(/* @vite-ignore */ fixture);
      await app.mount();
      app.pendingVector(vector);
    }, vector);
    const host = page.locator('#ask-page-fixture');
    const count = vector.expected.length;
    if (count) {
      const trigger = host.getByRole('button', { name: `Needs you: ${count}`, exact: true });
      await expect(trigger).toBeVisible();
      await trigger.click();
      const items = page.getByRole('menu').getByRole('menuitem');
      await expect(items).toHaveCount(count);
      expect(
        await items.evaluateAll((nodes) =>
          nodes.map((node) => ({
            kind: (node as HTMLElement).dataset.needsYouKind,
            id: (node as HTMLElement).dataset.needsYouId,
          })),
        ),
      ).toEqual(vector.expected);
    } else {
      await expect(host.getByLabel('✓ Nothing waits', { exact: true })).toBeVisible();
      await expect(host.locator('.needs-you-header button')).toHaveCount(0);
    }
  });
}

for (const detached of [false, true]) {
  test(`proposal navigation keeps records unchanged (${detached ? 'comments fallback' : 'inline'})`, async ({
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
    await expect(page.getByRole('button', { name: 'Needs you: 4', exact: true })).toBeVisible();
    const before = await proof();
    await page.getByRole('button', { name: 'Needs you: 4', exact: true }).click();
    await expect(page.getByRole('menu').getByText('Please decide · 1')).toBeVisible();
    await page.getByRole('menuitem', { name: 'Choose release · Agent 1' }).click();
    await expect(page.getByRole('menu')).toHaveCount(0);
    const target = detached
      ? page.getByRole('dialog', { name: 'Comments', exact: true })
      : page.locator('[data-inline-proposal]');
    await expect(target).toBeVisible();
    if (!detached)
      await expect
        .poll(() => target.evaluate((node) => node.contains(document.activeElement)))
        .toBe(true);
    expect(await proof()).toEqual(before);
  });
}
