import { capturePath } from './captures.js';
import { expect, test, type Page } from '@playwright/test';
test.use({ actionTimeout: 5000 });
async function mount(page: Page, size: 'stages' | 'work' = 'stages') {
  await page.goto('/');
  await page.evaluate(async (size) => {
    const path = '/test/page-list-browser.tsx';
    await (await import(path)).mountPageList(size, '/', ['design']);
  }, size);
}
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
test('Manage creates and reorders explicit shortcuts while sidebar is unchanged', async ({
  page,
}) => {
  await mount(page);
  await page.evaluate(async () => {
    const path = '/test/page-list-browser.tsx';
    await (await import(path)).mountPageList('stages', '/?scope=design', ['design']);
  });
  await page.getByRole('button', { name: 'Save as shortcut', exact: true }).click();
  const save = page.getByRole('dialog', { name: 'Save as shortcut', exact: true });
  await save.getByRole('textbox', { name: 'Name', exact: true }).fill('Design');
  await save.getByRole('button', { name: 'Save', exact: true }).click();
  await page.getByRole('button', { name: 'Manage shortcuts', exact: true }).click();
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
  await expect(page.locator('a[data-id="project:design"]').first()).toBeVisible();
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
  await page.getByRole('button', { name: 'Manage shortcuts', exact: true }).click();
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
      await page.getByRole('button', { name: 'Manage shortcuts', exact: true }).click();
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
      await expect(
        page.getByRole('button', { name: 'Manage shortcuts', exact: true }),
      ).toBeFocused();
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
  await page.getByRole('button', { name: 'Manage shortcuts', exact: true }).click();
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
    await fresh.getByRole('button', { name: 'Manage shortcuts', exact: true }).click();
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
    await (await import(path)).mountPageList('stages', '/?scope=gone', ['design']);
  });
  await page.getByRole('button', { name: 'Save as shortcut', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Save as shortcut', exact: true });
  await expect(dialog.getByRole('status')).toHaveText('5 matching pages');
  await dialog.getByRole('textbox', { name: 'Name', exact: true }).fill('Current view');
  await dialog.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.locator('#list-fixture .pages > li')).toHaveCount(5);
});

test('PR A leaves legacy pins unmigrated while the pinned-project sidebar remains active', async ({
  page,
}) => {
  await mount(page);
  await expect(page.locator('a[data-id="project:design"]').first()).toBeVisible();
  const record = await page.evaluate(async () => {
    const path = '/test/page-list-browser.tsx';
    const { pageListSpace } = await import(path);
    return localStorage.getItem(`tmt-colab-shortcuts-v1:${pageListSpace}`);
  });
  expect(record).toBeNull();
  await page.getByRole('button', { name: 'Manage shortcuts', exact: true }).click();
  await expect(page.getByRole('dialog', { name: 'Manage shortcuts', exact: true })).toContainText(
    'No shortcuts yet.',
  );
  await page
    .getByRole('dialog', { name: 'Manage shortcuts', exact: true })
    .getByRole('button', { name: 'Cancel', exact: true })
    .click();
  await page.getByRole('button', { name: 'Save as shortcut', exact: true }).click();
  const save = page.getByRole('dialog', { name: 'Save as shortcut', exact: true });
  await save.getByRole('textbox', { name: 'Name', exact: true }).fill('Manual view');
  await save.getByRole('button', { name: 'Save', exact: true }).click();
  const saved = await page.evaluate(async () => {
    const path = '/test/page-list-browser.tsx';
    const { pageListSpace } = await import(path);
    return JSON.parse(localStorage.getItem(`tmt-colab-shortcuts-v1:${pageListSpace}`)!);
  });
  expect(saved.migrated).toBe(false);
  expect(saved.items.map((item: { name: string }) => item.name)).toEqual(['Manual view']);
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
  await page.getByRole('button', { name: 'Manage shortcuts', exact: true }).click();
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
