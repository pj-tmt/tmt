import { expect, test, type Locator, type Page } from '@playwright/test';
import { text } from '../src/strings.js';
import { capturePath } from './captures.js';

/**
 * A review's note fields keep their look while its write is in flight (gui-style.md, Transient
 * states): fenced and readable, not natively disabled, and nothing moves. A real failure is shown
 * at once.
 */
const fixture = '/test/ask-page-browser.tsx';
const run = (page: Page, method: string, argument?: unknown) =>
  page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
const look = process.env.COLAB_LOOK ?? '1440:light';

async function mount(page: Page) {
  const [width, scheme] = look.split(':');
  await page.setViewportSize({ width: Number(width), height: 900 });
  await page.emulateMedia({ colorScheme: scheme === 'dark' ? 'dark' : 'light' });
  await page.goto('/');
  await run(page, 'mount');
  await run(page, 'review');
  const card = page.locator('.review-card').first();
  await expect(page.locator('[data-inline-review]').first()).not.toHaveAttribute('data-detached');
  return card;
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

for (const [kind, open, label] of [
  ['note', text.reviewAddNote, text.reviewNoteLabel],
  ['change', text.reviewNeedsChange, text.reviewChangeLabel],
] as const)
  test(`the ${kind} field keeps its look and its text while its write is in flight`, async ({
    page,
  }) => {
    const card = await mount(page);
    await card.getByRole('button', { name: open, exact: true }).click();
    const field = card.getByLabel(label, { exact: true });
    await field.fill('Exact words to keep');
    await page.mouse.move(2, 2);
    const idle = await appearance(field);
    await run(page, 'holdReply');
    await card
      .getByRole('button', {
        name: kind === 'note' ? text.reviewConfirm : text.reviewChangeSend,
        exact: true,
      })
      .click();
    await expect(card.getByRole('button', { name: /./ }).first()).toHaveAttribute(
      'aria-busy',
      'true',
    );
    // Fenced: readable, inert, and not natively disabled, so it keeps the idle look.
    await expect(field).toHaveValue('Exact words to keep');
    expect(await field.evaluate((node) => (node as HTMLInputElement).disabled)).toBe(false);
    expect(await field.getAttribute('aria-disabled')).toBe('true');
    expect(await field.evaluate((node) => (node as HTMLInputElement).readOnly)).toBe(true);
    const busy = await appearance(field);
    expect(busy.color).toBe(idle.color);
    expect(busy.background).toBe(idle.background);
    expect(busy.border).toBe(idle.border);
    for (const [index, value] of busy.rect.entries())
      expect(Math.abs(value - idle.rect[index]!)).toBeLessThanOrEqual(1);
    if (process.env.COLAB_CAPTURE_DIR)
      await page.screenshot({
        path: capturePath(`review-field-${kind}-busy-${look.replace(':', '-')}.png`),
      });
    await run(page, 'releaseReply');
  });
