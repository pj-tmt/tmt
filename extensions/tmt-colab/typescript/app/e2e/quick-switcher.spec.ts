import { expect, test, type Page } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.route('**/src/main.tsx', (route) =>
    route.fulfill({ contentType: 'text/javascript', body: '' }),
  );
});

async function mount(
  page: Page,
  mode: 'home' | 'page' | 'empty' | 'cold-failed' | 'cold-loading' = 'home',
) {
  await page.goto('/');
  await page.evaluate(async (mode) => {
    const fixture = '/test/quick-switcher-browser.tsx';
    await (await import(/* @vite-ignore */ fixture)).mount(mode);
  }, mode);
  await expect(page.locator('.tmt-ui-header')).toBeVisible();
}
const dialog = (page: Page) => page.getByRole('dialog', { name: 'Search pages', exact: true });
const control = (page: Page) => page.getByRole('button', { name: 'Search pages', exact: true });
const options = (page: Page) => dialog(page).getByRole('option');
const proof = (page: Page) =>
  page.evaluate(async () => {
    const fixture = '/test/quick-switcher-browser.tsx';
    return (await import(/* @vite-ignore */ fixture)).proof();
  });
const shortcut = (page: Page) =>
  page.evaluate(() => (/Mac|iPhone|iPad|iPod/.test(navigator.platform) ? 'Meta+k' : 'Control+k'));

