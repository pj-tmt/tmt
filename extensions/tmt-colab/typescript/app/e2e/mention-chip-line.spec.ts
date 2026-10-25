import { expect, test, type Locator, type Page } from '@playwright/test';
import { capturePath } from './captures.js';

const fixture = '/test/message-composer-browser.html';
async function mount(page: Page) {
  await page.goto(fixture);
  await page.evaluate(async () => {
    const path = '/test/message-composer-browser.tsx';
    (await import(path)).mountMentions('online');
  });
  await expect(page.locator('.agent-chip[data-variant="chip"]')).toHaveCount(1);
  return page.getByRole('combobox', { name: 'Message', exact: true });
}

/** The y of a text baseline: the bottom of a zero-size inline-block sits on it. */
async function measure(page: Page) {
  return page.evaluate(() => {
    const chip = document.querySelector('.agent-chip[data-variant="chip"]') as HTMLElement;
    const paragraph = chip.closest('p') as HTMLElement;
    const baselineIn = (place: (probe: HTMLElement) => void) => {
      const probe = document.createElement('span');
      probe.style.cssText = 'display:inline-block;width:0;height:0;vertical-align:baseline';
      place(probe);
      const y = probe.getBoundingClientRect().bottom;
      probe.remove();
      return y;
    };
    const lineHeight = parseFloat(getComputedStyle(paragraph).lineHeight);
    const box = paragraph.getBoundingClientRect();
    return {
      lineHeight,
      chipHeight: chip.getBoundingClientRect().height,
      // The chip's own name against the line the chip sits in (a probe right after the chip).
      nameBaseline: baselineIn((probe) => chip.querySelector('.agent-chip-name')!.append(probe)),
      lineBaseline: baselineIn((probe) => chip.after(probe)),
      lines: box.height / lineHeight,
    };
  });
}

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`the composer mention chip sits on the text line: ${width}px ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 844 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((value) => {
        document.documentElement.dataset.theme = value;
      }, theme);
      const input = await mount(page);
      await input.click();
      await page.keyboard.press('End');
      await page.keyboard.type(
        'please review the section below and tell me what you think about the long wrapped sentence that keeps going',
      );
      const m = await measure(page);
      // Its height fits the line box.
      expect(m.chipHeight).toBeLessThanOrEqual(m.lineHeight);
      // Its text shares the surrounding text's baseline.
      expect(Math.abs(m.nameBaseline - m.lineBaseline)).toBeLessThanOrEqual(1);
      // Wrapped lines keep the even line spacing: the paragraph is a whole number of lines.
      expect(Math.round(m.lines)).toBeGreaterThanOrEqual(2);
      expect(Math.abs(m.lines - Math.round(m.lines)) * m.lineHeight).toBeLessThanOrEqual(1);
      await page.screenshot({ path: await capturePath(`3897-chip-line-${width}-${theme}.png`) });
    });

/** The x of the text right after the chip: a zero-size probe placed behind it. */
const textAfter = (chip: Locator) =>
  chip.evaluate((node) => {
    const probe = document.createElement('span');
    probe.style.cssText = 'display:inline-block;width:0;height:0';
    node.after(probe);
    const left = probe.getBoundingClientRect().left;
    probe.remove();
    return left;
  });

test('a resting chip is its label and padding, and the remove badge on hover moves nothing and covers no text', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 844 });
  const input = await mount(page);
  await input.click();
  await page.keyboard.press('End');
  await page.keyboard.type(' and the rest of the sentence');
  const chip = page.locator('.agent-chip[data-variant="chip"]');
  const control = chip.locator('.tmt-ui-icon-action');
  const button = control.locator('button');
  await page.mouse.move(2, 2);
  await expect(control).toHaveCSS('opacity', '0');
  // The control is out of the line: the chip is 9px of padding and edge, the label, 9px again.
  await expect(control).toHaveCSS('position', 'absolute');
  const resting = (await chip.boundingBox())!;
  const label = (await chip.locator('.agent-chip-name').boundingBox())!;
  expect(Math.abs(resting.width - (label.width + 18))).toBeLessThanOrEqual(1);
  const next = await textAfter(chip);
  await chip.hover();
  await expect(control).toHaveCSS('opacity', '1');
  expect(await textAfter(chip)).toBe(next);
  expect((await chip.boundingBox())!.width).toBe(resting.width);
  // A 16px badge on the chip's top-right corner, 7px outside it, whose box stops where the label's ends.
  const badge = (await button.boundingBox())!;
  expect(Math.abs(badge.width - 16)).toBeLessThanOrEqual(1);
  expect(Math.abs(badge.height - 16)).toBeLessThanOrEqual(1);
  expect(Math.abs(badge.x + badge.width - (resting.x + resting.width) - 7)).toBeLessThanOrEqual(1);
  expect(Math.abs(resting.y - badge.y - 7)).toBeLessThanOrEqual(1);
  expect(badge.x).toBeGreaterThanOrEqual(label.x + label.width - 1);
  // The hit area is 24px: a point 11px from the badge's centre still reaches the button.
  const hit = await button.evaluate((node) => {
    const rect = node.getBoundingClientRect();
    const reach = (dx: number, dy: number) =>
      document
        .elementFromPoint(rect.left + rect.width / 2 + dx, rect.top + rect.height / 2 + dy)
        ?.closest('button') === node;
    return {
      near: reach(11, 0) && reach(0, 11) && reach(-11, 0) && reach(0, -11),
      far: reach(14, 0),
    };
  });
  expect(hit).toEqual({ near: true, far: false });
  // It still removes the mention.
  await button.click();
  await expect(chip).toHaveCount(0);
});

test.describe('without hover', () => {
  test.use({ hasTouch: true, isMobile: true });
  test('the remove control stays in the line and visible', async ({ page }) => {
    await mount(page);
    const control = page.locator('.agent-chip[data-variant="chip"] .tmt-ui-icon-action');
    await expect(control).toHaveCSS('opacity', '1');
    await expect(control).toHaveCSS('position', 'static');
  });
});
