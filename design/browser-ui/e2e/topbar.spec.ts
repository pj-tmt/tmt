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
      test(`${renderer} common top bar ${width} ${theme}: order, modal and search focus`, async ({
        page,
      }) => {
        await page.setViewportSize({ width, height: 992 });
        await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
        await page.goto(`/?frame=topbar&renderer=${renderer}&theme=${theme}`);
        const bar = page.locator('.tmt-ui-topbar');
        await expect(bar).toHaveCount(1);
        await expect(bar).toHaveCSS('height', width === 390 ? '56px' : '62px');
        const themeControl = bar.getByRole('button', { name: /^Theme:/ });
        const remote = bar.getByRole('button', { name: 'Remote: 6 live sessions', exact: true });
        const action = bar.getByRole('button', { name: 'Module action', exact: true });
        const status = bar.getByText('Live', { exact: true });
        const boxes = await Promise.all(
          [status, themeControl, remote, action].map((item) => item.boundingBox()),
        );
        for (let i = 1; i < boxes.length; i++)
          expect(boxes[i]!.x).toBeGreaterThanOrEqual(boxes[i - 1]!.x + boxes[i - 1]!.width);
        await expect(remote.locator('.tmt-ui-count')).toHaveText('6');
        await remote.click();
        const remoteDialog = page.getByRole('dialog', { name: 'Remote', exact: true });
        await expect(remoteDialog).toBeVisible();
        await expect(remoteDialog).toContainText('Remote: 6 live sessions');
        await remoteDialog.dispatchEvent('keydown', { key: 'Escape' });
        await expect(remoteDialog).toBeVisible();
        await expect(remoteDialog.getByRole('link', { name: 'Remote settings' })).toHaveAttribute(
          'href',
          '/settings',
        );
        await page.keyboard.press('Escape');
        await expect(remoteDialog).toBeHidden();
        await expect(remote).toBeFocused();
        await observe(page, 'zero');
        const zero = bar.getByRole('button', { name: 'Remote: no live sessions', exact: true });
        await expect(zero).toBeFocused();
        await expect(zero.locator('.tmt-ui-count')).toBeHidden();
        await observe(page, 'unknown');
        const unknown = bar.getByRole('button', {
          name: 'Remote: session count unconfirmed',
          exact: true,
        });
        await expect(unknown).toBeFocused();
        await expect(unknown.locator('.tmt-ui-count')).toBeHidden();
        await unknown.click();
        await expect(remoteDialog).toContainText('session count unconfirmed');
        await remoteDialog.getByRole('button', { name: 'Close' }).click();
        const search =
          width === 390
            ? bar.getByRole('button', { name: 'Search TMT', exact: true })
            : page
                .locator('nav.tmt-ui-rail')
                .getByRole('button', { name: 'Search TMT', exact: true });
        await search.click();
        const dialog = page.getByRole('dialog', { name: 'Search TMT', exact: true });
        await expect(dialog).toBeVisible();
        await expect(
          dialog.getByRole('button', { name: 'Colab pages', exact: true }),
        ).toBeVisible();
        await page.keyboard.press('Escape');
        await expect(search).toBeFocused();
        await page.keyboard.press(await shortcut(page));
        await dialog.getByLabel('Search', { exact: true }).fill('digest');
        await expect(dialog.getByRole('button', { name: 'Digest', exact: true })).toBeVisible();
        await page.keyboard.press('Escape');
        await expect(search).toBeFocused();
        const frames = await focusFrames(page, '.tmt-ui-topbar');
        expect(frames.checked).toBeGreaterThan(2);
        expect(frames.clipped).toEqual([]);
        expect(frames.doubled).toEqual([]);
        expect(await textAtSurfaceEdge(page, '.tmt-ui-topbar')).toEqual([]);
        expect((await controlRadii(page, '.tmt-ui-topbar')).off).toEqual([]);
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
    page.getByRole('button', { name: 'Remote: 6 live sessions', exact: true }),
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

test('global search lives above Home and remains beside navigation on phones', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/?frame=topbar&renderer=native');
  const rail = page.locator('nav.tmt-ui-rail');
  const search = rail.getByRole('button', { name: 'Search TMT', exact: true });
  const home = rail.locator('a[data-id="home"]');
  await expect(search).toBeVisible();
  const [searchBox, homeBox] = await Promise.all([search.boundingBox(), home.boundingBox()]);
  expect(searchBox!.y + searchBox!.height).toBeLessThan(homeBox!.y);
  await search.click();
  await expect(page.getByRole('dialog', { name: 'Search TMT', exact: true })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(search).toBeFocused();
  await page.setViewportSize({ width: 390, height: 900 });
  const bar = page.locator('.tmt-ui-topbar');
  const navigation = bar.getByRole('button', { name: 'Open navigation', exact: true });
  const mobileSearch = bar.getByRole('button', { name: 'Search TMT', exact: true });
  await expect(mobileSearch).toBeVisible();
  const [navBox, mobileBox] = await Promise.all([
    navigation.boundingBox(),
    mobileSearch.boundingBox(),
  ]);
  expect(mobileBox!.x).toBeGreaterThanOrEqual(navBox!.x + navBox!.width);
  expect(mobileBox!.x - navBox!.x - navBox!.width).toBeLessThanOrEqual(16);
});

for (const renderer of ['react', 'native', 'hosted'])
  test(`${renderer}: route title belongs to the top bar left group`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 900 });
    await page.goto(`/?frame=topbar&renderer=${renderer}`);
    await expect(
      page.locator('.tmt-ui-topbar-navigation').getByRole('heading', { level: 1 }),
    ).toHaveText('Shared top bar');
  });

