import { writeFileSync } from 'node:fs';
import { expect, test, type Locator, type Page } from '@playwright/test';
import { capturePath } from './captures.js';

declare global {
  interface Window {
    __workPage?: import('../test/work-page-browser.js').WorkPageHook;
  }
}
/** The fixture's page IDs (`test/work-page-browser.tsx`). */
const id = (n: number) => `20000000-0000-4000-8000-${String(n).padStart(12, '0')}`;
const PAGE = {
  remote: id(1),
  invites: id(2),
  machines: id(3),
  pairing: id(12),
  dataPlan: id(51),
  made: id(99),
};

async function mount(page: Page, owner = true, scenario: 'tree' | 'empty' = 'tree', tall = false) {
  await page.goto('/');
  await page.evaluate(
    async ([owner, scenario, tall]) => {
      const path = '/test/work-page-browser.tsx';
      (await import(path)).mountWorkPage(owner, scenario, tall);
    },
    [owner, scenario, tall] as const,
  );
  await expect(page.locator('.page-header-actions:visible')).toBeVisible();
  const band = page.getByRole('region', { name: 'Work on this page' });
  await expect(band).toBeVisible();
  return band;
}
const head = (band: Locator) => band.locator('.work-head');
const toggle = (band: Locator) => band.locator('.work-toggle');
const row = (band: Locator, title: string) =>
  band
    .locator('.work-row-line')
    .filter({ has: band.page().getByRole('link', { name: title, exact: true }) });
const calls = (page: Page) => page.evaluate(() => window.__workPage!.calls);
/** A point of the head away from its chip and Add. */
const blank = (band: Locator) =>
  head(band).evaluate((node) => ({ x: node.clientWidth - 80, y: node.clientHeight / 2 }));
