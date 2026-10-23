import { expect, test, type Page } from '@playwright/test';

async function mount(page: Page) {
  await page.route('**/src/main.tsx', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: "import { mount } from '/test/quick-switcher-browser.tsx'; void mount();",
    }),
  );
  await page.goto('/');
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
      'Search pages, projects or authors…',
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

test('project and author suggestions own the first Enter and active-descendant without replacing the editable', async ({
  page,
}) => {
  await mount(page);
  await input(page).evaluate((node) => Object.assign(window, { originalSearchRoot: node }));
  await page.keyboard.type('project:des');
  const suggestions = dialog(page).getByRole('listbox', { name: 'Search pages', exact: true });
  await expect(suggestions.getByRole('option')).toHaveText(['design (2)']);
  await expect(input(page)).toHaveAttribute(
    'aria-controls',
    (await suggestions.getAttribute('id'))!,
  );
  await page.keyboard.press('Enter');
  await expect(
    dialog(page).getByRole('button', { name: 'Remove project filter design' }),
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
  await page.keyboard.type('project:des');
  await page.keyboard.press('Enter');
  const chip = dialog(page).getByRole('button', { name: 'Remove project filter design' });
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
  await page.keyboard.type('project:des');
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

test('search results are real page links and preserve modified clicks', async ({
  page,
  context,
}) => {
  await mount(page);
  const result = list(page).getByRole('option', { name: /Beta review/ });
  await expect(result).toHaveAttribute('href', /#\/pages\/c0000002-1111-4111-8111-000000000001$/);
  await result.evaluate((node) => (node as HTMLAnchorElement).click());
  await expect(dialog(page)).toBeVisible();
  expect((await reads(page)).pageReads).toEqual([]);
  expect(context.pages()).toHaveLength(1);
  for (const options of [
    { modifiers: ['ControlOrMeta'] },
    { modifiers: ['Shift'] },
    { button: 'middle' as const },
  ] satisfies NonNullable<Parameters<typeof result.click>[0]>[]) {
    const opened = context.waitForEvent('page');
    await result.click(options);
    const destination = await opened;
    await expect(destination).toHaveURL(/#\/pages\/c0000002-1111-4111-8111-000000000001$/);
    await expect(dialog(page)).toBeVisible();
    expect((await reads(page)).pageReads).toEqual([]);
    await destination.close();
    await page.bringToFront();
  }
});

for (const key of ['Control+Enter', 'Meta+Enter', 'Shift+Enter']) {
  test(`search result ${key} opens another browser page and keeps the search`, async ({
    page,
    context,
  }) => {
    await mount(page);
    await expectActive(page, 'Beta review');
    const opened = context.waitForEvent('page');
    await input(page).press(key);
    const destination = await opened;
    await expect(destination).toHaveURL(/#\/pages\/c0000002-1111-4111-8111-000000000001$/);
    await expect(dialog(page)).toBeVisible();
    expect((await reads(page)).pageReads).toEqual([]);
    await destination.close();
  });
}

test('page-list links preserve native modified activation without changing this page', async ({
  page,
  context,
}) => {
  await mount(page);
  await input(page).press('Escape');
  const link = page.locator('.pages').getByRole('link', { name: 'Beta review', exact: true });
  await expect(link).toHaveAttribute('href', /#\/pages\/c0000002-1111-4111-8111-000000000001$/);
  for (const key of ['Control+Enter', 'Meta+Enter', 'Shift+Enter']) {
    const opened = context.waitForEvent('page');
    await link.press(key);
    const destination = await opened;
    await expect(destination).toHaveURL(/#\/pages\/c0000002-1111-4111-8111-000000000001$/);
    expect((await reads(page)).pageReads).toEqual([]);
    await destination.close();
    await page.bringToFront();
  }
  for (const options of [
    { modifiers: ['ControlOrMeta'] },
    { button: 'middle' as const },
  ] satisfies NonNullable<Parameters<typeof link.click>[0]>[]) {
    const opened = context.waitForEvent('page');
    await link.click(options);
    const destination = await opened;
    await expect(destination).toHaveURL(/#\/pages\/c0000002-1111-4111-8111-000000000001$/);
    expect((await reads(page)).pageReads).toEqual([]);
    await destination.close();
    await page.bringToFront();
  }
  await link.press('Enter');
  await expect(page.locator('.tmt-ui-header h1')).toHaveText('Beta review');
});

test('page-list Shift-click keeps the href and leaves navigation to the browser', async ({
  page,
}) => {
  await mount(page);
  await input(page).press('Escape');
  const link = page.locator('.pages').getByRole('link', { name: 'Beta review', exact: true });
  const source = page.url();
  await expect(link).toHaveAttribute('href', /#\/pages\/c0000002-1111-4111-8111-000000000001$/);
  const shifted = await link.evaluateHandle((anchor) => {
    const observation = { event: null as MouseEvent | null };
    const capture = (event: MouseEvent) => {
      if (!(event.target instanceof Node) || !anchor.contains(event.target)) return;
      // Keep the native event itself; read defaultPrevented only after click dispatch completes.
      observation.event = event;
      document.removeEventListener('click', capture, true);
    };
    document.addEventListener('click', capture, true);
    return observation;
  });
  await link.click({ modifiers: ['Shift'] });
  expect(
    await shifted.evaluate(
      ({ event }) =>
        event && {
          trusted: event.isTrusted,
          shift: event.shiftKey,
          button: event.button,
          cancelable: event.cancelable,
          prevented: event.defaultPrevented,
        },
    ),
  ).toEqual({ trusted: true, shift: true, button: 0, cancelable: true, prevented: false });
  await expect(link).toHaveAttribute('href', /#\/pages\/c0000002-1111-4111-8111-000000000001$/);
  expect(page.url()).toBe(source);
  expect((await reads(page)).pageReads).toEqual([]);
  await shifted.dispose();
});
