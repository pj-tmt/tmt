import { expect, test } from '@playwright/test';

for (const renderer of ['native', 'react'])
  for (const theme of ['light', 'dark'])
    for (const width of [1440, 390])
      test(`rail tree ${renderer} ${theme} ${width}`, async ({ page }, testInfo) => {
        await page.setViewportSize({ width, height: 900 });
        await page.goto(`/?frame=rail&tree=true&renderer=${renderer}&theme=${theme}`);
        const rail = page.locator('nav.tmt-ui-rail');
        if (width === 390) await page.getByRole('button', { name: 'Open navigation' }).click();
        await expect(rail).toBeVisible();
        const menu = rail.locator('.tmt-ui-rail-sub');
        const link = (id: string) => menu.locator(`a[data-id="${id}"]`);
        const disclosure = (id: string) =>
          menu.locator(`li[data-menu-item="${id}"] > .tmt-ui-rail-disclosure`);
        const hostOpen = page.locator('[data-host-open]');
        const selected = menu.locator('[aria-current="page"]');
        const capture = async (state: string) =>
          testInfo.attach(`rail-tree-${renderer}-${theme}-${width}-${state}`, {
            body: await page.screenshot(),
            contentType: 'image/png',
          });

        // The collapsed night squad holds the current member, so it is the one selected row.
        await expect(selected).toHaveCount(1);
        await expect(selected).toHaveAttribute('data-id', 'night');
        await expect(link('night-1')).toHaveCount(0);
        await expect(disclosure('night')).toHaveAttribute('aria-expanded', 'false');
        await expect(disclosure('night')).toHaveAccessibleName('Show night');
        await expect(disclosure('tmt-gui')).toHaveAttribute('aria-expanded', 'true');
        await expect(disclosure('tmt-gui')).toHaveAccessibleName('Hide tmt-gui');
        await expect(menu.locator('li').last()).toHaveClass(/tmt-ui-rail-settings/);

        // Unread is bold primary ink, never a colour of its own; the count slot is unchanged.
        const unread = link('gui-2');
        await expect(unread).toHaveAttribute('data-emphasis', 'unread');
        await expect(unread).toHaveAccessibleName(/^gui-2 unread\b/);
        await expect(unread).toHaveCSS('font-weight', '700');
        const ink = await selected.evaluate((node) => getComputedStyle(node).color);
        await expect(unread).toHaveCSS('color', ink);
        await expect(link('gui-3')).toHaveCSS('font-weight', '400');
        await expect(unread.locator('.tmt-ui-shell-count')).toHaveText('2');
        await expect(unread.locator('.tmt-ui-attention-dot')).toHaveCount(0);
        await expect(unread).toHaveAccessibleName('gui-2 unread · 2 total');

        // Children hang one rule and the 8px sub-indent beyond their parent's label.
        const label = async (id: string) =>
          (await link(id).locator('.tmt-ui-module-menu-label').boundingBox())!.x;
        const children = menu.locator('li[data-menu-item="tmt-gui"] > .tmt-ui-rail-sub-children');
        await expect(children).toHaveCSS('padding-left', '8px');
        await expect(children).toHaveCSS('border-left-width', '1px');
        const parentX = await label('tmt-gui');
        const ruleX = (await children.boundingBox())!.x;
        expect(Math.abs(ruleX - parentX)).toBeLessThanOrEqual(1);
        expect((await label('gui-2')) - parentX).toBeCloseTo(22, 0);
        await capture('initial');

        // The disclosure asks the host; the drawer stays open and the current child takes over.
        await disclosure('night').click();
        await expect(hostOpen).toHaveAttribute('data-host-open', 'tmt-gui,night');
        await expect(disclosure('night')).toHaveAttribute('aria-expanded', 'true');
        await expect(selected).toHaveCount(1);
        await expect(selected).toHaveAttribute('data-id', 'night-1');
        await expect(disclosure('night')).toBeFocused();
        if (width === 390)
          await expect(page.locator('.tmt-ui-shell')).toHaveAttribute('data-drawer', 'open');
        await capture('expanded');

        // Keyboard: Left closes, Right opens then enters, Up/Down walk rows, Left leaves a branch.
        await link('tmt-gui').focus();
        await page.keyboard.press('ArrowLeft');
        await expect(hostOpen).toHaveAttribute('data-host-open', 'night');
        await expect(link('tmt-gui')).toBeFocused();
        await expect(link('gui-2')).toHaveCount(0);
        await page.keyboard.press('ArrowRight');
        await expect(hostOpen).toHaveAttribute('data-host-open', 'night,tmt-gui');
        await expect(link('tmt-gui')).toBeFocused();
        await page.keyboard.press('ArrowRight');
        await expect(link('tmt-gui-lead')).toBeFocused();
        await page.keyboard.press('ArrowDown');
        await expect(link('gui-2')).toBeFocused();
        await page.keyboard.press('ArrowLeft');
        await expect(link('tmt-gui')).toBeFocused();
        await page.keyboard.press('ArrowUp');
        await expect(menu.locator('a').first()).toBeFocused();
        await link('tmt-gui').focus();
        await page.keyboard.press('Tab');
        await expect(disclosure('tmt-gui')).toBeFocused();
        await page.keyboard.press('Enter');
        await expect(hostOpen).toHaveAttribute('data-host-open', 'night');
        await expect(disclosure('tmt-gui')).toBeFocused();

        // A child link still navigates through the host's select.
        await link('night-lead').click();
        await expect(selected).toHaveAttribute('data-id', 'night-lead');
      });

test('flat menus draw no tree markup', async ({ page }) => {
  await page.goto('/?frame=rail&interactive=true&renderer=native');
  const menu = page.locator('nav.tmt-ui-rail .tmt-ui-rail-sub');
  await expect(menu.locator('a').first()).toBeVisible();
  await expect(
    menu.locator(
      '[data-disclosure], [data-emphasis], .tmt-ui-rail-disclosure, .tmt-ui-rail-sub-children',
    ),
  ).toHaveCount(0);
});
