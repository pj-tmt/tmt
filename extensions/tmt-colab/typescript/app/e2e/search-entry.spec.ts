import { expect, test, type Page } from '@playwright/test';
import { join } from 'node:path';

test.beforeEach(async ({ page }) => {
  await page.route('**/src/main.tsx', (route) =>
    route.fulfill({ contentType: 'text/javascript', body: '' }),
  );
});

const fixture = '/test/search-entry-browser.tsx';
async function mount(page: Page, mode: 'ready' | 'cold-failed' | 'no-colab-data' = 'ready') {
  await page.goto('/');
  await page.evaluate(
    async ([path, mode]) => {
      await (await import(/* @vite-ignore */ path!)).mount(mode);
    },
    [fixture, mode],
  );
  await expect(page.locator('#search-opener')).toBeVisible();
}
const call = (page: Page, name: string, ...args: unknown[]) =>
  page.evaluate(
    async ([path, name, args]) => {
      const module = await import(/* @vite-ignore */ path as string);
      return module[name as string](...(args as unknown[]));
    },
    [fixture, name, args] as const,
  );
const proof = (page: Page) =>
  call(page, 'proof') as Promise<{ reads: number; went: string[]; navigated: string[] }>;
const opener = (page: Page) => page.locator('#search-opener');
const dialog = (page: Page) => page.getByRole('dialog', { name: 'Search TMT', exact: true });
const options = (page: Page) => dialog(page).getByRole('option');
const input = (page: Page) => dialog(page).getByRole('combobox', { name: 'Search TMT' });
const names = (page: Page) =>
  options(page).evaluateAll((nodes) =>
    nodes.map((node) => node.querySelector('.quick-switcher-title')!.textContent),
  );
