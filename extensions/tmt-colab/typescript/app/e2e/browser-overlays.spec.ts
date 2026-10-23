import { test, expect } from '@playwright/test';

test('Tab out of a menu moves focus on and the close that follows does not take it back to the trigger', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/');
  await page.evaluate(async () => {
    const fixture = '/test/browser-overlays.tsx';
    (await import(fixture)).mount();
  });
  await page.getByRole('button', { name: 'Open drawer' }).click();
  const trigger = page
    .getByRole('dialog', { name: 'Fixture' })
    .getByRole('button', { name: /^Choice:/ });
  await trigger.click();
  const menu = page.getByRole('menu', { name: 'Choice' });
  await expect(menu).toBeVisible();
  await menu.press('Tab');
  // The menu leaves with its exit (hidden from the role tree at once, detached when it ends);
  // its native close event arrives while it is still mounted.
  await expect(page.locator('dialog[aria-label="Choice"]')).toHaveCount(0);
  await expect(trigger).not.toBeFocused();
});

for (const [width, theme] of [
  [1440, 'light'],
  [390, 'dark'],
] as const) {
  test(`shared top layers intercept iframe and header presses, retain draft and unwind focus ${width} ${theme}`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/');
    await page.evaluate(async () => {
      const fixture = '/test/browser-overlays.tsx';
      (await import(fixture)).mount();
    });
    await page.evaluate((value) => (document.documentElement.dataset.theme = value), theme);
    const opener = page.getByRole('button', { name: 'Open drawer' });
    await opener.click();
    const drawer = page.getByRole('dialog', { name: 'Fixture' });
    await expect(drawer).toBeVisible();
    expect(await drawer.evaluate((node) => node.matches(':modal'))).toBe(true);
    const draft = drawer.getByRole('textbox', { name: 'Retained draft' });
    await draft.fill('Keep this draft');
    await draft.evaluate((node) => {
      (node as HTMLElement).dataset.identity = 'kept';
    });
    await drawer.getByRole('button', { name: /^Choice:/ }).click();
    const menu = page.getByRole('menu', { name: 'Choice' });
    await expect(menu.getByRole('menuitemradio', { name: 'First' })).toBeFocused();
    await menu.press('End');
    await expect(menu.getByRole('menuitemradio', { name: 'Last' })).toBeFocused();
    await menu.press('Home');
    await menu.press('l');
    await expect(menu.getByRole('menuitemradio', { name: 'Last' })).toBeFocused();
    await expect(menu.getByRole('menuitemradio', { name: 'Unavailable' })).toBeDisabled();
    await expect(
      menu.getByRole('menuitemradio', { name: 'Unavailable' }),
    ).toHaveAccessibleDescription('Read only');
    await menu.press('Shift+Escape');
    await expect(menu).toBeVisible();
    await menu.press('Escape');
    await expect(menu).toHaveCount(0);
    await expect(drawer).toBeVisible();
    await expect(drawer.getByRole('button', { name: /^Choice:/ })).toBeFocused();
    await drawer.getByRole('button', { name: /^Choice:/ }).click();
    await menu.press('l');
    await expect(menu.getByRole('menuitemradio', { name: 'Last' })).toBeFocused();
    await menu.press('Escape');
    expect(await page.evaluate(() => document.body.style.overflow)).toBe('');
    await expect(drawer.locator('.tmt-ui-drawer-body')).toHaveCSS('overscroll-behavior', 'contain');
    await page.screenshot({ path: testInfo.outputPath(`drawer-${width}-${theme}.png`) });
    // Press the covered header using coordinates; locator.click correctly refuses inert targets.
    const header = await page.getByRole('button', { name: 'Header action' }).boundingBox();
    await page.mouse.click(header!.x + header!.width / 2, header!.y + header!.height / 2);
    await expect(drawer).toBeHidden();
    await expect(page.getByTestId('header-count')).toHaveText('0');
    await expect(opener).toBeFocused();
    await page.getByRole('button', { name: 'Header action' }).click();
    await expect(page.getByTestId('header-count')).toHaveText('1');
    await opener.click();
    await expect(draft).toHaveValue('Keep this draft');
    await expect(draft).toHaveAttribute('data-identity', 'kept');
    await expect(page.getByTestId('mounts')).toHaveText('1');
    await drawer.press('Escape');
    await expect(drawer).toBeHidden();
    await page.getByRole('button', { name: 'Header menu' }).click();
    await expect(page.getByRole('menu', { name: 'Header menu' })).toBeVisible();
    const iframe = await page.locator('iframe').boundingBox();
    await page.mouse.click(iframe!.x + 30, iframe!.y + 50);
    await expect(page.getByRole('menu')).toHaveCount(0);
    expect(
      await page.frameLocator('iframe').locator('body').getAttribute('data-presses'),
    ).toBeNull();
    await opener.click();
    // Compact drawers cover the frame; their exposed header is the dismissal target.
    if (width > 640) await page.mouse.click(iframe!.x + 30, iframe!.y + 50);
    else await page.mouse.click(header!.x + header!.width / 2, header!.y + header!.height / 2);
    await expect(drawer).toBeHidden();
    expect(
      await page.frameLocator('iframe').locator('body').getAttribute('data-presses'),
    ).toBeNull();
    await page.getByRole('button', { name: 'Bottom action' }).click();
    const bottom = page.getByRole('menu', { name: 'Bottom action' });
    const bounds = await bottom.boundingBox();
    const anchor = await page.getByRole('button', { name: 'Bottom action' }).boundingBox();
    expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(anchor!.y + 1);
    await page.screenshot({ path: testInfo.outputPath(`bottom-menu-${width}-${theme}.png`) });
    await bottom.press('Escape');
    await expect(page.getByRole('button', { name: 'Bottom action' })).toBeFocused();
    // Native closure must synchronize visibility even when focus escapes to the body.
    await opener.click();
    await drawer.getByRole('button', { name: /^Choice:/ }).click();
    await page.evaluate(() => {
      (document.activeElement as HTMLElement).blur();
    });
    expect(await page.evaluate(() => document.activeElement?.tagName)).toBe('BODY');
    await page.keyboard.press('Escape');
    await expect(menu).toHaveCount(0);
    await expect(drawer).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(drawer).toBeHidden();
    await expect(opener).toBeFocused();
    await opener.click();
    await expect(drawer).toBeVisible();
    await expect(draft).toHaveValue('Keep this draft');
    await drawer.evaluate((node: HTMLDialogElement) => node.close());
    await expect(drawer).toBeHidden();
    await opener.click();
    await expect(drawer).toBeVisible();
    const heading = await drawer.locator('h2').boundingBox();
    const caption = await drawer.locator('.tmt-ui-drawer-caption').boundingBox();
    expect(caption!.y).toBeLessThan(heading!.y + heading!.height);
    await drawer.press('Escape');
    await expect(page.locator('.tmt-ui-icon-action-tooltip:popover-open')).toHaveCount(0);
  });
}
