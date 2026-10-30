import { expect, test, type Locator } from '@playwright/test';

const scrollOwners = (rail: Locator) =>
  rail.evaluate((node) =>
    [node, ...node.querySelectorAll('*')]
      .filter((entry) => ['auto', 'scroll'].includes(getComputedStyle(entry).overflowY))
      .map((entry) => entry.className),
  );

for (const theme of ['light', 'dark'] as const)
  for (const projects of [3, 20])
    test(`rail ${projects} projects ${theme}: one column, sticky brand, Settings in flow`, async ({
      page,
    }) => {
      await page.setViewportSize({ width: 1440, height: 900 });
      await page.goto(`/?frame=shell&projects=${projects}&theme=${theme}`);
      const rail = page.locator('nav.tmt-ui-rail');
      const brand = rail.locator('> .tmt-ui-shell-brand');
      const bottom = rail.locator('> .tmt-ui-rail-bottom');
      expect(await scrollOwners(rail)).toEqual(['tmt-ui-rail']);
      await expect(brand).toHaveCSS('position', 'sticky');
      await expect(bottom).toHaveCSS('position', 'static');
      const brandTop = (await brand.boundingBox())!.y;
      const cover = await brand.boundingBox();
      const railBox = await rail.boundingBox();
      expect(cover!.x).toBe(railBox!.x);
      expect(cover!.width).toBe(await rail.evaluate((node) => node.clientWidth));
      await expect(rail).not.toHaveAttribute('data-scrolled');
      if (projects === 20) {
        expect((await bottom.boundingBox())!.y).toBeGreaterThan(900);
        await rail.evaluate((node) => {
          node.scrollTop = 100;
        });
        const current = rail.locator('.tmt-ui-rail-item[data-module-current]');
        await current.focus();
        await expect
          .poll(async () => (await current.boundingBox())!.y)
          .toBeGreaterThanOrEqual(cover!.y + cover!.height);
        await rail.evaluate((node) => {
          node.scrollTop = node.scrollHeight;
        });
        await expect(rail).toHaveAttribute('data-scrolled', '');
        expect((await brand.boundingBox())!.y).toBe(brandTop);
        expect(await brand.evaluate((node) => getComputedStyle(node, '::after').height)).toBe(
          '1px',
        );
      }
      const box = (await bottom.boundingBox())!;
      expect(box.y + box.height).toBeLessThanOrEqual(900);
      expect(box.y + box.height).toBeGreaterThan(875);
      // Moving the page leaves the rail's independent scroll position unchanged.
      const top = await rail.evaluate((node) => node.scrollTop);
      await page.evaluate(() => window.scrollTo(0, 200));
      await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(200);
      expect(await rail.evaluate((node) => node.scrollTop)).toBe(top);
      expect((await brand.boundingBox())!.y).toBe(brandTop);
      await page.evaluate(() => window.scrollTo(0, 0));
      if (projects === 20) {
        await page.getByRole('button', { name: 'Use 3 projects', exact: true }).click();
        await expect(rail).not.toHaveAttribute('data-scrolled');
        expect(await rail.evaluate((node) => node.scrollTop)).toBe(0);
        const short = (await bottom.boundingBox())!;
        expect(short.y + short.height).toBeGreaterThan(875);
      }
      await rail.getByRole('link', { name: 'TMT settings', exact: true }).focus();
      await expect(rail.getByRole('link', { name: 'TMT settings', exact: true })).toBeInViewport();
      expect(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth)).toBe(
        false,
      );
    });

for (const theme of ['light', 'dark'] as const)
  test(`phone rail ${theme}: single scroll owner, reachable Settings and original dismissal`, async ({
    page,
  }) => {
    // A short viewport exercises actual drawer overflow; the same project tree stays in the drawer.
    await page.setViewportSize({ width: 390, height: 260 });
    await page.goto(`/?frame=shell&projects=20&theme=${theme}`);
    const rail = page.locator('nav.tmt-ui-rail');
    const open = page.getByRole('button', { name: 'Open navigation', exact: true });
    await open.click();
    expect(await scrollOwners(rail)).toEqual(['tmt-ui-rail']);
    await expect(rail.locator('.tmt-ui-shell-brand')).toBeHidden();
    await expect(rail.locator('.tmt-ui-rail-bottom')).toHaveCSS('position', 'static');
    await rail.getByRole('link', { name: 'TMT settings', exact: true }).focus();
    await expect(rail.getByRole('link', { name: 'TMT settings', exact: true })).toBeInViewport();
    expect(await rail.evaluate((node) => node.scrollTop)).toBeGreaterThan(0);
    await page.keyboard.press('Escape');
    await expect(rail).toBeHidden();
    await expect(open).toBeFocused();
  });

test('disposed rail releases its scroll listener', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/?frame=shell&projects=20');
  const rail = page.locator('nav.tmt-ui-rail');
  await rail.evaluate((node) => {
    node.scrollTop = node.scrollHeight;
  });
  await expect(rail).toHaveAttribute('data-scrolled', '');
  const detached = await rail.elementHandle();
  await page.getByRole('button', { name: 'Unmount shell', exact: true }).click();
  await expect(rail).toHaveCount(0);
  expect(
    await detached!.evaluate((node) => {
      node.scrollTop = 0;
      node.dispatchEvent(new Event('scroll'));
      return node.hasAttribute('data-scrolled');
    }),
  ).toBe(true);
  await detached!.dispose();
});