const active = (page: Page) =>
  page.evaluate(() => {
    const field = document.querySelector('dialog [role="combobox"]')!;
    const id = field.getAttribute('aria-activedescendant');
    return {
      title: id
        ? document.getElementById(id)?.querySelector('.quick-switcher-title')?.textContent
        : null,
      marked: [...document.querySelectorAll('dialog [role="option"][data-active="true"]')].map(
        (node) => node.querySelector('.quick-switcher-title')?.textContent,
      ),
    };
  });

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'])
    test(`opens off the Colab route with the groups in order ${width} ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.addInitScript((value) => localStorage.setItem('tmt-ui-theme', value), theme);
      await mount(page);
      await page.evaluate((value) => (document.documentElement.dataset.theme = value), theme);
      await opener(page).click();
      await expect(dialog(page)).toBeVisible();
      await expect(input(page)).toBeFocused();
      // No Colab route roots: the dialog is the only Colab surface on this page.
      await expect(page.locator('.tmt-ui-header')).toHaveCount(0);
      await expect(dialog(page).getByRole('group')).toHaveCount(3);
      await expect(dialog(page).getByRole('group', { name: 'Navigation' })).toBeVisible();
      await expect(dialog(page).getByRole('group', { name: 'Colab' })).toBeVisible();
      await expect(dialog(page).getByRole('group', { name: 'Ops' })).toBeVisible();
      // Remote has no admitted snapshot: no heading and no row, not a labelled empty.
      await expect(dialog(page).getByRole('group', { name: 'Remote' })).toHaveCount(0);
      // Headings are not options; one active descendant names the first option.
      expect(await names(page)).toEqual([
        'Home',
        'Colab pages',
        'Ops agents',
        'Digest',
        'Beta review',
        'Alpha brief',
        'Design notes',
        'review-agent',
      ]);
      expect(await active(page)).toEqual({ title: 'Home', marked: ['Home'] });
      await expect(dialog(page).locator('.quick-switcher-scope')).toHaveText('Colab:');
      // Each row draws the glyph its item names, and the arrow when the name is not one of the dialog's.
      const glyphs = await options(page).evaluateAll((nodes) =>
        nodes.map((node) => node.querySelector('svg')!.getAttribute('class')),
      );
      expect(new Set(glyphs.slice(0, 4)).size).toBe(4);
      await expect(page.locator('html')).toHaveJSProperty('scrollWidth', width);
      const geometry = await dialog(page).evaluate((node) => {
        const box = node.getBoundingClientRect();
        const list = node.querySelector('[role="listbox"]')!.getBoundingClientRect();
        return {
          width: Math.round(box.width),
          list: Math.round(list.height),
          inside: list.right <= box.right && list.left >= box.left,
        };
      });
      expect(geometry.inside).toBe(true);
      // About six rows before the list scrolls, capped by the viewport (56% of 900px).
      expect(geometry.list).toBeGreaterThan(450);
      expect(geometry.list).toBeLessThanOrEqual(504);
      if (width === 1440) expect(geometry.width).toBeLessThanOrEqual(590);
      if (process.env.TMT_SEARCH_CAPTURE_DIR) {
        await page.evaluate(() => document.fonts.ready);
        await page.screenshot({
          path: join(process.env.TMT_SEARCH_CAPTURE_DIR, `search-${width}-${theme}.png`),
        });
      }
      expect((await proof(page)).reads).toBe(1);
    });

test('one active option crosses the group boundaries and a provider item opens its route', async ({
  page,
}) => {
  await mount(page);
  await opener(page).click();
  await expect(input(page)).toBeFocused();
  for (const title of ['Colab pages', 'Ops agents', 'Digest', 'Beta review', 'Alpha brief']) {
    await page.keyboard.press('ArrowDown');
    expect(await active(page)).toEqual({ title, marked: [title] });
  }
  await page.keyboard.press('ArrowUp');
  expect(await active(page)).toEqual({ title: 'Beta review', marked: ['Beta review'] });
  await page.keyboard.press('End');
  expect(await active(page)).toEqual({ title: 'review-agent', marked: ['review-agent'] });
  await page.keyboard.press('Home');
  expect(await active(page)).toEqual({ title: 'Home', marked: ['Home'] });
  await page.keyboard.press('End');
  await page.keyboard.press('Enter');
  await expect(dialog(page)).toHaveCount(0);
  await expect(opener(page)).toBeFocused();
  expect(await proof(page)).toMatchObject({ went: ['/ops/#agents'], navigated: [] });
});

test('a Colab page opens through Colab navigation, never the host route', async ({ page }) => {
  await mount(page);
  await opener(page).click();
  await options(page).filter({ hasText: 'Beta review' }).click();
  await expect(dialog(page)).toHaveCount(0);
  expect(await proof(page)).toMatchObject({
    went: [],
    navigated: ['c0000002-1111-4111-8111-000000000001'],
  });
  // A modified click keeps the real link: the page opens in a new tab, not through the host.
  await opener(page).click();
  await expect(options(page).filter({ hasText: 'Alpha brief' })).toHaveAttribute(
    'href',
    '/colab/p/c0000001-1111-4111-8111-000000000001',
  );
  await expect(options(page).filter({ hasText: 'Home' })).toHaveAttribute('href', '/home/');
});

test('the project filter scopes Colab only and the text names projects and modules', async ({
  page,
}) => {
  await mount(page);
  await opener(page).click();
  await input(page).fill('design');
  expect(await names(page)).toEqual(['Design notes', 'Alpha brief', 'design']);
  await expect(options(page).last().locator('.quick-switcher-meta')).toHaveText(
    'Project · 2 pages',
  );
  await options(page).last().click();
  expect((await proof(page)).went).toEqual(['/colab/?scope=design']);
  await opener(page).click();
  await dialog(page).getByRole('button', { name: 'Filter search by project', exact: true }).click();
  await page.getByRole('menuitemcheckbox', { name: 'ops', exact: true }).click();
  // Colab's group is scoped to the ops project; Navigation and Ops are not.
  expect(await names(page)).toEqual([
    'Home',
    'Colab pages',
    'Ops agents',
    'Digest',
    'Beta review',
    'review-agent',
  ]);
  await input(page).fill('ops');
  // Navigation first, then Colab's page and the project the text names; the Ops agent's text has no ops.
  expect(await names(page)).toEqual(['Ops agents', 'Beta review', 'ops']);
});

test('a provider that withdraws an item or its snapshot leaves the open list at once', async ({
  page,
}) => {
  await mount(page);
  await opener(page).click();
  await expect(dialog(page).getByRole('group', { name: 'Ops' })).toBeVisible();
  await call(page, 'removeOpsItem', 'review-agent');
  await expect(dialog(page).getByRole('group', { name: 'Ops' })).toHaveCount(0);
  expect(await names(page)).not.toContain('review-agent');
  await call(page, 'setRemote', [
    {
      key: 'mac',
      label: 'Studio Mac',
      detail: 'Device · 2 live sessions',
      icon: 'monitor-smartphone',
      href: '/remote/devices',
    },
  ]);
  await expect(dialog(page).getByRole('group', { name: 'Remote' })).toBeVisible();
  await page.keyboard.press('End');
  await page.keyboard.press('Enter');
  expect(await proof(page)).toMatchObject({ went: ['/remote/devices'] });
});

test('the groups stay when the pages cannot be read, and it says so beneath them', async ({
  page,
}) => {
  await mount(page, 'cold-failed');
  await opener(page).click();
  await expect(dialog(page).getByRole('group', { name: 'Navigation' })).toBeVisible();
  await expect(dialog(page).getByRole('group', { name: 'Colab' })).toHaveCount(0);
  await expect(dialog(page).getByRole('status')).toHaveText(
    "Couldn't load pages. Close and try again.",
  );
  await page.keyboard.press('Escape');
  await expect(dialog(page)).toHaveCount(0);
  await expect(opener(page)).toBeFocused();
});

test('disposal closes the dialog, adds no shortcut and leaves the entry inert', async ({
  page,
}) => {
  await mount(page);
  await page.keyboard.press('ControlOrMeta+k');
  await expect(dialog(page)).toHaveCount(0);
  await opener(page).click();
  await expect(dialog(page)).toBeVisible();
  await call(page, 'disposeEntryOnly');
  await expect(dialog(page)).toHaveCount(0);
  await expect(page.locator('#search-host')).toBeEmpty();
  await call(page, 'openDirect');
  await expect(dialog(page)).toHaveCount(0);
});