for (const width of [1440, 390]) {
  test(`cached pages open without a loading flash, pointer choice navigates and focuses the destination ${width}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await mount(page);
    await expect(page.locator('.pages > li')).toHaveCount(3);
    // The desktop text control must reserve its actual width beside the theme action.
    const searchBox = await control(page).boundingBox();
    const themeBox = await page.locator('.tmt-ui-header').getByRole('button').last().boundingBox();
    expect(searchBox!.x + searchBox!.width).toBeLessThanOrEqual(themeBox!.x);
    await page.evaluate(() => {
      const seen: string[] = [];
      (window as unknown as { switcherStatuses: string[] }).switcherStatuses = seen;
      new MutationObserver(() => {
        for (const node of document.querySelectorAll('.quick-switcher-message'))
          seen.push(node.textContent ?? '');
      }).observe(document.body, { childList: true, subtree: true, characterData: true });
    });
    await control(page).click();
    await expect(dialog(page)).toBeVisible();
    await expect(dialog(page).getByRole('combobox', { name: 'Search pages' })).toBeFocused();
    await expect(options(page)).toHaveCount(3);
    await expect(options(page).filter({ hasText: 'Alpha brief' })).toHaveCount(1);
    const beta = options(page).filter({ hasText: 'Beta review' });
    await expect(beta).toContainText('Waiting for you');
    expect(
      await page.evaluate(
        () => (window as unknown as { switcherStatuses: string[] }).switcherStatuses,
      ),
    ).not.toContain('Loading pages…');
    await beta.click();
    await expect(dialog(page)).toHaveCount(0);
    await expect(page.locator('.tmt-ui-header h1')).toHaveText('Beta review');
    await expect(page.locator('.tmt-ui-header h1')).toBeFocused();
    expect((await proof(page)).pageReads).toEqual(['c0000002-1111-4111-8111-000000000001']);
    await page.keyboard.press(await shortcut(page));
    await expect(options(page)).toHaveCount(2);
    await expect(options(page).filter({ hasText: 'Beta review' })).toHaveCount(0);
    await page.keyboard.press('Escape');
    await expect(dialog(page)).toHaveCount(0);
    await expect(page.locator('.tmt-ui-header h1')).toBeFocused();
  });
}

test('empty, loading and failed list reads have explicit states and close remains reachable', async ({
  page,
}) => {
  await mount(page, 'empty');
  await control(page).click();
  await expect(dialog(page).locator('.quick-switcher-message')).toHaveText('No pages yet.');
  await page.keyboard.press('Escape');
  await mount(page, 'cold-loading');
  await page.keyboard.press(await shortcut(page));
  await expect(dialog(page).locator('.quick-switcher-message')).toHaveText('Loading pages…');
  await page.evaluate(async () => {
    const fixture = '/test/quick-switcher-browser.tsx';
    (await import(/* @vite-ignore */ fixture)).finishRead();
  });
  await expect(options(page)).toHaveCount(2);
  await page.keyboard.press('Escape');
  await mount(page, 'cold-failed');
  await control(page).click();
  await expect(dialog(page).locator('.quick-switcher-message')).toHaveText(
    "Couldn't load pages. Close and try again.",
  );
  await dialog(page).getByRole('button', { name: 'Close Search pages', exact: true }).click();
  await expect(control(page)).toBeFocused();
});

test('trusted frame shortcut opens inert search; editor and modified shortcuts do not', async ({
  page,
}) => {
  await mount(page, 'page');
  const frame = page.frameLocator('iframe[title="Page content"]');
  await expect(frame.getByText('Page content', { exact: true })).toBeVisible();
  const before = await proof(page);
  await frame.getByRole('textbox', { name: 'Author link editor' }).focus();
  await page.keyboard.press(await shortcut(page));
  // A subsequent trusted ordinary key establishes that the frame processed the keyboard sequence.
  await page.keyboard.type('owned by editor');
  await expect(frame.getByRole('textbox', { name: 'Author link editor' })).toHaveValue(
    'owned by editor',
  );
  await expect(dialog(page)).toHaveCount(0);
  await frame.getByText('Page content', { exact: true }).click();
  await page.keyboard.press(await shortcut(page));
  await expect(dialog(page)).toBeVisible();
  expect((await proof(page)).pageReads).toEqual(before.pageReads);
  await page.keyboard.press('Escape');
  await frame.getByText('Page content', { exact: true }).click();
  await page.keyboard.press((await shortcut(page)).replace('+k', '+Shift+k'));
  await frame.getByRole('textbox', { name: 'Author link editor' }).fill('Still here');
  await expect(dialog(page)).toHaveCount(0);
  expect((await proof(page)).pageReads).toEqual(before.pageReads);
});

test('closed current-frame K requests reject malformed messages and can never choose a page', async ({
  page,
}) => {
  await mount(page, 'page');
  const frame = page.locator('iframe[title="Page content"]');
  const child = page.frameLocator('iframe[title="Page content"]').locator('html');
  await expect(child.getByText('Page content', { exact: true })).toBeVisible();
  const renderId = await frame.getAttribute('data-render-id');
  const command = await page.evaluate(() => /Mac|iPhone|iPad|iPod/.test(navigator.platform));
  const valid = {
    type: 'colab.render.key',
    renderId,
    key: 'K',
    ctrlKey: !command,
    metaKey: command,
    altKey: false,
    shiftKey: false,
  };
  const before = await proof(page);
  const messages: Record<string, unknown>[] = [
    { ...valid, renderId: 'stale' },
    { ...valid, key: 'k' },
    { ...valid, extra: true },
    { ...valid, ctrlKey: 1 },
    { ...valid, metaKey: 'true' },
    { ...valid, altKey: true },
    { ...valid, shiftKey: true },
    { ...valid, ctrlKey: false, metaKey: false },
    { ...valid, ctrlKey: true, metaKey: true },
    Object.fromEntries(Object.entries(valid).filter(([name]) => name !== 'shiftKey')),
  ];
  await page.evaluate(() => {
    const probe = window as unknown as { keyBarrier: Promise<void> };
    probe.keyBarrier = new Promise((resolve) => {
      const receive = (event: MessageEvent) => {
        if (
          event.source !== document.querySelector('iframe')?.contentWindow ||
          event.data !== 'key-test-barrier'
        )
          return;
        window.removeEventListener('message', receive);
        // Drain the earlier same-sender messages and their React paint, not a fixed sleep.
        requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
      };
      window.addEventListener('message', receive);
    });
  });
  await child.evaluate((node, messages) => {
    const parent = node.ownerDocument.defaultView!.parent;
    for (const message of messages) parent.postMessage(message, '*');
    parent.postMessage('key-test-barrier', '*');
  }, messages);
  await page.evaluate(() => (window as unknown as { keyBarrier: Promise<void> }).keyBarrier);
  await expect(dialog(page)).toHaveCount(0);
  expect((await proof(page)).pageReads).toEqual(before.pageReads);
  // A matching payload from the parent window is not a current-frame request.
  await page.evaluate((message) => {
    const probe = window as unknown as { sourceBarrier: Promise<void> };
    probe.sourceBarrier = new Promise((resolve) => {
      const receive = (event: MessageEvent) => {
        if (event.source !== window || event.data !== 'source-test-barrier') return;
        window.removeEventListener('message', receive);
        requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
      };
      window.addEventListener('message', receive);
    });
    window.postMessage(message, '*');
    window.postMessage('source-test-barrier', '*');
  }, valid);
  await page.evaluate(() => (window as unknown as { sourceBarrier: Promise<void> }).sourceBarrier);
  await expect(dialog(page)).toHaveCount(0);
  // An author can impersonate the cosmetic bridge but receives only an inert search surface.
  await child.evaluate(
    (node, message) => node.ownerDocument.defaultView!.parent.postMessage(message, '*'),
    valid,
  );
  await expect(dialog(page)).toBeVisible();
  await expect(options(page)).toHaveCount(2);
  expect((await proof(page)).pageReads).toEqual(before.pageReads);
  await dialog(page).getByRole('button', { name: 'Close Search pages', exact: true }).click();
  await expect(dialog(page)).toHaveCount(0);
  expect((await proof(page)).pageReads).toEqual(before.pageReads);
});

test('a Shadow DOM editor in the real frame retains its own search shortcut', async ({ page }) => {
  await mount(page, 'page');
  const frame = page.frameLocator('iframe[title="Page content"]');
  await expect(frame.getByText('Page content', { exact: true })).toBeVisible();
  await frame.locator('body').evaluate((body) => {
    const host = body.ownerDocument.createElement('div');
    body.append(host);
    const root = host.attachShadow({ mode: 'open' });
    const input = body.ownerDocument.createElement('input');
    input.setAttribute('aria-label', 'Shadow link editor');
    root.append(input);
  });
  const editor = frame.getByRole('textbox', { name: 'Shadow link editor' });
  await editor.focus();
  await page.keyboard.press(await shortcut(page));
  await page.keyboard.type('Still editing inside the frame');
  await expect(editor).toHaveValue('Still editing inside the frame');
  await expect(dialog(page)).toHaveCount(0);
  expect((await proof(page)).pageReads).toEqual(['c0000001-1111-4111-8111-000000000001']);
});
