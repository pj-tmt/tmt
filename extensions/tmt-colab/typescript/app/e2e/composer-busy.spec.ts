import { expect, test, type Locator, type Page } from '@playwright/test';
import { destination } from '../test/ask-fixtures.js';
import { capturePath } from './captures.js';

/**
 * A comment composer keeps its look and its words while its write is in flight (gui-style.md,
 * Transient states): fenced and readable, not greyed, and nothing moves.
 */
const fixture = '/test/annotation-browser.tsx';
const run = (page: Page, method: string, argument?: unknown) =>
  page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
const look = process.env.COLAB_LOOK ?? '1440:light';

async function mount(page: Page, surface: string) {
  const [width, scheme] = look.split(':');
  await page.setViewportSize({ width: Number(width), height: 900 });
  await page.emulateMedia({ colorScheme: scheme === 'dark' ? 'dark' : 'light' });
  await page.goto('/');
  await run(page, 'setSurface', surface);
  await run(page, 'setAgents', [{ ...destination(), agentName: 'alpha' }]);
  await run(page, 'setCreator', false);
  await run(page, 'setDirectory', 'ok');
  await run(page, 'mount', 'accepted');
  const input = page.getByRole('combobox', { name: 'Message', exact: true });
  await expect(input).toBeVisible();
  await expect(page.locator('.annotation-hint[role="status"]')).not.toContainText('Checking');
  return input;
}
const appearance = (field: Locator) =>
  field.evaluate((node) => {
    const style = getComputedStyle(node);
    const box = node.getBoundingClientRect();
    return {
      color: style.color,
      background: style.backgroundColor,
      border: style.borderTopColor,
      rect: [box.x, box.y, box.width, box.height],
    };
  });

for (const surface of ['annotation', 'reply'])
  test(`${surface}: the composer keeps its look and its words while its write is in flight`, async ({
    page,
  }) => {
    const input = await mount(page, surface);
    await input.fill('Words that stay readable.');
    await page.mouse.move(2, 2);
    const idle = await appearance(input);
    await run(page, 'holdWrite');
    await input.press('Enter');
    // Inert (not editable) and marked busy, with no disabled look.
    await expect(input).toHaveAttribute('aria-busy', 'true');
    await expect(input).toHaveAttribute('contenteditable', 'false');
    await expect(input).toContainText('Words that stay readable.');
    const busy = await appearance(input);
    expect(busy.color).toBe(idle.color);
    expect(busy.background).toBe(idle.background);
    expect(busy.border).toBe(idle.border);
    for (const [index, value] of busy.rect.entries())
      expect(Math.abs(value - idle.rect[index]!)).toBeLessThanOrEqual(1);
    if (process.env.COLAB_CAPTURE_DIR)
      await page.screenshot({
        path: capturePath(`composer-${surface}-busy-${look.replace(':', '-')}.png`),
      });
    await run(page, 'releaseWrite');
  });
