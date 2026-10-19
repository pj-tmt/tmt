import { expect, test, type Page } from '@playwright/test';
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
    const remove = chip.querySelector('.tmt-ui-icon-action-control') as HTMLElement;
    return {
      lineHeight,
      chipHeight: chip.getBoundingClientRect().height,
      // The chip's own name against the line the chip sits in (a probe right after the chip).
      nameBaseline: baselineIn((probe) => chip.querySelector('.agent-chip-name')!.append(probe)),
      lineBaseline: baselineIn((probe) => chip.after(probe)),
      lines: box.height / lineHeight,
      removeHeight: remove.getBoundingClientRect().height,
      removeWidth: remove.getBoundingClientRect().width,
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
      expect(m.lines).toBeGreaterThanOrEqual(2);
      expect(Math.abs(m.lines - Math.round(m.lines)) * m.lineHeight).toBeLessThanOrEqual(1);
      // The remove control keeps its hit area.
      expect(m.removeHeight).toBeGreaterThanOrEqual(24);
      expect(m.removeWidth).toBeGreaterThanOrEqual(24);
      await page.screenshot({ path: await capturePath(`3897-chip-line-${width}-${theme}.png`) });
    });
