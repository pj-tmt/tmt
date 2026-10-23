import { expect, test } from '@playwright/test';

// Exercise the presentation leaf with the existing headless browser harness.
const fixture = '/test/browser-tabs.html';

for (const theme of ['light', 'dark']) {
  for (const width of [1440, 390, 320]) {
    test(`tabs keyboard, names, drafts and overflow at ${width} ${theme}`, async ({
      page,
    }, info) => {
      await page.setViewportSize({ width, height: 800 });
      await page.goto(fixture);
      await page.evaluate((value) => {
        document.documentElement.dataset.theme = value;
      }, theme);
      const tabs = page.getByRole('tab');
      const selected = page.getByRole('tab', { selected: true });
      await expect(tabs).toHaveCount(12);
      await expect(
        page.getByRole('tab', { name: '✓ Layout · Answered', exact: true }),
      ).toBeVisible();
      await expect(selected).toHaveAccessibleName('◆ Pages · Waiting');
      const statusBelow = () =>
        page.locator('.tmt-ui-tabs-header').evaluate((header) => {
          const list = header.querySelector('[role="tablist"]')!.getBoundingClientRect();
          const status = header.querySelector('.tmt-ui-tabs-status')!.getBoundingClientRect();
          return status.top >= list.bottom - 1;
        });
      expect(await statusBelow()).toBe(width < 480);
      if (width !== 320)
        await page.screenshot({
          path: info.outputPath(`tabs-${width}-${theme}.png`),
          fullPage: true,
        });
      await selected.focus();
      await selected.press('ArrowLeft');
      await expect(selected).toHaveAccessibleName('✓ Layout · Answered');
      await expect(selected).toBeFocused();
      await expect(page.locator('#events')).toHaveText('1:keyboard:true');
      const draft = page.getByRole('textbox', { name: 'Layout draft' });
      await selected.press('Tab');
      await expect(draft).toBeFocused();
      await draft.fill('Unsent draft 3');
      await selected.focus();
      await selected.press('End');
      await expect(selected).toHaveAccessibleName('◆ Decision 12 · Waiting');
      await expect(selected).toBeFocused();
      await expect(page.getByRole('tabpanel')).toHaveCount(1);
      await expect(page.getByRole('textbox', { name: 'Layout draft' })).not.toBeVisible();
      await expect(page.locator('#panel-1 input')).toHaveValue('Unsent draft 3');
      await selected.press('ArrowRight');
      await expect(selected).toHaveAccessibleName('✓ Layout · Answered');
      await expect(draft).toHaveValue('Unsent draft 3');
      const longTab = page.getByRole('tab', { name: `◆ ${'W'.repeat(24)} · Waiting`, exact: true });
      await longTab.click();
      await expect(longTab).toHaveAttribute('aria-selected', 'true');
      await expect(longTab).toHaveAccessibleName(`◆ ${'W'.repeat(24)} · Waiting`);
      expect(
        await longTab.evaluate((tab) => {
          const a = tab.parentElement!.getBoundingClientRect(),
            b = tab.getBoundingClientRect();
          return b.left >= a.left - 1 && b.right <= a.right + 1;
        }),
      ).toBe(true);
      await selected.press('Home');
      await expect(selected).toBeFocused();
      const output = page.locator('#events');
      const before = JSON.parse((await output.getAttribute('data-events'))!) as string[];
      await selected.press('Enter');
      await expect(output).toHaveAttribute(
        'data-events',
        JSON.stringify([...before, '1:activation:true']),
      );
      await selected.press('Space');
      await expect(output).toHaveAttribute(
        'data-events',
        JSON.stringify([...before, '1:activation:true', '1:activation:true']),
      );
      await selected.press('End');
      const geometry = await page.locator('.tmt-ui-tab-list').evaluate((list) => {
        const chosen = list.querySelector('[aria-selected="true"]')!;
        const a = list.getBoundingClientRect(),
          b = chosen.getBoundingClientRect();
        return {
          contained: b.left >= a.left - 1 && b.right <= a.right + 1,
          pageFits: document.documentElement.scrollWidth <= innerWidth,
          pageY: scrollY,
          selectedRule: getComputedStyle(chosen).borderTopWidth,
          selectedFilled: getComputedStyle(chosen).backgroundColor !== 'rgba(0, 0, 0, 0)',
          shadow: getComputedStyle(chosen).boxShadow,
        };
      });
      const overflow = geometry.pageFits
        ? []
        : await page.locator('body *').evaluateAll((nodes) =>
            nodes
              .filter((node) => node.getBoundingClientRect().right > innerWidth + 1)
              .map((node) => ({
                tag: node.tagName,
                id: node.id,
                class: node.className,
                width: node.getBoundingClientRect().width,
                minWidth: getComputedStyle(node).minWidth,
              })),
          );
      expect(geometry, JSON.stringify(overflow)).toEqual({
        contained: true,
        pageFits: true,
        pageY: 0,
        // A segmented strip: the selected tab is a fill, not a rule on one edge.
        selectedRule: '0px',
        selectedFilled: true,
        shadow: 'none',
      });
      // Resize must retain the selected tab without an extra selection or page scroll.
      await page.setViewportSize({ width: 320, height: 800 });
      await expect
        .poll(() =>
          selected.evaluate((tab) => {
            const a = tab.parentElement!.getBoundingClientRect(),
              b = tab.getBoundingClientRect();
            return b.left >= a.left - 1 && b.right <= a.right + 1;
          }),
        )
        .toBe(true);
      expect(await page.evaluate(() => scrollY)).toBe(0);
      expect(
        await tabs.evaluateAll(
          (nodes) => nodes.filter((node) => node.getAttribute('tabindex') === '0').length,
        ),
      ).toBe(1);
      // Hosts opt into a panel stop; default Tab navigation goes straight to controls.
      await page.goto(`${fixture}?focus-panel`);
      await selected.focus();
      await selected.press('Tab');
      await expect(page.getByRole('tabpanel')).toBeFocused();
      await page.keyboard.press('Tab');
      await expect(page.getByRole('textbox', { name: 'Pages draft' })).toBeFocused();
      if (width === 1440) {
        await page.setViewportSize({ width, height: 800 });
        await page.locator('#root').evaluate((root) => {
          (root as HTMLElement).style.width = '390px';
        });
        expect(await statusBelow()).toBe(true);
      }
    });
  }
}
