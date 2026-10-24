import { expect, test, type Page } from '@playwright/test';
import { controlRadii, focusFrames, textAtSurfaceEdge } from './audits.js';

const shortcut = (page: Page) =>
  page
    .evaluate(() => /Mac|iPhone|iPad|iPod/.test(navigator.platform))
    .then((command) => (command ? 'Meta+k' : 'Control+k'));
const observe = (page: Page, value: string) =>
  page.evaluate((detail) => {
    window.dispatchEvent(new CustomEvent('topbar-fixture-status', { detail }));
  }, value);

for (const renderer of ['react', 'native', 'hosted'])
  for (const width of [1440, 390])
    for (const theme of ['light', 'dark'] as const)
      test(`${renderer} common top bar ${width} ${theme}: one owner, counts and search focus`, async ({
        page,
      }) => {
        await page.setViewportSize({ width, height: 992 });
        await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
        await page.goto(`/?frame=topbar&renderer=${renderer}&theme=${theme}`);
        const bar = page.locator('.tmt-ui-topbar');
        await expect(bar).toHaveCount(1);
        await expect(bar).toHaveCSS('height', width === 390 ? '56px' : '62px');
        await expect(bar.getByRole('heading')).toHaveCount(0);
        await expect(page.getByRole('heading', { level: 1 })).toHaveCount(1);
        await expect(bar.getByRole('button', { name: 'Module action', exact: true })).toHaveCount(
          1,
        );
        await expect(bar.getByRole('button', { name: /^Theme:/ })).toHaveCount(1);
        const search = bar.getByRole('button', { name: 'Search TMT', exact: true });
        const remote = bar.getByRole('link', { name: 'Remote: 6 live sessions', exact: true });
        await expect(remote).toHaveAttribute('href', '/settings');
        await expect(remote.locator('.tmt-ui-count')).toHaveText('6');
        await remote.focus();
        await observe(page, 'zero');
        await expect(remote).toHaveCount(0);
        const zero = bar.getByRole('link', { name: 'Remote: no live sessions', exact: true });
        await expect(zero).toBeFocused();
        await expect(zero.locator('.tmt-ui-count')).toBeHidden();
        await observe(page, 'unknown');
        const unknown = bar.getByRole('link', {
          name: 'Remote: session count unconfirmed',
          exact: true,
        });
        await expect(unknown).toBeFocused();
        await expect(unknown.locator('.tmt-ui-count')).toBeHidden();
        await expect(unknown).toHaveAttribute('href', '/settings');
        await page.keyboard.press('Tab');
        const frames = await focusFrames(page, '.tmt-ui-topbar');
        expect(frames.checked).toBeGreaterThan(2);
        expect(frames.clipped).toEqual([]);
        expect(frames.doubled).toEqual([]);
        expect(await textAtSurfaceEdge(page, '.tmt-ui-topbar')).toEqual([]);
        expect((await controlRadii(page, '.tmt-ui-topbar')).off).toEqual([]);
        await search.click();
        const dialog = page.getByRole('dialog', { name: 'Search TMT', exact: true });
        await expect(dialog).toBeVisible();
        await expect(
          dialog.getByRole('button', { name: 'Colab pages', exact: true }),
        ).toBeVisible();
        await expect(dialog.getByRole('button', { name: 'Ops timers', exact: true })).toBeVisible();
        await expect(
          dialog.getByRole('button', { name: 'Remote devices', exact: true }),
        ).toBeVisible();
        await page.keyboard.press('Escape');
        await expect(dialog).toBeHidden();
        await expect(search).toBeFocused();
        await page.keyboard.press(await shortcut(page));
        await expect(dialog).toBeVisible();
        await dialog.getByLabel('Search', { exact: true }).fill('digest');
        await expect(dialog.getByRole('button', { name: 'Digest', exact: true })).toBeVisible();
        await expect(dialog.getByRole('button', { name: 'Ops timers', exact: true })).toHaveCount(
          0,
        );
        await page.keyboard.press('Escape');
        await expect(search).toBeFocused();
        if (width === 390) {
          const nav = bar.getByRole('button', { name: 'Open navigation', exact: true });
          await nav.click();
          await expect(nav).toHaveAttribute('aria-expanded', 'true');
          await page.keyboard.press('Escape');
          await expect(nav).toHaveAttribute('aria-expanded', 'false');
        }
      });