async function expand(band: Locator) {
  await expect(toggle(band)).toHaveAttribute('aria-expanded', 'false');
  await head(band).click({ position: await blank(band) });
  await expect(toggle(band)).toHaveAttribute('aria-expanded', 'true');
}
async function openAdd(page: Page, band: Locator) {
  await band.locator('.work-add-row').getByRole('button', { name: 'Add', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Add a sub-page to “Release plan”' });
  await expect(dialog).toBeVisible();
  return dialog;
}
const px = (value: string) => Number.parseFloat(value);

test('Mock 1: the collapsed head shows the page state and direct sub-page progress', async ({
  page,
}) => {
  const band = await mount(page);
  // Four direct sub-pages by their own states (2 done, 2 in progress).
  await expect(head(band)).toContainText('In progress');
  await expect(head(band)).toContainText('2 of 4 done');
  await expect(band.locator('.work-segments')).toHaveAttribute(
    'aria-label',
    'Done 2, In progress 2, To do 0, Unknown 0',
  );
  await expect(toggle(band)).toHaveCSS('cursor', 'pointer');
  // The chip and the toggle keep their own roles: no control is nested in another.
  await expect(band.locator('.work-head [role="button"]')).toHaveCount(0);
  await expect(toggle(band)).toHaveAccessibleName('Expand work panel');
  const before = await head(band).evaluate((node) => getComputedStyle(node).backgroundColor);
  await head(band).hover({ position: await blank(band) });
  await expect
    .poll(() => head(band).evaluate((node) => getComputedStyle(node).backgroundColor))
    .not.toBe(before);
});

test('the whole head toggles the list, by pointer or Enter/Space, but its state chip does not', async ({
  page,
}) => {
  const band = await mount(page);
  const width = await head(band).evaluate((node) => node.clientWidth);
  for (const x of [Math.round(width / 2), width - 20]) {
    await head(band).click({ position: { x, y: 22 } });
    await expect(toggle(band)).toHaveAttribute('aria-expanded', 'true');
    await expect(band.getByRole('list', { name: 'Sub-pages' })).toBeVisible();
    await head(band).click({ position: { x, y: 22 } });
    await expect(toggle(band)).toHaveAttribute('aria-expanded', 'false');
  }
  await band.getByRole('button', { name: 'Page state' }).click();
  await expect(page.getByRole('menuitemradio', { name: 'Done', exact: true })).toBeVisible();
  await expect(toggle(band)).toHaveAttribute('aria-expanded', 'false');
  await page.keyboard.press('Escape');
  await toggle(band).focus();
  await page.keyboard.press('Enter');
  await expect(toggle(band)).toHaveAttribute('aria-expanded', 'true');
  await page.keyboard.press(' ');
  await expect(toggle(band)).toHaveAttribute('aria-expanded', 'false');
});

test('Mock 2: with nothing attached there is no bar, a quiet line and Add; the row opens Add', async ({
  page,
}) => {
  const band = await mount(page, true, 'empty');
  await expect(head(band)).toContainText('No sub-pages yet');
  await expect(band.locator('.work-segments')).toHaveCount(0);
  await expect(toggle(band)).not.toHaveAttribute('aria-expanded');
  await expect(toggle(band)).toHaveAccessibleName('Add a sub-page');
  await head(band).click({ position: await blank(band) });
  const dialog = page.getByRole('dialog', { name: 'Add a sub-page to “Release plan”' });
  await expect(dialog).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  await head(band).getByRole('button', { name: 'Add', exact: true }).click();
  await expect(dialog).toBeVisible();
  expect(await calls(page)).toEqual([]);

  // A viewer who cannot add sees the line and no control.
  const shared = await mount(page, false, 'empty');
  await expect(head(shared)).toContainText('No sub-pages yet');
  await expect(toggle(shared)).toHaveCount(0);
  await expect(shared.getByRole('button', { name: 'Add', exact: true })).toHaveCount(0);
});

test('Mock 3: one list of sub-pages nests three levels in place, each with direct progress', async ({
  page,
}) => {
  const band = await mount(page);
  await expand(band);
  const tree = band.getByRole('list', { name: 'Sub-pages' });
  await expect(tree.locator(':scope > li')).toHaveCount(4);
  await expect(row(band, 'Remote pages v2').locator('.work-row-progress')).toHaveText('2 of 4');
  await expect(row(band, 'Release notes draft').locator('.work-row-progress')).toHaveText('—');
  await expect(row(band, 'Release notes draft').locator('button.work-twisty')).toHaveCount(0);
  await band.getByRole('button', { name: 'Expand Remote pages v2' }).click();
  await expect(row(band, 'Machines detail').locator('.work-row-progress')).toHaveText('1 of 1');
  await band.getByRole('button', { name: 'Expand Machines detail' }).click();
  await expect(row(band, 'Device list columns')).toBeVisible();
  // One level indents the title cell by 24px, and the right-hand columns line up at every level.
  const levels = ['Remote pages v2', 'Machines detail', 'Device list columns'];
  const box = async (title: string, part: string) =>
    (await row(band, title).locator(part).first().boundingBox())!;
  const [one, two, three] = await Promise.all(levels.map((title) => box(title, '.work-twisty')));
  expect(two.x - one.x).toBe(24);
  expect(three.x - two.x).toBe(24);
  for (const part of ['.work-row-progress', '.work-row-state', '.work-row-more']) {
    const boxes = await Promise.all(levels.map((title) => box(title, part)));
    expect(new Set(boxes.map((cell) => cell.x + cell.width)).size, part).toBe(1);
  }
  await expect(row(band, 'Remote pages v2').getByRole('link')).toHaveAttribute(
    'href',
    new RegExp(PAGE.remote),
  );
});

test('Mock 5: Add attaches a chosen page and keeps unavailable pages listed with why', async ({
  page,
}) => {
  const band = await mount(page);
  await expand(band);
  const dialog = await openAdd(page, band);
  const search = dialog.getByRole('combobox');
  await expect(search).toBeFocused();
  await search.fill('remote');
  const options = dialog.getByRole('option');
  await expect(options.first()).toHaveText('Create “remote” as a new sub-page');
  for (const [title, why] of [
    ['Remote pages v2', 'Already under Release plan'],
    ['Remote launch', 'Would loop: it is above this page'],
    ['Remote copy deck', 'Already under Remote launch'],
  ]) {
    const option = options.filter({
      has: page.locator('.work-add-title', { hasText: new RegExp(`^${title}$`) }),
    });
    await expect(option).toContainText(why);
    await expect(option).toHaveAttribute('aria-disabled', 'true');
  }
  await search.fill('Release plan');
  await expect(options.filter({ hasText: 'Release plan' }).first()).toContainText('This page');
  // An exact title match offers no Create row.
  await expect(options.filter({ hasText: 'Create “' })).toHaveCount(0);
  await search.fill('data plan');
  await options.filter({ hasText: 'Remote data plan design' }).click();
  await expect(dialog).toHaveCount(0);
  expect(await calls(page)).toEqual([
    { page: 'notes', action: 'subpage.attach', body: { pageId: PAGE.dataPlan } },
  ]);
  await expect(row(band, 'Remote data plan design')).toBeVisible();
  await expect(head(band)).toContainText('2 of 5 done');
});

test('Create makes a page named as typed; Enter picks the highlighted row, Esc closes', async ({
  page,
}) => {
  const band = await mount(page);
  await expand(band);
  let dialog = await openAdd(page, band);
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  dialog = await openAdd(page, band);
  await dialog.getByRole('combobox').fill('remote rollout');
  await page.keyboard.press('Enter');
  await expect(dialog).toHaveCount(0);
  expect(await calls(page)).toEqual([
    { page: 'notes', action: 'subpage.create', body: { title: 'remote rollout' } },
  ]);
  await expect(row(band, 'remote rollout').getByRole('link')).toHaveAttribute(
    'href',
    new RegExp(PAGE.made),
  );
});

test('a refused attach says why and keeps the window open, with nothing to clean up', async ({
  page,
}) => {
  const band = await mount(page);
  await expand(band);
  await page.evaluate(() => {
    window.__workPage!.refuse = {
      code: 'COLAB_LINK_REFUSED',
      message: 'Remote data plan design has Release plan below it.',
    };
  });
  const dialog = await openAdd(page, band);
  await dialog.getByRole('combobox').fill('data plan');
  const option = dialog.getByRole('option', { name: /Remote data plan design/ });
  await option.click();
  await expect(dialog.getByRole('alert')).toHaveText(
    'Remote data plan design has Release plan below it.',
  );
  expect((await calls(page)).map(({ action, body }) => [action, body])).toEqual([
    ['subpage.attach', { pageId: PAGE.dataPlan }],
  ]);
  // An outcome nobody knows is not sent again and leaves nothing to clean up.
  await page.evaluate(() => {
    window.__workPage!.unknownAttach = true;
  });
  await option.click();
  await expect(dialog.getByRole('alert')).toHaveText(
    'The result is unknown. Check the sub-pages before trying again.',
  );
  expect(await calls(page)).toHaveLength(2);
});

test('Mock 4: the row menu opens a page or detaches it, here and on a deeper level', async ({
  page,
}) => {
  const band = await mount(page);
  await expand(band);
  const more = (title: string) => band.getByRole('button', { name: `More for ${title}` });
  await row(band, 'Invites tab').hover();
  await more('Invites tab').click();
  await expect(page.getByRole('menuitem')).toHaveText(['Open', 'Detach']);
  await page.getByRole('menuitem', { name: 'Detach' }).click();
  await expect(row(band, 'Invites tab')).toHaveCount(0);
  await band.getByRole('button', { name: 'Expand Remote pages v2' }).click();
  await row(band, 'Pairing flow').hover();
  await more('Pairing flow').click();
  await page.getByRole('menuitem', { name: 'Detach' }).click();
  await expect(row(band, 'Pairing flow')).toHaveCount(0);
  expect(await calls(page)).toEqual([
    { page: 'notes', action: 'subpage.detach', body: { pageId: PAGE.invites } },
    { page: PAGE.remote, action: 'subpage.detach', body: { pageId: PAGE.pairing } },
  ]);
  await row(band, 'Machines list').hover();
  await more('Machines list').click();
  await page.getByRole('menuitem', { name: 'Open' }).click();
  await expect(page).toHaveURL(new RegExp(PAGE.machines));
});

test("a sub-page's state chip changes that page's state, and progress follows", async ({
  page,
}) => {
  const band = await mount(page);
  await expand(band);
  await band.getByRole('button', { name: 'State of Machines list' }).click();
  await page.getByRole('menuitemradio', { name: 'Done', exact: true }).click();
  await expect(band.getByRole('button', { name: 'State of Machines list' })).toContainText('Done');
  await expect(head(band)).toContainText('3 of 4 done');
  expect(await calls(page)).toEqual([
    { page: PAGE.machines, action: 'page.state', body: { state: 'done' } },
  ]);
});

test('a viewer sees the same tree without write controls', async ({ page }) => {
  const band = await mount(page, false);
  await expand(band);
  await band.getByRole('button', { name: 'Expand Remote pages v2' }).click();
  await expect(row(band, 'Machines detail')).toBeVisible();
  await expect(band.getByRole('button', { name: /^More for / })).toHaveCount(0);
  await expect(band.getByRole('button', { name: /^State of / })).toHaveCount(0);
  await expect(band.getByRole('button', { name: 'Add', exact: true })).toHaveCount(0);
});

test('the band stays under the nav as the page scrolls, and only its list scrolls', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 420 });
  const band = await mount(page, true, 'tree', true);
  const top = (await head(band).boundingBox())!.y;
  await page.mouse.move(700, 380);
  await page.mouse.wheel(0, 1600);
  await expect.poll(async () => (await head(band).boundingBox())!.y).toBe(top);
  await expand(band);
  await band.getByRole('button', { name: 'Expand Remote pages v2' }).click();
  await band.getByRole('button', { name: 'Expand Machines detail' }).click();
  const body = band.locator('.work-body');
  await expect
    .poll(() => body.evaluate((node) => node.scrollHeight > node.clientHeight))
    .toBe(true);
  const shown = (await band.boundingBox())!;
  expect(shown.y + shown.height).toBeLessThanOrEqual(420);
  await body.evaluate((node) => node.scrollBy(0, 120));
  await expect.poll(() => body.evaluate((node) => node.scrollTop)).toBeGreaterThan(0);
  expect((await head(band).boundingBox())!.y).toBe(top);
});

