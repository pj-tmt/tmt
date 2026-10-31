import { expect, test } from '@playwright/test';

for (const theme of ['light', 'dark'] as const)
  for (const width of [1440, 390]) {
    test(`editorial layer sets plain prose on the ramp at ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 1600 });
      await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
      await page.goto('/?frame=prose&theme=' + theme);
      const prose = page.locator(`.gallery-column[data-theme="${theme}"] [data-testid="prose"]`);
      await expect(prose).toBeVisible();
      const measured = await prose.evaluate((root) => {
        const style = (selector: string) => getComputedStyle(root.querySelector(selector)!);
        const token = (name: string) => getComputedStyle(root).getPropertyValue(name).trim();
        const probe = document.createElement('span');
        probe.style.background = 'var(--tmt-ui-surface-code)';
        root.append(probe);
        const codeSurface = getComputedStyle(probe).backgroundColor;
        probe.remove();
        return {
          body: [style('p').fontSize, style('p').lineHeight],
          h1: style('h1').fontSize,
          h2: [style('h2').fontSize, style('h2').fontWeight, style('h2').marginTop],
          h3: style('h3').fontSize,
          h4: [style('h4').fontSize, style('h4').fontWeight],
          gap: style('p').marginBottom,
          firstTop: style('h1').marginTop,
          item: style('li + li').marginTop,
          link: [style('a').color, style('a').textDecorationLine, token('--tmt-ui-color-link')],
          quote: [style('blockquote').borderLeftStyle, style('blockquote').paddingLeft],
          cell: [style('td').borderBottomStyle, style('th').fontWeight],
          caption: [style('figcaption').fontSize, style('figcaption').marginTop],
          code: [style('p code').backgroundColor, codeSurface],
          pre: [style('pre').whiteSpace, style('pre').fontSize, style('pre code').backgroundColor],
          tag: [style('.tmt-ui-tag').fontSize, style('.tmt-ui-tag').borderTopStyle],
          overflow: root.scrollWidth - root.clientWidth,
        };
      });
      const compact = width <= 760;
      expect(measured.body).toEqual([compact ? '14px' : '15px', compact ? '22.4px' : '24px']);
      expect(measured.h1).toBe('29px');
      expect(measured.h2).toEqual(['22px', '660', '32px']);
      expect(measured.h3).toBe('19px');
      expect(measured.h4).toEqual(['15px', '660']);
      expect(measured.gap).toBe('16px');
      expect(measured.firstTop).toBe('0px');
      expect(measured.item).toBe('4px');
      expect(measured.link[1]).toBe('underline');
      expect(measured.quote).toEqual(['solid', '16px']);
      expect(measured.cell).toEqual(['solid', '600']);
      expect(measured.caption).toEqual(['12px', '8px']);
      expect(measured.code[0]).toBe(measured.code[1]);
      expect(measured.pre[0]).toBe('pre-wrap');
      expect(measured.pre[1]).toBe('13px');
      expect(measured.pre[2]).toBe('rgba(0, 0, 0, 0)');
      expect(measured.tag).toEqual(['11px', 'solid']);
      expect(measured.overflow).toBeLessThanOrEqual(0);
    });
  }

test('a component class inside prose keeps its own look', async ({ page }) => {
  await page.goto('/?frame=prose&theme=light');
  const sizes = await page.evaluate(() => {
    const root = document.querySelector('[data-testid="prose"]')!;
    const action = document.createElement('a');
    action.className = 'tmt-ui-action';
    action.textContent = 'Open';
    action.href = '#';
    root.append(action);
    const style = getComputedStyle(action);
    return [style.textDecorationLine, style.color];
  });
  const outside = await page.evaluate(() => {
    const action = document.createElement('a');
    action.className = 'tmt-ui-action';
    action.textContent = 'Open';
    action.href = '#';
    document.body.append(action);
    const style = getComputedStyle(action);
    return [style.textDecorationLine, style.color];
  });
  expect(sizes).toEqual(outside);
});
