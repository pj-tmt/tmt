import { expect, test, type Page } from '@playwright/test';

async function mount(page: Page) {
  await page.route('**/src/main.tsx', (route) =>
    route.fulfill({ contentType: 'text/javascript', body: '' }),
  );
  await page.goto('/');
  await page.evaluate(async () => {
    const fixture = '/test/quick-switcher-browser.tsx';
    await (await import(/* @vite-ignore */ fixture)).mount();
  });
  await expect(page.locator('.pages > li')).toHaveCount(3);
  await page.getByRole('button', { name: 'Search pages', exact: true }).click();
}
const dialog = (page: Page) => page.getByRole('dialog', { name: 'Search pages', exact: true });
const input = (page: Page) =>
  dialog(page).getByRole('combobox', { name: 'Search pages', exact: true });
const list = (page: Page) => dialog(page).getByRole('listbox', { name: 'Pages', exact: true });
const reads = (page: Page) =>
  page.evaluate(async () => {
    const fixture = '/test/quick-switcher-browser.tsx';
    return (await import(/* @vite-ignore */ fixture)).proof();
  });

async function expectActive(page: Page, title: string) {
  const selected = list(page).getByRole('option', { selected: true });
  await expect(selected).toContainText(title);
  await expect(input(page)).toHaveAttribute(
    'aria-controls',
    (await list(page).getAttribute('id'))!,
  );
  await expect(input(page)).toHaveAttribute(
    'aria-activedescendant',
    (await selected.getAttribute('id'))!,
  );
  await expect(input(page)).toBeFocused();
}

for (const width of [1440, 390]) {
  test(`result arrows clamp, Enter chooses once and focuses the destination ${width}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await mount(page);
    await expect(input(page)).toHaveAttribute(
      'data-placeholder',
      'Search pages, labels or authors…',
    );
    await expectActive(page, 'Beta review');
    await page.keyboard.press('ArrowUp');
    await expectActive(page, 'Beta review');
    await page.keyboard.press('ArrowDown');
    await expectActive(page, 'Alpha brief');
    await page.keyboard.press('ArrowDown');
    await expectActive(page, 'Long title');
    await page.keyboard.press('ArrowDown');
    await expectActive(page, 'Long title');
    await page.keyboard.press('ArrowUp');
    await expectActive(page, 'Alpha brief');
    await page.keyboard.press('Enter');
    await expect(dialog(page)).toHaveCount(0);
    await expect(page.locator('.tmt-ui-header h1')).toHaveText('Alpha brief');
    await expect(page.locator('.tmt-ui-header h1')).toBeFocused();
    expect((await reads(page)).pageReads).toEqual(['c0000001-1111-4111-8111-000000000001']);
  });
}

test('label and author suggestions own the first Enter and active-descendant without replacing the editable', async ({
  page,
}) => {
  await mount(page);
  await input(page).evaluate((node) => Object.assign(window, { originalSearchRoot: node }));
  await page.keyboard.type('label:des');
  const suggestions = dialog(page).getByRole('listbox', { name: 'Search pages', exact: true });
  await expect(suggestions.getByRole('option')).toHaveText(['design (2)']);
  await expect(input(page)).toHaveAttribute(
    'aria-controls',
    (await suggestions.getAttribute('id'))!,
  );
  await page.keyboard.press('Enter');
  await expect(
    dialog(page).getByRole('button', { name: 'Remove label filter design' }),
  ).toBeVisible();
  await expect(list(page).getByRole('option')).toHaveCount(2);
  expect((await reads(page)).pageReads).toEqual([]);
  await page.keyboard.type('by:Ad');
  await expect(suggestions.getByRole('option')).toHaveText(['Ada (2)']);
  await page.keyboard.press('Enter');
  await expect(
    dialog(page).getByRole('button', { name: 'Remove author filter Ada' }),
  ).toBeVisible();
  await expect(list(page).getByRole('option')).toHaveCount(2);
  expect((await reads(page)).pageReads).toEqual([]);
  expect(
    await input(page).evaluate(
      (node) =>
        node === (window as unknown as { originalSearchRoot: HTMLElement }).originalSearchRoot,
    ),
  ).toBe(true);
  await expectActive(page, 'Alpha brief');
  await page.keyboard.press('Enter');
  await expect(page.locator('.tmt-ui-header h1')).toHaveText('Alpha brief');
});

test('chip Enter removes only its filter; Tab never opens a result and Escape restores the opener', async ({
  page,
}) => {
  await mount(page);
  await page.keyboard.type('label:des');
  await page.keyboard.press('Enter');
  const chip = dialog(page).getByRole('button', { name: 'Remove label filter design' });
  await chip.focus();
  await page.keyboard.press('Enter');
  await expect(chip).toHaveCount(0);
  await expect(list(page).getByRole('option')).toHaveCount(3);
  expect((await reads(page)).pageReads).toEqual([]);
  await input(page).focus();
  await page.keyboard.press('Tab');
  await expect(dialog(page)).toBeVisible();
  expect((await reads(page)).pageReads).toEqual([]);
  await page.keyboard.press('Escape');
  await expect(dialog(page)).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Search pages', exact: true })).toBeFocused();
});

test('Escape closes suggestions first, then the dialog, and empty matches cannot navigate', async ({
  page,
}) => {
  await mount(page);
  await page.keyboard.type('label:des');
  const suggestions = dialog(page).getByRole('listbox', { name: 'Search pages', exact: true });
  await expect(suggestions).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(suggestions).not.toBeVisible();
  await expect(dialog(page)).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(dialog(page)).toHaveCount(0);
  await page.getByRole('button', { name: 'Search pages', exact: true }).click();
  await page.keyboard.type('no such page');
  await expect(dialog(page).locator('.quick-switcher-message')).toHaveText('No matching pages.');
  await page.keyboard.press('Shift+Escape');
  await expect(dialog(page)).toBeVisible();
  await expect(input(page)).toContainText('no such page');
  await page.keyboard.press('Enter');
  await expect(dialog(page)).toBeVisible();
  expect((await reads(page)).pageReads).toEqual([]);
});
