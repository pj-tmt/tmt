import { capturePath } from './captures.js';
import { expect, test, type Page } from '@playwright/test';
test.use({ actionTimeout: 5000 });
/** Automatic pins by default: only explicit pins become shortcuts, so the record starts empty. */
async function mount(
  page: Page,
  size: 'stages' | 'work' = 'stages',
  pinned: 'automatic' | string[] = 'automatic',
) {
  await page.goto('/');
  await page.evaluate(
    async ({ size, pinned }) => {
      const path = '/test/page-list-browser.tsx';
      await (await import(path)).mountPageList(size, '/', pinned);
    },
    { size, pinned },
  );
}
const record = (page: Page) =>
  page.evaluate(async () => {
    const path = '/test/page-list-browser.tsx';
    const { pageListSpace } = await import(path);
    return JSON.parse(localStorage.getItem(`tmt-colab-shortcuts-v1:${pageListSpace}`) ?? 'null');
  });
/** The sidebar group owns the Manage entry point. */
const rail = (page: Page) => page.locator('#list-fixture .tmt-ui-rail-sub');
const manageShortcuts = (page: Page) =>
  rail(page).getByRole('button', { name: 'Manage', exact: true });
const railShortcuts = (page: Page) => rail(page).locator('a[data-id^="shortcut:"]');
const selected = (page: Page) => rail(page).locator('[aria-current="page"]');
test('Save current filters, edit the shortcut, retain drafts on failed storage and keep owner writes untouched', async ({
  page,
}) => {
  await mount(page);
  await page.getByRole('button', { name: 'Filter', exact: true }).click();
  await page.getByRole('menuitemcheckbox', { name: 'Include archived', exact: true }).click();
  await page.getByRole('button', { name: 'Save as shortcut', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Save as shortcut', exact: true });
  await dialog.getByRole('textbox', { name: 'Name', exact: true }).fill('My work');
  await dialog.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(dialog).not.toBeVisible();
  const hero = page.locator('.page-list-hero:visible');
  await expect(page.locator('#list-fixture header').first()).toContainText('My work');
  await expect(page.locator('#list-fixture .page-list-search-box')).toContainText(
    'Include archived',
  );
  await expect(
    hero.getByRole('button', { name: 'Edit shortcut', exact: true }).locator('svg.lucide-pencil'),
  ).toBeVisible();
  await page.getByRole('button', { name: 'Edit shortcut', exact: true }).click();
  const edit = page.getByRole('dialog', { name: 'Edit shortcut', exact: true });
  await expect(edit.getByRole('textbox', { name: 'Name', exact: true })).toHaveValue('My work');
  await edit.getByRole('textbox', { name: 'Name', exact: true }).fill('Retained draft');
  await page.evaluate(() => {
    Storage.prototype.setItem = () => {
      throw new Error('quota');
    };
  });
  await edit.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(edit.getByRole('alert')).toContainText('could not');
  await expect(edit.getByRole('textbox', { name: 'Name', exact: true })).toHaveValue(
    'Retained draft',
  );
  await edit.getByRole('button', { name: 'Cancel', exact: true }).click();
  expect(
    await page.evaluate(() => [
      window.__projectManage,
      window.__projectRequests,
      window.__pageOperations,
    ]),
  ).toEqual([[], [], []]);
});
test('Manage creates and reorders explicit shortcuts and the side menu follows its order', async ({
  page,
}) => {
  await mount(page);
  await page.evaluate(async () => {
    const path = '/test/page-list-browser.tsx';
    await (await import(path)).mountPageList('stages', '/?scope=design', 'automatic');
  });
  await page.getByRole('button', { name: 'Save as shortcut', exact: true }).click();
  const save = page.getByRole('dialog', { name: 'Save as shortcut', exact: true });
  await save.getByRole('textbox', { name: 'Name', exact: true }).fill('Design');
  await save.getByRole('button', { name: 'Save', exact: true }).click();
  await manageShortcuts(page).click();
  const dialog = page.getByRole('dialog', { name: 'Manage shortcuts', exact: true });
  await expect(dialog.getByRole('button', { name: 'Design', exact: true })).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'design', exact: true })).toHaveAttribute(
    'aria-pressed',
    'true',
  );
  await dialog.getByRole('button', { name: 'New shortcut', exact: true }).click();
  await dialog.getByRole('textbox', { name: 'Name', exact: true }).fill('Everything');
  await dialog.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(dialog.getByRole('button', { name: 'Everything', exact: true })).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Everything', exact: true })).toHaveAttribute(
    'aria-keyshortcuts',
    'Alt+ArrowUp Alt+ArrowDown',
  );
  await dialog.getByRole('button', { name: 'Everything', exact: true }).press('Alt+ArrowUp');
  await expect(dialog.locator('.shortcut-list > li').first()).toContainText('Everything');
  await dialog
    .locator('.shortcut-list > li')
    .nth(1)
    .dragTo(dialog.locator('.shortcut-list > li').first());
  await expect(dialog.locator('.shortcut-list > li').first()).toContainText('Design');
  await dialog.getByRole('button', { name: 'Close Manage shortcuts', exact: true }).click();
  await expect(railShortcuts(page).locator('.tmt-ui-module-menu-label')).toHaveText([
    'Design',
    'Everything',
  ]);
  await expect(rail(page).locator('a[data-id^="project:"]')).toHaveCount(0);
});

