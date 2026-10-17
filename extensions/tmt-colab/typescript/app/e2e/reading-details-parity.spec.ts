import { expect, test } from '@playwright/test';
import inputs from '../../../contracts/vectors/needs-you-v1.json' with { type: 'json' };
import vectors from '../../../contracts/vectors/reading-details-v1.json' with { type: 'json' };

for (const vector of vectors.cases) {
  test(`owner reading descriptor parity: ${vector.pendingCase}`, async ({ page }) => {
    const input = inputs.cases.find((row) => row.name === vector.pendingCase);
    expect(input, 'the named admitted input exists').toBeDefined();
    await page.goto('/');
    await page.evaluate(async (vector) => {
      const fixture = '/test/ask-page-browser.tsx';
      const app = await import(/* @vite-ignore */ fixture);
      await app.mount();
      app.pendingVector(vector);
    }, input);
    const rows = page.locator('[data-inline-reading]:not([data-detached])');
    await expect(rows).toHaveCount(vector.expected.length);
    await expect
      .poll(() =>
        rows.evaluateAll((nodes) =>
          nodes.map((node) => ({
            id: (node as HTMLElement).dataset.inlineReading,
            title: node.getAttribute('aria-label')?.replace(/^Reading: /, ''),
            state: (node as HTMLElement).dataset.readingState,
          })),
        ),
      )
      .toEqual(vector.expected.map(({ id, title, state }) => ({ id, title, state })));
    for (const row of vector.expected) {
      if (row.state === 'unread') {
        const band = page.locator(`[data-inline-reading="${row.id}"] .needs-you-bar`);
        await expect(band).toContainText(row.agent);
        await expect(band).toContainText(row.why);
      }
    }
  });
}