// These are shared-slot examples of the approved route identities, not product-route adoption.
const routeTitles = [
  ['home', 'Home'],
  ['colab-list', 'pm'],
  ['colab-page', 'Night design: editor hints and history cleanup'],
  ['colab-settings', 'Settings'],
  ['remote', 'Remote'],
  ['remote-members', 'Members'],
  ['ops', 'Agents'],
  ['digest', 'Digest'],
  ['settings', 'General'],
  ['reader', 'Weekly plan'],
  ['pair', 'tmt'],
] as const;
for (const renderer of ['react', 'native', 'hosted'])
  for (const width of [1440, 390])
    for (const theme of ['light', 'dark'] as const)
      for (const [identity, title] of routeTitles)
        test(`${renderer} route identity ${identity} ${width} ${theme}`, async ({ page }, info) => {
          await page.setViewportSize({ width, height: 992 });
          await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
          await page.goto(
            `/?frame=topbar&renderer=${renderer}&identity=${identity}&theme=${theme}`,
          );
          const bar = page.locator('.tmt-ui-topbar');
          const left = bar.locator('.tmt-ui-topbar-navigation');
          const heading = left.getByRole('heading', { level: 1 });
          await expect(heading).toBeVisible();
          await expect(page.getByRole('heading', { level: 1 })).toHaveCount(1);
          const current = heading.locator('.tmt-ui-topbar-title-current');
          await expect(current).toHaveText(title);
          await expect(current).toHaveCSS('font-weight', '600');
          await expect(current).toHaveCSS('text-overflow', 'ellipsis');
          const right = bar.locator('.tmt-ui-topbar-controls');
          const [leftBox, headingBox, rightBox] = await Promise.all([
            left.boundingBox(),
            heading.boundingBox(),
            right.boundingBox(),
          ]);
          expect(headingBox!.x).toBeGreaterThanOrEqual(leftBox!.x);
          expect(headingBox!.x + headingBox!.width).toBeLessThanOrEqual(rightBox!.x);
          for (const prefix of await heading.locator('.tmt-ui-topbar-title-prefix').all()) {
            if (width <= 900) await expect(prefix).toBeHidden();
            else {
              await expect(prefix).toBeVisible();
              await expect(prefix).toHaveCSS('font-weight', '400');
            }
          }
          if (identity === 'colab-page') {
            const chips = left.locator('.tmt-ui-topbar-title-chips');
            if (width <= 900) {
              await expect(chips).toBeHidden();
              expect(await current.evaluate((node) => node.scrollWidth > node.clientWidth)).toBe(
                true,
              );
            } else await expect(chips).toBeVisible();
          }
          if (identity === 'colab-page' || identity === 'reader') {
            const author = left.getByText('by Ben', { exact: true });
            if (width <= 900) await expect(author).toBeHidden();
            else {
              await expect(author).toBeVisible();
              await expect(author).toHaveCSS(
                'color',
                await heading
                  .locator('.tmt-ui-topbar-title-prefix')
                  .evaluate((node) => getComputedStyle(node).color),
              );
              if (identity === 'colab-page') {
                const pin = left.getByRole('button', { name: 'Pin page', exact: true });
                expect((await author.boundingBox())!.x).toBeGreaterThan(
                  (await pin.boundingBox())!.x,
                );
              }
            }
          }
          if (width <= 900) {
            const navigation = left.getByRole('button', { name: 'Open navigation', exact: true });
            const search = left.getByRole('button', { name: 'Search TMT', exact: true });
            const [navBox, searchBox] = await Promise.all([
              navigation.boundingBox(),
              search.boundingBox(),
            ]);
            expect(searchBox!.x).toBeGreaterThanOrEqual(navBox!.x + navBox!.width);
            expect(headingBox!.x).toBeGreaterThanOrEqual(searchBox!.x + searchBox!.width);
          }
          await bar.screenshot({
            path: info.outputPath(`title-${identity}-${renderer}-${width}-${theme}.png`),
          });
        });

for (const width of [900, 901])
  test(`route title compact boundary ${width}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 992 });
    await page.goto('/?frame=topbar&renderer=react&identity=colab-page');
    const bar = page.locator('.tmt-ui-topbar');
    const prefix = bar.locator('.tmt-ui-topbar-title-prefix');
    const chips = bar.locator('.tmt-ui-topbar-title-chips');
    if (width === 900) {
      await expect(prefix).toBeHidden();
      await expect(chips).toBeHidden();
    } else {
      await expect(prefix).toBeVisible();
      await expect(chips).toBeVisible();
    }
  });