for (const renderer of ['react', 'native', 'hosted']) {
  test(`${renderer}: theme uses the existing preference and System follows the device`, async ({
    page,
  }) => {
    await page.emulateMedia({ colorScheme: 'dark', reducedMotion: 'reduce' });
    await page.goto(`/?frame=topbar&renderer=${renderer}&theme=light`);
    const menu = page.getByRole('menu', { name: /^Theme:/ });
    await page.getByRole('button', { name: /^Theme:/ }).click();
    await menu.getByRole('menuitemradio', { name: 'Dark', exact: true }).click();
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
    expect(await page.evaluate(() => localStorage.getItem('tmt-ui-theme'))).toBe('dark');
    await page.getByRole('button', { name: 'Theme: dark', exact: true }).click();
    await expect(menu.getByRole('menuitemradio', { name: 'Dark', exact: true })).toHaveAttribute(
      'aria-checked',
      'true',
    );
    await menu.getByRole('menuitemradio', { name: 'System', exact: true }).click();
    await expect(page.locator('html')).not.toHaveAttribute('data-theme');
    expect(await page.evaluate(() => localStorage.getItem('tmt-ui-theme'))).toBe(null);
    await expect(page.getByRole('button', { name: 'Theme: system', exact: true })).toBeFocused();
  });

  test(`${renderer}: closed root releases common listeners and active menu`, async ({ page }) => {
    await page.goto(`/?frame=topbar&renderer=${renderer}`);
    await page.getByRole('button', { name: /^Theme:/ }).click();
    await expect(page.getByRole('menu', { name: /^Theme:/ })).toBeVisible();
    await page.evaluate(() => window.dispatchEvent(new Event('topbar-fixture-hide')));
    await expect(page.locator('.tmt-ui-topbar')).toHaveCount(0);
    await expect(page.locator('.tmt-ui-menu-surface[open]')).toHaveCount(0);
    await page.keyboard.press(await shortcut(page));
    await expect(page.getByRole('dialog', { name: 'Search TMT', exact: true })).toBeHidden();
    await expect(page.getByTestId('search-requests')).toHaveText('0');
  });
}

test('blocked storage preserves the current choice; untrusted Search opens nothing', async ({
  page,
}) => {
  await page.addInitScript(() =>
    Object.defineProperty(window, 'localStorage', {
      get() {
        throw new DOMException('Blocked', 'SecurityError');
      },
    }),
  );
  await page.goto('/?frame=topbar&renderer=native');
  const search = page.getByRole('button', { name: 'Search TMT', exact: true });
  await search.evaluate((node) => (node as HTMLButtonElement).click());
  await expect(page.getByTestId('search-requests')).toHaveText('0');
  await page.getByRole('button', { name: /^Theme:/ }).click();
  await page.getByRole('menuitemradio', { name: 'Dark', exact: true }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await expect(page.getByRole('button', { name: 'Theme: dark', exact: true })).toBeVisible();
});

test('host route transition releases the old module action and installs one common bar', async ({
  page,
}) => {
  await page.goto('/?frame=topbar&renderer=hosted');
  await page.getByRole('button', { name: 'Module action', exact: true }).click();
  await expect(page.getByTestId('module-actions')).toHaveText('1');
  await page.getByRole('button', { name: 'Search TMT', exact: true }).click();
  await page
    .getByRole('dialog', { name: 'Search TMT', exact: true })
    .getByRole('button', { name: 'Ops agents', exact: true })
    .click();
  await expect(
    page.getByRole('heading', { level: 1, name: 'Ops agents', exact: true }),
  ).toBeVisible();
  await expect(page.locator('.tmt-ui-topbar')).toHaveCount(1);
  await expect(page.getByRole('button', { name: 'Module action', exact: true })).toHaveCount(1);
  await page.getByRole('button', { name: 'Module action', exact: true }).click();
  await expect(page.getByTestId('module-actions')).toHaveText('2');
  await expect(
    page.getByRole('link', { name: 'Remote: 6 live sessions', exact: true }),
  ).toHaveCount(1);
});

test('a route load settling after root disposal cannot install a header or Search listener', async ({
  page,
}) => {
  await page.goto('/?frame=topbar&renderer=hosted&delay=true');
  await expect(page.locator('[data-native-topbar]')).toBeVisible();
  await expect(page.getByTestId('module-mounts')).toHaveText('0');
  await page.evaluate(() => window.dispatchEvent(new Event('topbar-fixture-hide')));
  await expect(page.locator('[data-native-topbar]')).toHaveCount(0);
  await page.evaluate(async () => {
    window.dispatchEvent(new Event('topbar-fixture-release'));
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
  });
  await expect(page.getByTestId('module-mounts')).toHaveText('0');
  await page.keyboard.press(await shortcut(page));
  await expect(page.getByTestId('search-requests')).toHaveText('0');
  await expect(page.locator('.tmt-ui-topbar')).toHaveCount(0);
});
