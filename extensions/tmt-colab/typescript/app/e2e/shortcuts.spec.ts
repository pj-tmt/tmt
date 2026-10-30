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
/** The list's own Manage; the side menu's Shortcuts group has the same command. */
const heroManage = (page: Page) =>
  page
    .locator('.page-list-shortcuts')
    .getByRole('button', { name: 'Manage shortcuts', exact: true });
const rail = (page: Page) => page.locator('#list-fixture .tmt-ui-rail-sub');
const railShortcuts = (page: Page) => rail(page).locator('a[data-id^="shortcut:"]');
const selected = (page: Page) => rail(page).locator('[aria-current="page"]');
test('Save current filters, edit the shortcut, retain drafts on failed storage and keep owner writes untouched', async ({
  page,
}) => {
  await mount(page);
  await page.getByRole('checkbox', { name: 'Include archived', exact: true }).check();
  await page.getByRole('button', { name: 'Save as shortcut', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Save as shortcut', exact: true });
  await dialog.getByRole('textbox', { name: 'Name', exact: true }).fill('My work');
  await dialog.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(dialog).not.toBeVisible();
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
  await heroManage(page).click();
  const dialog = page.getByRole('dialog', { name: 'Manage shortcuts', exact: true });
  await expect(dialog.getByRole('button', { name: 'Design', exact: true })).toBeVisible();
  await expect(dialog.getByRole('checkbox', { name: 'design', exact: true })).toBeChecked();
  await dialog.getByRole('button', { name: 'New shortcut', exact: true }).click();
  await dialog.getByRole('textbox', { name: 'Name', exact: true }).fill('Everything');
  await dialog.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(dialog.getByRole('button', { name: 'Everything', exact: true })).toBeVisible();
  await dialog.getByRole('button', { name: 'Move Everything up', exact: true }).click();
  await expect(dialog.locator('.shortcut-list > li').first()).toContainText('Everything');
  await dialog
    .locator('.shortcut-list > li')
    .nth(1)
    .dragTo(dialog.locator('.shortcut-list > li').first());
  await expect(dialog.locator('.shortcut-list > li').first()).toContainText('Design');
  await dialog.getByRole('button', { name: 'Close Manage shortcuts', exact: true }).click();
  await expect(railShortcuts(page)).toHaveText(['Design', 'Everything']);
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
  await heroManage(page).click();
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
      await page.getByRole('button', { name: 'Save as shortcut', exact: true }).click();
      const save = page.getByRole('dialog', { name: 'Save as shortcut', exact: true });
      await save.getByRole('textbox', { name: 'Name', exact: true }).fill('Design reviews');
      await expect(save).toBeVisible();
      await page.mouse.move(width - 1, 899);
      await page.screenshot({ path: capturePath(`shortcuts-save-${width}-${theme}.png`) });
      await save.getByRole('button', { name: 'Save', exact: true }).click();
      await heroManage(page).click();
      const manage = page.getByRole('dialog', { name: 'Manage shortcuts', exact: true });
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
      await expect(heroManage(page)).toBeFocused();
    });
  }

test('Clear filters also clears manual state and include archived', async ({ page }) => {
  await mount(page);
  await page.getByRole('button', { name: 'Page state: Any', exact: true }).click();
  await page.getByRole('menuitemradio', { name: 'To do', exact: true }).click();
  await page.getByRole('checkbox', { name: 'Include archived', exact: true }).check();
  await page.getByRole('button', { name: 'Clear filters', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Page state: Any', exact: true })).toBeVisible();
  await expect(
    page.getByRole('checkbox', { name: 'Include archived', exact: true }),
  ).not.toBeChecked();
});

test('another tab receives storage updates; a separate browser profile has no saved shortcut', async ({
  page,
  browser,
}) => {
  await mount(page);
  await heroManage(page).click();
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
    await heroManage(fresh).click();
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
  await expect(dialog.getByRole('status')).toHaveText('5 matching pages');
  await dialog.getByRole('textbox', { name: 'Name', exact: true }).fill('Current view');
  await dialog.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.locator('#list-fixture .pages > li')).toHaveCount(5);
});

test('explicit pinned projects become project shortcuts once, in order; automatic ones do not', async ({
  page,
}) => {
  await mount(page, 'stages', ['ops', 'design']);
  await expect(railShortcuts(page)).toHaveText(['ops', 'design']);
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
  await expect(railShortcuts(page)).toHaveText(['ops', 'design']);
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
  await expect(railShortcuts(page)).toHaveText(['ops']);
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

test('Manage uses shared field labels and the same page-state menu as the list', async ({
  page,
}) => {
  await mount(page);
  await heroManage(page).click();
  const dialog = page.getByRole('dialog', { name: 'Manage shortcuts', exact: true });
  const labels = dialog.locator('.tmt-ui-field-label');
  await expect(labels).toHaveText(['Name', 'Projects', 'Page state']);
  const positions = await labels.evaluateAll((nodes) =>
    nodes.map((node) => node.getBoundingClientRect().left),
  );
  expect(new Set(positions).size).toBe(1);
  await expect(dialog.locator('select')).toHaveCount(0);
  await dialog.getByRole('button', { name: /^Page state:/ }).click();
  await page.getByRole('menuitemradio', { name: 'Done', exact: true }).click();
  await expect(dialog.getByRole('button', { name: 'Page state: Done', exact: true })).toBeVisible();
});

test('the side menu lists the shortcuts, selects exactly one row and opens the list dialogs', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await mount(page);
  await expect(railShortcuts(page)).toHaveCount(0);
  await expect(selected(page)).toHaveAttribute('data-id', 'all');
  // + New shortcut opens the list's Save dialog.
  await rail(page).getByRole('button', { name: '+ New shortcut', exact: true }).click();
  const save = page.getByRole('dialog', { name: 'Save as shortcut', exact: true });
  await save.getByRole('textbox', { name: 'Name', exact: true }).fill('Mine');
  await save.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(railShortcuts(page)).toHaveText(['Mine']);
  // The list now shows the shortcut, so its row is the one selected and the bar names it.
  await expect(selected(page)).toHaveCount(1);
  await expect(railShortcuts(page)).toHaveAttribute('aria-current', 'page');
  await expect(page.locator('#list-fixture .tmt-ui-topbar-title-current')).toHaveText('Mine');
  // A page opened from that list keeps the shortcut selected.
  await page.locator('#list-fixture .pages > li .tmt-ui-list-title > a').first().click();
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
  await rail(page).getByRole('button', { name: 'Manage shortcuts', exact: true }).click();
  await expect(page.getByRole('dialog', { name: 'Manage shortcuts', exact: true })).toBeVisible();
  await expect(selected(page)).toHaveAttribute('data-id', 'all');
});