test('saved shortcuts survive remount, propagate same-page changes and keep legacy pins untouched', async ({
  page,
}) => {
  await mount(page);
  await page.getByRole('button', { name: 'Save as shortcut', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Save as shortcut', exact: true });
  await dialog.getByRole('textbox', { name: 'Name', exact: true }).fill('Persistent');
  await dialog.getByRole('button', { name: 'Save', exact: true }).click();
  await mount(page);
  await manageShortcuts(page).click();
  const manage = page.getByRole('dialog', { name: 'Manage shortcuts', exact: true });
  await expect(manage.getByRole('button', { name: 'Persistent', exact: true })).toBeVisible();
  await page.evaluate(async () => {
    const path = '/src/shortcut-store.ts';
    const { ShortcutStore } = await import(path);
    new ShortcutStore((await import('/test/page-list-browser.tsx' as string)).pageListSpace).save({
      name: 'Another tab',
      search: {},
    });
  });
  await expect(manage.getByRole('button', { name: 'Another tab', exact: true })).toBeVisible();
  expect(await page.evaluate(() => window.__projectManage)).toEqual([]);
});
for (const width of [390, 1440])
  for (const theme of ['light', 'dark'] as const) {
    test(`shortcut surfaces ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await mount(page);
      await page.evaluate((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      await expect(
        page.getByRole('button', { name: 'Save as shortcut', exact: true }),
      ).toBeVisible();
      await page.mouse.move(width - 1, 899);
      await page.screenshot({ path: capturePath(`shortcuts-hero-${width}-${theme}.png`) });
      await page.getByRole('button', { name: 'Save as shortcut', exact: true }).click();
      const save = page.getByRole('dialog', { name: 'Save as shortcut', exact: true });
      await save.getByRole('textbox', { name: 'Name', exact: true }).fill('Design reviews');
      await expect(save).toBeVisible();
      await page.mouse.move(width - 1, 899);
      await page.screenshot({ path: capturePath(`shortcuts-save-${width}-${theme}.png`) });
      await save.getByRole('button', { name: 'Save', exact: true }).click();
      if (width === 390) await page.getByRole('button', { name: 'Open navigation' }).click();
      await manageShortcuts(page).click();
      const manage = page.getByRole('dialog', { name: 'Manage shortcuts', exact: true });
      if (width === 390)
        await manage.getByRole('button', { name: 'Design reviews', exact: true }).click();
      await expect(manage.getByRole('textbox', { name: 'Name', exact: true })).toHaveValue(
        'Design reviews',
      );
      await page.mouse.move(width - 1, 899);
      await page.screenshot({ path: capturePath(`shortcuts-manage-${width}-${theme}.png`) });
      expect(await manage.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(
        true,
      );
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
      await manage.getByRole('button', { name: 'Cancel', exact: true }).click();
      if (width === 390)
        await expect(page.getByRole('button', { name: 'Open navigation' })).toBeFocused();
      else await expect(manageShortcuts(page)).toBeFocused();
    });
  }

test('Clear filters also clears manual state and include archived', async ({ page }) => {
  await mount(page);
  await page.getByRole('button', { name: 'Filter', exact: true }).click();
  await page.getByRole('menuitemcheckbox', { name: 'To do', exact: true }).click();
  await page.getByRole('button', { name: 'Filter', exact: true }).click();
  await page.getByRole('menuitemcheckbox', { name: 'Include archived', exact: true }).click();
  await page.getByRole('button', { name: 'Clear filters', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Remove To do' })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Remove Include archived' })).toHaveCount(0);
});

test('another tab receives storage updates; a separate browser profile has no saved shortcut', async ({
  page,
  browser,
}) => {
  await mount(page);
  await manageShortcuts(page).click();
  const peer = await page.context().newPage();
  await peer.goto('/');
  await peer.evaluate(async () => {
    const path = '/src/shortcut-store.ts';
    const { ShortcutStore } = await import(path);
    new ShortcutStore((await import('/test/page-list-browser.tsx' as string)).pageListSpace).save({
      name: 'From other tab',
      search: { needs: true },
    });
  });
  await expect(
    page
      .getByRole('dialog', { name: 'Manage shortcuts', exact: true })
      .getByRole('button', { name: 'From other tab', exact: true }),
  ).toBeVisible();
  await peer.close();
  const isolated = await browser.newContext();
  try {
    const fresh = await isolated.newPage();
    await mount(fresh);
    await manageShortcuts(fresh).click();
    await expect(fresh.getByRole('button', { name: 'From other tab', exact: true })).toHaveCount(0);
  } finally {
    await isolated.close();
  }
});

test('sticky chrome never transfers shortcut focus into New page', async ({ page }) => {
  await mount(page, 'work');
  const home = page.locator('#list-fixture [data-page-list]');
  const save = home.getByRole('button', { name: 'Save as shortcut', exact: true });
  await save.focus();
  await home.evaluate((node) => {
    node.scrollTop = 520;
  });
  await expect(home).toHaveAttribute('data-compact', '');
  await expect(save).toBeFocused();
  await home.evaluate((node) => {
    node.scrollTop = 0;
  });
  await expect(home).not.toHaveAttribute('data-compact');
  await expect(save).toBeFocused();
});

test('an external deletion does not remount an active Edit draft', async ({ page }) => {
  await mount(page);
  await page.getByRole('button', { name: 'Save as shortcut', exact: true }).click();
  let dialog = page.getByRole('dialog', { name: 'Save as shortcut', exact: true });
  await dialog.getByRole('textbox', { name: 'Name', exact: true }).fill('Original');
  await dialog.getByRole('button', { name: 'Save', exact: true }).click();
  await page.getByRole('button', { name: 'Edit shortcut', exact: true }).click();
  dialog = page.getByRole('dialog', { name: 'Edit shortcut', exact: true });
  await dialog.getByRole('textbox', { name: 'Name', exact: true }).fill('Unsaved draft');
  const peer = await page.context().newPage();
  try {
    await peer.goto('/');
    await peer.evaluate(async () => {
      const path = '/src/shortcut-store.ts';
      const { ShortcutStore } = await import(path);
      const store = new ShortcutStore(
        (await import('/test/page-list-browser.tsx' as string)).pageListSpace,
      );
      const item = store.read().items.find((item: { name: string }) => item.name === 'Original');
      if (!item) throw new Error('Saved shortcut missing');
      store.delete(item.id);
    });
    await expect(dialog.getByRole('textbox', { name: 'Name', exact: true })).toHaveValue(
      'Unsaved draft',
    );
    await dialog.getByRole('button', { name: 'Save', exact: true }).click();
    await expect(dialog.getByRole('alert')).toContainText('could not');
    await expect(dialog.getByRole('textbox', { name: 'Name', exact: true })).toHaveValue(
      'Unsaved draft',
    );
  } finally {
    await peer.close();
  }
});

test('Save uses the effective list scope when a removed project falls back to All pages', async ({
  page,
}) => {
  await mount(page);
  await page.evaluate(async () => {
    const path = '/test/page-list-browser.tsx';
    await (await import(path)).mountPageList('stages', '/?scope=gone', 'automatic');
  });
  await page.getByRole('button', { name: 'Save as shortcut', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Save as shortcut', exact: true });
  await expect(dialog.getByRole('status')).toHaveText('5 pages');
  await dialog.getByRole('textbox', { name: 'Name', exact: true }).fill('Current view');
  await dialog.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.locator('#list-fixture .pages [data-page-id]')).toHaveCount(5);
});

test('explicit pinned projects become project shortcuts once, in order; automatic ones do not', async ({
  page,
}) => {
  await mount(page, 'stages', ['ops', 'design']);
  await expect(railShortcuts(page).locator('.tmt-ui-module-menu-label')).toHaveText([
    'ops',
    'design',
  ]);
  await expect(rail(page).locator('a[data-id^="project:"]')).toHaveCount(0);
  const migrated = await record(page);
  expect(migrated.migrated).toBe(true);
  expect(
    migrated.items.map((item: { name: string; search: unknown }) => [item.name, item.search]),
  ).toEqual([
    ['ops', { project: ['ops'] }],
    ['design', { project: ['design'] }],
  ]);
  // Once only: a later load, even with other pins, adds nothing, and the owner's pins are not written.
  await mount(page, 'stages', ['design', 'q4 plan']);
  await expect(railShortcuts(page).locator('.tmt-ui-module-menu-label')).toHaveText([
    'ops',
    'design',
  ]);
  expect(await page.evaluate(() => window.__projectManage)).toEqual([]);
  // A migrated shortcut opens the list on its project.
  await railShortcuts(page).first().click();
  await expect(page).toHaveURL(/shortcut=/);
  await expect(page).toHaveURL(/project=/);
  await expect(railShortcuts(page).first()).toHaveAttribute('aria-current', 'page');
  // Automatic recent projects are suggestions, not choices: none migrates.
  await page.evaluate(() => localStorage.clear());
  await mount(page);
  await expect(railShortcuts(page)).toHaveCount(0);
  expect(await record(page)).toEqual({ version: 1, migrated: true, items: [] });
});

test('a failed migration write leaves it to retry on the next load', async ({ page }) => {
  await page.goto('/');
  await page.evaluate(() => {
    const write = Storage.prototype.setItem;
    Storage.prototype.setItem = function (key, value) {
      if (key.startsWith('tmt-colab-shortcuts-v1:')) throw new Error('quota');
      return write.call(this, key, value);
    };
  });
  await page.evaluate(async () => {
    const path = '/test/page-list-browser.tsx';
    await (await import(path)).mountPageList('stages', '/', ['ops']);
  });
  await expect(rail(page).locator('a[data-id="all"]')).toBeVisible();
  await expect(railShortcuts(page)).toHaveCount(0);
  expect(await record(page)).toBeNull();
  await mount(page, 'stages', ['ops']);
  await expect(railShortcuts(page).locator('.tmt-ui-module-menu-label')).toHaveText(['ops']);
});

test('no shortcut record is read or written before the space is defined', async ({ page }) => {
  await page.goto('/');
  await page.evaluate(async () => {
    const calls: string[] = [];
    const read = Storage.prototype.getItem;
    const write = Storage.prototype.setItem;
    Storage.prototype.getItem = function (key) {
      if (key.startsWith('tmt-colab-shortcuts-v1:')) calls.push(`read:${key}`);
      return read.call(this, key);
    };
    Storage.prototype.setItem = function (key, value) {
      if (key.startsWith('tmt-colab-shortcuts-v1:')) calls.push(`write:${key}`);
      return write.call(this, key, value);
    };
    Object.assign(window, { shortcutStorageCalls: calls });
    const path = '/test/page-list-browser.tsx';
    await (await import(path)).mountPageList('stages', '/', ['design'], false);
  });
  await expect(page.getByRole('button', { name: 'Save as shortcut', exact: true })).toBeVisible();
  expect(
    await page.evaluate(
      () => (window as unknown as { shortcutStorageCalls: string[] }).shortcutStorageCalls,
    ),
  ).toEqual([]);
  await expect(page.getByRole('button', { name: 'Save as shortcut', exact: true })).toBeDisabled();
});

test('Manage uses shared field labels and multi-select page-state chips', async ({ page }) => {
  await mount(page);
  await manageShortcuts(page).click();
  const dialog = page.getByRole('dialog', { name: 'Manage shortcuts', exact: true });
  const labels = dialog.locator('.tmt-ui-field-label');
  await expect(labels).toHaveText(['Name', 'Projects', 'Page state', 'Also']);
  const positions = await labels.evaluateAll((nodes) =>
    nodes.map((node) => node.getBoundingClientRect().left),
  );
  expect(new Set(positions).size).toBe(1);
  await expect(dialog.locator('select')).toHaveCount(0);
  await expect(dialog.getByText('None selected means every project.')).toHaveCSS(
    'font-size',
    '12px',
  );
  await dialog.getByRole('button', { name: 'Done', exact: true }).click();
  await expect(dialog.getByRole('button', { name: 'Done', exact: true })).toHaveAttribute(
    'aria-pressed',
    'true',
  );
  await dialog.getByRole('button', { name: 'To do', exact: true }).click();
  await expect(dialog.getByRole('button', { name: 'To do', exact: true })).toHaveAttribute(
    'aria-pressed',
    'true',
  );
  await dialog.getByRole('textbox', { name: 'Name' }).fill('Two states');
  await dialog.getByRole('button', { name: 'Save', exact: true }).click();
  expect((await record(page)).items[0].search.state).toEqual(['done', 'todo']);
});

test('the side menu lists the shortcuts, selects exactly one row and opens the list dialogs', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await mount(page);
  await expect(railShortcuts(page)).toHaveCount(0);
  await expect(selected(page)).toHaveAttribute('data-id', 'all');
  // Saving starts from the list bookmark; the rail has no add row.
  await expect(rail(page).getByRole('button', { name: 'New shortcut', exact: true })).toHaveCount(
    0,
  );
  await page.getByRole('button', { name: 'Save as shortcut', exact: true }).click();
  const save = page.getByRole('dialog', { name: 'Save as shortcut', exact: true });
  await save.getByRole('textbox', { name: 'Name', exact: true }).fill('Mine');
  await save.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(railShortcuts(page).locator('.tmt-ui-module-menu-label')).toHaveText(['Mine']);
  // The list now shows the shortcut, so its row is the one selected and the bar names it.
  await expect(selected(page)).toHaveCount(1);
  await expect(railShortcuts(page)).toHaveAttribute('aria-current', 'page');
  await expect(page.locator('#list-fixture .tmt-ui-topbar-title-current')).toHaveText('Mine');
  // A page opened from that list keeps the shortcut selected.
  await page.locator('#list-fixture .pages [data-page-id] .tmt-ui-list-title > a').first().click();
  await expect(page).toHaveURL(/#\/pages\//);
  await expect(selected(page)).toHaveCount(1);
  await expect(railShortcuts(page)).toHaveAttribute('aria-current', 'page');
  // All pages opens the plain list and is the one selected row.
  await rail(page).locator('a[data-id="all"]').click();
  await expect(page).not.toHaveURL(/shortcut=/);
  await expect(selected(page)).toHaveCount(1);
  await expect(selected(page)).toHaveAttribute('data-id', 'all');
  await expect(page.locator('#list-fixture .tmt-ui-topbar-title-current')).toHaveText('All pages');
  // The shortcut row opens the list with it.
  await railShortcuts(page).click();
  await expect(page).toHaveURL(/shortcut=/);
  await expect(railShortcuts(page)).toHaveAttribute('aria-current', 'page');
  // From settings, the group's Manage opens the list first, then its dialog.
  await rail(page).locator('a[data-id="colab-settings"]').click();
  await expect(selected(page)).toHaveAttribute('data-id', 'colab-settings');
  await expect(selected(page)).toHaveCount(1);
  await rail(page).getByRole('button', { name: 'Manage', exact: true }).click();
  await expect(page.getByRole('dialog', { name: 'Manage shortcuts', exact: true })).toBeVisible();
  await expect(selected(page)).toHaveAttribute('data-id', 'all');
});

test('sidebar shortcut exposes honest counts and row controls without navigation', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await mount(page, 'work');
  const readsBefore = await page.evaluate(() => window.__listReads);
  await page.evaluate(async () => {
    const storePath = '/src/shortcut-store.ts';
    const fixturePath = '/test/page-list-browser.tsx';
    const { ShortcutStore } = await import(storePath);
    const { pageListSpace } = await import(fixturePath);
    new ShortcutStore(pageListSpace).save({ name: 'All work', search: {} });
  });
  const row = rail(page).locator('li[data-menu-item^="shortcut:"]');
  const link = row.getByRole('link', { name: /^All work/ });
  await expect(link.locator('.tmt-ui-shell-count')).toHaveText(['75']);
  await expect(link).toHaveAccessibleName('All work · 75 total, 1 need you');
  await expect(link.locator('.tmt-ui-shell-attention')).toHaveAttribute('aria-hidden', 'true');
  await row.hover();
  await expect(row.getByRole('button', { name: 'Edit All work' })).toBeVisible();
  await expect(row.getByRole('button', { name: 'Delete All work' })).toBeVisible();
  await expect(row.getByRole('button', { name: 'Reorder All work' })).toBeVisible();
  await expect(page).not.toHaveURL(/shortcut=/);
  expect(await page.evaluate(() => window.__listReads)).toBe(readsBefore);
});

test('sidebar edit, delete and keyboard reorder keep shortcut identity and focus', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await mount(page, 'work');
  await page.evaluate(async () => {
    const storePath = '/src/shortcut-store.ts';
    const fixturePath = '/test/page-list-browser.tsx';
    const { ShortcutStore } = await import(storePath);
    const { pageListSpace } = await import(fixturePath);
    const store = new ShortcutStore(pageListSpace);
    store.save({ name: 'First', search: {} });
    store.save({ name: 'Second', search: { project: ['spec'] } });
    store.save({ name: 'Third', search: { project: ['blog'] } });
  });
  const row = (name: string) =>
    rail(page)
      .locator('li[data-menu-item^="shortcut:"]')
      .filter({ has: page.getByText(name, { exact: true }) });
  await expect(railShortcuts(page)).toHaveCount(3);
  await railShortcuts(page).filter({ hasText: 'First' }).click();
  await page.locator('#list-fixture .pages [data-page-id] .tmt-ui-list-title > a').first().click();
  await expect(page).toHaveURL(/#\/pages\//);
  await row('Third').hover();
  await row('Third').getByRole('button', { name: 'Edit Third' }).click();
  const edit = page.getByRole('dialog', { name: 'Edit shortcut', exact: true });
  await expect(edit.getByRole('textbox', { name: 'Name' })).toHaveValue('Third');
  await expect(edit.getByRole('button', { name: 'blog', exact: true })).toHaveAttribute(
    'aria-pressed',
    'true',
  );
  await edit.getByRole('button', { name: 'Cancel', exact: true }).click();
  await railShortcuts(page).filter({ hasText: 'First' }).click();
  await row('First').hover();
  await row('First').getByRole('button', { name: 'Delete First' }).click();
  await expect(page).not.toHaveURL(/shortcut=/);
  await expect(railShortcuts(page).locator('.tmt-ui-module-menu-label')).toHaveText([
    'Second',
    'Third',
  ]);
  await expect(row('Second').getByRole('link', { name: 'Second' })).toBeFocused();
  await row('Second').getByRole('link', { name: 'Second' }).press('Alt+ArrowDown');
  await expect(railShortcuts(page).locator('.tmt-ui-module-menu-label')).toHaveText([
    'Third',
    'Second',
  ]);
  expect((await record(page)).items.map((item: { name: string }) => item.name)).toEqual([
    'Third',
    'Second',
  ]);
  await row('Second').hover();
  await row('Second').getByRole('button', { name: 'Delete Second' }).click();
  await expect(row('Third').getByRole('link', { name: 'Third' })).toBeFocused();
  await row('Third').hover();
  await row('Third').getByRole('button', { name: 'Delete Third' }).click();
  await expect(rail(page).getByRole('button', { name: 'Manage' })).toBeFocused();
});

test('a failed sidebar delete keeps the shortcut and reports the failure', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await mount(page, 'work');
  await page.evaluate(async () => {
    const storePath = '/src/shortcut-store.ts';
    const fixturePath = '/test/page-list-browser.tsx';
    const { ShortcutStore } = await import(storePath);
    const { pageListSpace } = await import(fixturePath);
    new ShortcutStore(pageListSpace).save({ name: 'Keep me', search: {} });
  });
  const row = rail(page).locator('li[data-menu-item^="shortcut:"]');
  await expect(row).toContainText('Keep me');
  await page.evaluate(() => {
    Storage.prototype.setItem = () => {
      throw new Error('quota');
    };
  });
  await row.hover();
  await row.getByRole('button', { name: 'Delete Keep me' }).click();
  await expect(page.getByRole('alert')).toContainText('could not be saved');
  await expect(row).toContainText('Keep me');
  expect((await record(page)).items.map((item: { name: string }) => item.name)).toEqual([
    'Keep me',
  ]);
});

test('unknown sidebar counts appear only after the required summaries arrive', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/');
  await page.evaluate(async () => {
    const fixturePath = '/test/page-list-browser.tsx';
    await (await import(fixturePath)).mountPageList('cold', '/', 'automatic');
    const storePath = '/src/shortcut-store.ts';
    const { ShortcutStore } = await import(storePath);
    const { pageListSpace } = await import(fixturePath);
    new ShortcutStore(pageListSpace).save({ name: 'Working', search: { stage: 'in-progress' } });
  });
  const row = railShortcuts(page).filter({ hasText: 'Working' });
  await expect(row).toBeVisible();
  await expect(row.locator('.tmt-ui-shell-count')).toHaveCount(0);
  await page.evaluate(() => window.__reportTasks?.());
  await expect(row.locator('.tmt-ui-shell-count')).toHaveText(['1'], { timeout: 10000 });
});

test('sidebar handle drag persists a move to the target index', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await mount(page, 'work');
  await page.evaluate(async () => {
    const storePath = '/src/shortcut-store.ts';
    const fixturePath = '/test/page-list-browser.tsx';
    const { ShortcutStore } = await import(storePath);
    const { pageListSpace } = await import(fixturePath);
    const store = new ShortcutStore(pageListSpace);
    for (const name of ['First', 'Second', 'Third']) store.save({ name, search: {} });
  });
  await expect(railShortcuts(page)).toHaveCount(3);
  const row = (name: string) =>
    rail(page)
      .locator('li[data-menu-item^="shortcut:"]')
      .filter({ has: page.getByText(name, { exact: true }) });
  await row('First').hover();
  await row('First').getByRole('button', { name: 'Reorder First' }).dragTo(row('Third'));
  await expect(railShortcuts(page).locator('.tmt-ui-module-menu-label')).toHaveText([
    'Second',
    'Third',
    'First',
  ]);
  expect((await record(page)).items.map((item: { name: string }) => item.name)).toEqual([
    'Second',
    'Third',
    'First',
  ]);
});

for (const width of [390, 1440])
  for (const theme of ['light', 'dark'] as const)
    test(`sidebar shortcut controls and captures ${width} ${theme}`, async ({ page }, info) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((value) => {
        document.documentElement.dataset.theme = value;
      }, theme);
      await mount(page, 'work');
      await page.evaluate(async () => {
        const storePath = '/src/shortcut-store.ts';
        const fixturePath = '/test/page-list-browser.tsx';
        const { ShortcutStore } = await import(storePath);
        const { pageListSpace } = await import(fixturePath);
        const store = new ShortcutStore(pageListSpace);
        store.save({ name: 'All work', search: {} });
        store.save({ name: 'Spec', search: { project: ['spec'] } });
      });
      if (width === 390) await page.getByRole('button', { name: 'Open navigation' }).click();
      const row = rail(page)
        .locator('li[data-menu-item^="shortcut:"]')
        .filter({ has: page.getByText('All work', { exact: true }) });
      await expect(row).toBeVisible();
      await page.screenshot({ path: info.outputPath('idle.png') });
      await row.hover();
      await page.screenshot({ path: info.outputPath('hover.png') });
      await page.mouse.move(width - 1, 899);
      await row.getByRole('link').focus();
      await expect(row.getByRole('button', { name: 'Edit All work' })).toBeVisible();
      await page.screenshot({ path: info.outputPath('focus.png') });
      await row.getByRole('link').press('Alt+ArrowDown');
      await expect(railShortcuts(page).locator('.tmt-ui-module-menu-label')).toHaveText([
        'Spec',
        'All work',
      ]);
      await page.screenshot({ path: info.outputPath('reordered.png') });
      await railShortcuts(page)
        .filter({ has: page.getByText('Spec', { exact: true }) })
        .click();
      if (width === 390) await page.getByRole('button', { name: 'Open navigation' }).click();
      await expect(
        railShortcuts(page).filter({ has: page.getByText('Spec', { exact: true }) }),
      ).toHaveAttribute('aria-current', 'page');
      await page.screenshot({ path: info.outputPath('selected.png') });
      await row.getByRole('link').focus();
      await row.getByRole('button', { name: 'Edit All work' }).click();
      await expect(page.getByRole('dialog', { name: 'Edit shortcut' })).toBeVisible();
    });

test('shortcut save action and preview follow the page-list mock', async ({ page }) => {
  await mount(page);
  const hero = page.locator('.page-list-hero:visible');
  const save = hero.getByRole('button', { name: 'Save as shortcut', exact: true });
  await expect(hero.getByRole('heading')).toHaveCount(0);
  await expect(save.locator('svg.lucide-bookmark')).toBeVisible();
  await expect(hero.getByRole('button', { name: 'Manage shortcuts', exact: true })).toHaveCount(0);
  await expect(rail(page).getByRole('button', { name: 'Manage', exact: true })).toBeVisible();
  await save.click();
  const dialog = page.getByRole('dialog', { name: 'Save as shortcut', exact: true });
  await expect(dialog).toHaveCSS('width', '400px');
  await expect(dialog.getByRole('textbox', { name: 'Name', exact: true })).toBeVisible();
  await expect(dialog.locator('.shortcut-summary svg.lucide-funnel')).toBeVisible();
  await expect(
    dialog.getByText('It appears under Shortcuts in the sidebar. Change it later with Manage.'),
  ).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Cancel', exact: true })).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Save', exact: true })).toBeVisible();
});

test('Manage shortcut rows, project and Also chips, and preview follow the mock', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await mount(page);
  await page.getByRole('button', { name: 'Save as shortcut', exact: true }).click();
  const save = page.getByRole('dialog', { name: 'Save as shortcut', exact: true });
  await save.getByRole('textbox', { name: 'Name', exact: true }).fill('My filter');
  await save.getByRole('button', { name: 'Save', exact: true }).click();
  await manageShortcuts(page).click();
  const dialog = page.getByRole('dialog', { name: 'Manage shortcuts', exact: true });
  await expect(dialog).toHaveCSS('width', '640px');
  await expect(dialog.getByRole('navigation', { name: 'Saved shortcuts' })).toHaveCSS(
    'width',
    '220px',
  );
  const selectedRow = dialog.locator('.shortcut-list > li').first();
  await expect(selectedRow.getByRole('img', { name: 'Drag to reorder' })).toBeVisible();
  await expect(selectedRow.locator('svg.lucide-grip-vertical')).toBeVisible();
  await expect(selectedRow).toContainText('My filter');
  await expect(selectedRow.getByRole('button', { name: 'My filter' })).toHaveAttribute(
    'aria-current',
    'true',
  );
  await expect(selectedRow.locator('[data-match-count]')).toHaveText(/^[0-9]+$/);
  await expect(dialog.getByRole('button', { name: 'Move My filter up' })).toHaveCount(0);
  const project = dialog.getByRole('button', { name: 'design', exact: true });
  await expect(project).toHaveAttribute('aria-pressed', 'false');
  await project.click();
  await expect(project).toHaveAttribute('aria-pressed', 'true');
  await expect(dialog.getByText('None selected means every project.')).toBeVisible();
  const needs = dialog.getByRole('button', { name: 'Needs me', exact: true });
  await expect(needs).toHaveAttribute('aria-pressed', 'false');
  await needs.click();
  await expect(needs).toHaveAttribute('aria-pressed', 'true');
  await expect(dialog.locator('.shortcut-summary svg.lucide-funnel')).toBeVisible();
  await expect(dialog.getByRole('status')).toContainText(/^Matches [0-9]+ pages/);
  await dialog.getByRole('button', { name: 'Save', exact: true }).click();
  expect((await record(page)).items[0].search).toMatchObject({ project: ['design'], needs: true });
  const remove = dialog.getByRole('button', { name: 'Delete', exact: true });
  await expect(remove).toHaveAttribute('data-variant', 'ghost');
  const cancel = dialog.getByRole('button', { name: 'Cancel', exact: true });
  expect((await remove.boundingBox())!.x).toBeLessThan((await cancel.boundingBox())!.x);
  await remove.click();
  await expect(dialog.getByRole('button', { name: 'My filter', exact: true })).toHaveCount(0);
  expect((await record(page)).items).toHaveLength(0);
});

for (const width of [390, 1440])
  for (const theme of ['light', 'dark'] as const)
    test(`approved page list actions and filter chips ${width} ${theme}`, async ({
      page,
    }, info) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await mount(page);
      const hero = page.locator('.page-list-hero:visible');
      await expect(hero.getByRole('heading')).toHaveCount(0);
      await expect(hero).not.toContainText('Pages, decisions and conversations');
      await expect(
        hero.getByRole('button', { name: 'Save as shortcut', exact: true }),
      ).toBeVisible();
      await expect(hero.getByRole('button', { name: /New page/i })).toBeVisible();
      const filter = page.getByRole('button', { name: 'Filter', exact: true });
      if (width === 390) {
        await expect(page.locator('#list-fixture .tmt-ui-tab-list')).toHaveCSS(
          'flex-wrap',
          'nowrap',
        );
        const sort = page.getByRole('button', { name: 'Sort: Recently updated', exact: true });
        await expect(sort).toBeVisible();
        const sortBox = (await sort.boundingBox())!;
        const box = (await filter.boundingBox())!;
        expect(Math.abs(sortBox.y - box.y)).toBeLessThanOrEqual(1);
        await expect(page.getByRole('combobox', { name: 'Search pages' })).toHaveAttribute(
          'data-placeholder',
          'Search pages…',
        );
        expect(box.x + box.width).toBeLessThanOrEqual(width);
        expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
          width,
        );
      }
      await filter.click();
      await page.getByRole('menuitemcheckbox', { name: 'To do', exact: true }).click();
      await filter.click();
      await page.getByRole('menuitemcheckbox', { name: 'Done', exact: true }).click();
      await expect(page.locator('#list-fixture .page-list-search-box')).toContainText(
        'state: To do',
      );
      await expect(page.locator('#list-fixture .page-list-search-box')).toContainText(
        'state: Done',
      );
      const chipBoxes = await page
        .locator('#list-fixture .page-list-condition-chips')
        .evaluate((host) => {
          const boundary = host.getBoundingClientRect();
          return [...host.querySelectorAll<HTMLElement>('.tmt-ui-chip')].map((chip) => {
            const box = chip.getBoundingClientRect();
            return { left: box.left - boundary.left, right: boundary.right - box.right };
          });
        });
      expect(
        chipBoxes.every((box) => box.left >= 0 && box.right >= 0),
        JSON.stringify(chipBoxes),
      ).toBe(true);
      await expect(
        page
          .locator('#list-fixture .page-list-condition-chips .tmt-ui-icon-action-control')
          .first(),
      ).toHaveCSS('border-left-color', 'rgba(0, 0, 0, 0)');
      if (width === 1440)
        expect(
          (await page.locator('#list-fixture .page-list-search-box').boundingBox())!.width,
        ).toBeGreaterThan(500);
      await page.screenshot({ path: info.outputPath('list-filter.png') });
      await page.getByRole('button', { name: 'Remove To do' }).click();
      await expect(page.locator('#list-fixture .page-list-search-box')).not.toContainText(
        'state: To do',
      );
      await expect(page.locator('#list-fixture .page-list-search-box')).toContainText(
        'state: Done',
      );
    });

for (const theme of ['light', 'dark'] as const)
  test(`phone Manage opens the list, then an editor with a back action and fixed footer ${theme}`, async ({
    page,
  }, info) => {
    await page.setViewportSize({ width: 390, height: 900 });
    await page.emulateMedia({ colorScheme: theme });
    await mount(page);
    await page.getByRole('button', { name: 'Save as shortcut', exact: true }).click();
    const save = page.getByRole('dialog', { name: 'Save as shortcut' });
    await save.getByRole('textbox', { name: 'Name' }).fill('My shortcut');
    await save.getByRole('button', { name: 'Save', exact: true }).click();
    await page.getByRole('button', { name: 'Open navigation' }).click();
    await manageShortcuts(page).click();
    const dialog = page.getByRole('dialog', { name: 'Manage shortcuts' });
    await expect(dialog.getByRole('button', { name: 'My shortcut', exact: true })).toBeVisible();
    await expect(dialog.getByRole('textbox', { name: 'Name' })).toBeHidden();
    await page.screenshot({ path: info.outputPath('manage-list.png') });
    await dialog.getByRole('button', { name: 'My shortcut', exact: true }).click();
    await expect(dialog.getByRole('textbox', { name: 'Name' })).toBeVisible();
    await expect(dialog.getByRole('button', { name: 'Save', exact: true })).toBeVisible();
    await page.screenshot({ path: info.outputPath('manage-editor.png') });
    await dialog.getByRole('button', { name: 'Back to shortcuts' }).click();
    await expect(dialog.getByRole('button', { name: 'My shortcut', exact: true })).toBeVisible();
    await expect(dialog.getByRole('button', { name: 'My shortcut', exact: true })).toBeFocused();
  });

for (const width of [390, 1440])
  for (const theme of ['light', 'dark'] as const)
    test(`shortcut filter, empty and storage-error review states ${width} ${theme}`, async ({
      page,
    }, info) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await mount(page);
      await page.getByRole('button', { name: 'Filter', exact: true }).click();
      await expect(
        page.getByRole('menuitemcheckbox', { name: 'To do', exact: true }),
      ).toBeVisible();
      for (const name of ['Page state', 'Project', 'Also'])
        await expect(page.getByRole('group', { name, exact: true })).toBeVisible();
      const projects = page.getByRole('group', { name: 'Project', exact: true });
      await expect(
        projects.getByRole('menuitemcheckbox', { name: 'design', exact: true }),
      ).toBeVisible();
      await expect(
        projects.getByRole('menuitem', { name: 'Manage projects…', exact: true }),
      ).toBeVisible();
      await page.screenshot({ path: info.outputPath('filter-open.png') });
      await page.keyboard.press('Escape');
      if (width === 390) await page.getByRole('button', { name: 'Open navigation' }).click();
      await manageShortcuts(page).click();
      const manage = page.getByRole('dialog', { name: 'Manage shortcuts', exact: true });
      await expect(manage.getByText('No shortcuts yet.')).toBeVisible();
      await page.screenshot({ path: info.outputPath('manage-empty.png') });
      await manage.getByRole('button', { name: 'Cancel', exact: true }).click();
      await page.getByRole('button', { name: 'Save as shortcut', exact: true }).click();
      const save = page.getByRole('dialog', { name: 'Save as shortcut', exact: true });
      await save.getByRole('textbox', { name: 'Name' }).fill('Retained draft');
      await page.evaluate(() => {
        Storage.prototype.setItem = () => {
          throw new Error('quota');
        };
      });
      await save.getByRole('button', { name: 'Save', exact: true }).click();
      await expect(save.getByRole('alert')).toContainText('could not');
      await expect(save.getByRole('textbox', { name: 'Name' })).toHaveValue('Retained draft');
      await page.screenshot({ path: info.outputPath('save-error.png') });
    });