test('opening animates with the motion tokens and is instant under reduced motion', async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  const band = await mount(page);
  const transition = band.locator('.work-body-transition');
  await expect(transition).toHaveCSS('transition-duration', /0\.18s/);
  // A frame part-way through opening is shorter than the open list.
  const heights = await transition.evaluate(async (node) => {
    node.closest('.work-band')!.querySelector<HTMLElement>('.work-toggle')!.click();
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    const mid = node.getBoundingClientRect().height;
    await new Promise((resolve) => setTimeout(resolve, 400));
    return { mid, end: node.getBoundingClientRect().height };
  });
  expect(heights.mid).toBeLessThan(heights.end);
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await expect(transition).toHaveCSS('transition-duration', /^0s/);
});

test('measured spacing matches the mock table at 1440 and 390', async ({ page }) => {
  const measured: Record<string, unknown> = {};
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    const band = await mount(page);
    await expand(band);
    await band.getByRole('button', { name: 'Expand Remote pages v2' }).click();
    const gutter = (
      await page.evaluate(() =>
        getComputedStyle(document.querySelector('.work-band')!).getPropertyValue(
          '--tmt-ui-header-padding-inline',
        ),
      )
    ).trim();
    const style = (locator: Locator) =>
      locator.first().evaluate((node) => {
        const s = getComputedStyle(node);
        return {
          height: s.height,
          paddingTop: s.paddingTop,
          paddingBottom: s.paddingBottom,
          paddingLeft: s.paddingLeft,
          paddingRight: s.paddingRight,
          gap: s.columnGap,
          marginTop: s.marginTop,
          borderTop: s.borderTopWidth,
          borderRadius: s.borderRadius,
          columns: s.gridTemplateColumns,
        };
      });
    const values = {
      gutter,
      head: await style(head(band)),
      body: await style(band.locator('.work-body-content')),
      list: await style(band.locator('.work-body-content > .work-tree')),
      top: await style(row(band, 'Remote pages v2')),
      topName: await style(row(band, 'Remote pages v2').locator('.work-row-name')),
      nestedName: await style(row(band, 'Machines detail').locator('.work-row-name')),
      add: await style(band.locator('.work-add-row')),
    };
    measured[width] = values;
    expect(values.head.height).toBe('44px');
    expect(values.head.paddingLeft).toBe(gutter);
    expect(values.head.paddingRight).toBe(gutter);
    expect(values.head.gap).toBe('12px');
    expect(values.body.paddingTop).toBe('4px');
    expect(values.body.paddingBottom).toBe('16px');
    expect(values.body.paddingLeft).toBe(gutter);
    expect(values.top.height).toBe('40px');
    expect(values.top.gap).toBe('8px');
    expect(values.top.paddingLeft).toBe('8px');
    expect(values.top.paddingRight).toBe('8px');
    expect(values.top.borderRadius).toBe('8px');
    expect(values.topName.gap).toBe('8px');
    expect(px(values.nestedName.paddingLeft) - px(values.topName.paddingLeft)).toBe(24);
    expect(values.add.marginTop).toBe('8px');
    expect(values.add.paddingTop).toBe('8px');
    expect(values.add.borderTop).toBe('1px');
    expect(values.list.columns.split(' ').at(-1)).toBe('32px');
    // Collapsed, the band is exactly its head and rule.
    await head(band).click({ position: await blank(band) });
    await expect.poll(async () => (await band.boundingBox())!.height).toBe(45);
    await head(band).click({ position: await blank(band) });
    if (width === 1440) {
      await expect(head(band).locator('.work-wide')).toBeVisible();
    } else {
      await expect(row(band, 'Remote pages v2').locator('.work-row-progress')).toBeHidden();
      await expect(row(band, 'Remote pages v2').locator('.work-row-more')).toHaveCSS(
        'opacity',
        '1',
      );
      await expect(head(band).locator('.work-narrow')).toHaveText('2/4');
    }
    const dialog = await openAdd(page, band);
    await dialog.getByRole('combobox').fill('remote');
    const content = page.locator('.work-add-dialog');
    const option = dialog.getByRole('option').first();
    measured[`${width}-dialog`] = {
      width: (await content.boundingBox())!.width,
      padding: await content.evaluate((node) => getComputedStyle(node).padding),
      search: await dialog.getByRole('combobox').evaluate((node) => getComputedStyle(node).height),
      option: await option.evaluate((node) => getComputedStyle(node).height),
    };
    if (width === 1440) expect((await content.boundingBox())!.width).toBe(440);
    await expect(content).toHaveCSS('padding', '16px');
    await expect(dialog.getByRole('combobox')).toHaveCSS('height', '36px');
    expect((await option.boundingBox())!.height).toBeGreaterThanOrEqual(40);
  }
  writeFileSync(capturePath('work-panel-spacing.json'), JSON.stringify(measured, null, 2));
});

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`captures Mocks 1-6 at ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
      const shot = (name: string) =>
        page.screenshot({ path: capturePath(`work-panel-${name}-${width}-${theme}.png`) });
      let band = await mount(page);
      await head(band).hover({ position: await blank(band) });
      await shot('mock1-collapsed');
      await expand(band);
      await band.getByRole('button', { name: 'Expand Remote pages v2' }).click();
      await row(band, 'Remote pages v2').hover();
      await shot('mock3-expanded');
      await row(band, 'Invites tab').hover();
      await band.getByRole('button', { name: 'More for Invites tab' }).click();
      await shot('mock4-row-menu');
      await page.keyboard.press('Escape');
      const dialog = await openAdd(page, band);
      await dialog.getByRole('combobox').fill('remote');
      await shot('mock5-add');
      band = await mount(page, true, 'empty');
      await expect(head(band)).toContainText('No sub-pages yet');
      await shot('mock2-empty');
    });
