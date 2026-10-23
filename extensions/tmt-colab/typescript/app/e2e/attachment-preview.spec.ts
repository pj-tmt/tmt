import { expect, test, type Page } from '@playwright/test';
const fixture = '/test/attachment-preview-browser.tsx';
const proof = (page: Page) => page.evaluate(async (path) => (await import(path)).proof(), fixture);
const invoke = (page: Page, method: string) =>
  page.evaluate(async ({ path, method }) => (await import(path))[method](), {
    path: fixture,
    method,
  });
async function mount(
  page: Page,
  options: {
    count?: number;
    held?: boolean;
    kind?: string;
    refused?: boolean;
    files?: boolean;
  } = {},
) {
  await page.goto('/');
  await page.evaluate(async ({ path, options }) => (await import(path)).mount(options), {
    path: fixture,
    options,
  });
}
for (const count of [1, 2, 6, 7])
  test(`${count} images: visible tiles only, serial reads and exact grid alignment`, async ({
    page,
  }) => {
    await mount(page, { count });
    const expected = count > 6 ? 5 : count;
    await expect(page.locator('.attachment-thumbnail img')).toHaveCount(expected);
    await expect.poll(async () => (await proof(page)).reads.length).toBe(expected);
    expect((await proof(page)).maximum).toBe(1);
    expect((await proof(page)).zeroed).toBe(true);
    const alignment = await page
      .locator('.attachment-images')
      .evaluate((node) =>
        Math.abs(
          node.getBoundingClientRect().left -
            document.querySelector('.comment-body')!.getBoundingClientRect().left,
        ),
      );
    expect(alignment).toBeLessThanOrEqual(1);
    if (count === 7)
      await expect(page.getByRole('button', { name: 'Show 2 more images' })).toHaveText('+2');
    await page.getByRole('button', { name: 'After attachments' }).scrollIntoViewIfNeeded();
    await page.locator('.attachment-images').scrollIntoViewIfNeeded();
    expect((await proof(page)).reads).toHaveLength(expected);
  });
test('overflow opens image six, navigates all images, and Escape restores the original tile', async ({
  page,
}) => {
  await mount(page);
  await expect(page.locator('.attachment-thumbnail img')).toHaveCount(5);
  const more = page.getByRole('button', { name: 'Show 2 more images' });
  await more.click();
  const viewer = page.getByRole('dialog');
  await expect(viewer).toContainText('6 / 7');
  await expect(viewer.getByRole('button', { name: 'Close preview' })).toBeFocused();
  await expect(viewer.locator('img')).toHaveAttribute('alt', 'image-6.png');
  await page.keyboard.press('ArrowRight');
  await expect(viewer.locator('img')).toHaveAttribute('alt', 'image-7.png');
  await expect(viewer.getByRole('button', { name: 'Next image' })).toBeDisabled();
  await page.keyboard.press('Tab');
  expect(await viewer.evaluate((node) => node.contains(document.activeElement))).toBe(true);
  await page.keyboard.press('Escape');
  await expect(viewer).toHaveCount(0);
  await expect(more).toBeFocused();
  expect((await proof(page)).reads).toHaveLength(7);
});
test('scope replacement drops late reads and a new reference reads again', async ({ page }) => {
  await mount(page, { count: 1, held: true });
  await expect.poll(async () => (await proof(page)).reads.length).toBe(1);
  await invoke(page, 'advance');
  await invoke(page, 'finish');
  await expect(page.locator('.attachment-thumbnail img')).toHaveCount(1);
  expect((await proof(page)).reads).toHaveLength(2);
  expect((await proof(page)).zeroed).toBe(true);
  await invoke(page, 'revise');
  await expect.poll(async () => (await proof(page)).reads.length).toBe(3);
  expect((await proof(page)).reads.at(-1)).toBe('image-1.png:2');
});
test('non-raster descriptors never auto-read and remain inert download cards', async ({ page }) => {
  await mount(page, { count: 1, kind: 'image/svg+xml' });
  await expect(page.getByTestId('message-attachment')).toBeVisible();
  expect((await proof(page)).reads).toEqual([]);
  await expect(page.locator('img, video, object, embed')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Download', exact: true })).toBeVisible();
});
test('queued offscreen tiles do not read; returning retains already verified URLs', async ({
  page,
}) => {
  await mount(page, { held: true });
  await expect.poll(async () => (await proof(page)).reads.length).toBe(1);
  await page.getByRole('button', { name: 'After attachments' }).scrollIntoViewIfNeeded();
  await invoke(page, 'finish');
  await expect.poll(async () => (await proof(page)).zeroed).toBe(true);
  expect((await proof(page)).reads).toHaveLength(1);
  await page.locator('.attachment-images').scrollIntoViewIfNeeded();
  await expect(page.locator('.attachment-thumbnail img')).toHaveCount(5);
  expect((await proof(page)).reads).toHaveLength(5);
  expect((await proof(page)).maximum).toBe(1);
});
test('unmount drops a late admitted result and clears its plaintext', async ({ page }) => {
  await mount(page, { count: 1, held: true });
  await expect.poll(async () => (await proof(page)).reads.length).toBe(1);
  await invoke(page, 'unmount');
  await invoke(page, 'finish');
  await expect.poll(async () => (await proof(page)).zeroed).toBe(true);
  await expect(page.locator('.attachment-thumbnail img')).toHaveCount(0);
});
test('binding replacement rereads under a new lifetime and closes its viewer', async ({ page }) => {
  await mount(page, { count: 1 });
  await expect(page.locator('.attachment-thumbnail img')).toHaveCount(1);
  await page.getByRole('button', { name: 'Preview image-1.png' }).click();
  await expect(page.locator('.attachment-viewer')).toBeVisible();
  await invoke(page, 'replaceBinding');
  await expect(page.locator('.attachment-viewer')).toHaveCount(0);
  await expect.poll(async () => (await proof(page)).reads.length).toBe(2);
});
test('a denied thumbnail reports the admitted failure and never retries just from scrolling', async ({
  page,
}) => {
  await mount(page, { count: 1, refused: true });
  await expect(page.getByTestId('message-attachment').getByRole('alert')).toBeVisible();
  await expect(page.locator('img')).toHaveCount(0);
  await page.getByRole('button', { name: 'After attachments' }).scrollIntoViewIfNeeded();
  await page.locator('.attachment-images').scrollIntoViewIfNeeded();
  expect((await proof(page)).reads).toHaveLength(1);
  await page.getByRole('button', { name: 'Preview image-1.png' }).click();
  await expect(page.locator('.attachment-viewer').getByRole('alert')).toBeVisible();
  expect((await proof(page)).reads).toHaveLength(2);
});
const captures = process.env.COLAB_2554_CAPTURE_DIR;
for (const width of [1440, 390])
  for (const scheme of ['light', 'dark'] as const)
    test(`capture grid alignment and viewer ${width} ${scheme}`, async ({ page }) => {
      test.skip(!captures, 'Set COLAB_2554_CAPTURE_DIR for review captures.');
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: scheme });
      await mount(page);
      await expect(page.locator('.attachment-thumbnail img')).toHaveCount(5);
      await page.screenshot({ path: `${captures}/2554-grid-alignment-${width}-${scheme}.png` });
      await page.getByRole('button', { name: 'Show 2 more images' }).click();
      await expect(page.getByRole('dialog').locator('img')).toBeVisible();
      await page.screenshot({ path: `${captures}/2554-viewer-${width}-${scheme}.png` });
    });

test('Files click uses the current scope even before another render', async ({ page }) => {
  await mount(page, { count: 1, files: true });
  await expect(page.getByTestId('file-row')).toBeVisible();
  await invoke(page, 'advanceWithoutRender');
  const pending = page.waitForEvent('download');
  await page.getByRole('button', { name: 'Download', exact: true }).click();
  const saved = await pending;
  expect(saved.suggestedFilename()).toBe('image-1.png');
  const stream = await saved.createReadStream();
  const chunks: Buffer[] = [];
  for await (const chunk of stream!) chunks.push(chunk);
  expect(Buffer.concat(chunks).toString('base64')).toBe(
    'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==',
  );
  expect((await proof(page)).reads).toEqual(['image-1.png:document']);
  expect((await proof(page)).zeroed).toBe(true);
});

for (const change of ['advance', 'removeAll'])
  test(`Files ${change} during an explicit read shows retry without replay`, async ({ page }) => {
    await mount(page, { count: 1, files: true, held: true });
    const downloads: string[] = [];
    page.on('download', (item) => downloads.push(item.suggestedFilename()));
    await page.getByRole('button', { name: 'Download', exact: true }).click();
    await expect.poll(async () => (await proof(page)).reads.length).toBe(1);
    await invoke(page, change);
    await invoke(page, 'finish');
    await expect(page.getByRole('alert')).toHaveText(
      'The page changed while opening this file. Try again.',
    );
    expect((await proof(page)).reads).toEqual(['image-1.png:document']);
    expect((await proof(page)).zeroed).toBe(true);
    expect(downloads).toEqual([]);
  });

test('queued trusted download selects the current message revision without a second click', async ({
  page,
}) => {
  await mount(page, { count: 1, held: true });
  await expect.poll(async () => (await proof(page)).reads.length).toBe(1);
  await page.getByRole('button', { name: 'Preview image-1.png' }).click();
  let downloads = 0;
  page.on('download', () => downloads++);
  const pending = page.waitForEvent('download');
  await page.getByRole('dialog').getByRole('button', { name: 'Download', exact: true }).click();
  await invoke(page, 'revise');
  await invoke(page, 'advanceWithoutRender');
  await invoke(page, 'finish');
  expect((await pending).suggestedFilename()).toBe('image-1.png');
  // The download event does not join the replacement thumbnail's FileReader. Its rendered
  // image marks the current preview read complete before checking all returned bytes.
  await expect(page.locator('.attachment-thumbnail img')).toBeVisible();
  const result = await proof(page);
  expect(result.reads.slice(0, 2)).toEqual(['image-1.png:1', 'image-1.png:2']);
  expect(result.readScopes.slice(0, 2)).toEqual(['head:1', 'head:2']);
  expect(downloads).toBe(1);
  expect(result.maximum).toBe(1);
  expect(result.zeroed).toBe(true);
});

for (const change of ['removeAll', 'unavailable'])
  test(`queued trusted download refuses ${change} visibly without reading stale metadata`, async ({
    page,
  }) => {
    await mount(page, { count: 1, held: true });
    await expect.poll(async () => (await proof(page)).reads.length).toBe(1);
    await page.getByRole('button', { name: 'Preview image-1.png' }).click();
    await page.getByRole('dialog').getByRole('button', { name: 'Download', exact: true }).click();
    await invoke(page, change);
    await invoke(page, 'finish');
    await expect(page.getByRole('alert')).toHaveText(
      'The page changed while opening this file. Try again.',
    );
    expect((await proof(page)).reads).toEqual(['image-1.png:1']);
    expect((await proof(page)).zeroed).toBe(true);
  });

for (const change of ['advance', 'unmount'])
  test(`download URL ${change === 'advance' ? 'survives display replacement until handoff ends' : 'is revoked on component unmount'}`, async ({
    page,
  }) => {
    await page.addInitScript(() => {
      const active = new Set<string>();
      Object.defineProperty(window, 'attachmentDownloadUrls', { value: active });
      const create = URL.createObjectURL.bind(URL),
        revoke = URL.revokeObjectURL.bind(URL);
      URL.createObjectURL = (blob) => {
        const url = create(blob);
        active.add(url);
        return url;
      };
      URL.revokeObjectURL = (url) => {
        active.delete(url);
        revoke(url);
      };
    });
    await page.clock.install({ time: new Date('2026-01-01T00:00:00Z') });
    await mount(page, { count: 1, files: true });
    await expect(page.getByTestId('file-row')).toBeVisible();
    await page.clock.pauseAt(new Date('2026-01-01T00:01:00Z'));
    const pending = page.waitForEvent('download');
    await page.getByRole('button', { name: 'Download', exact: true }).click();
    await pending;
    const urls = () =>
      page.evaluate(
        () =>
          (window as unknown as { attachmentDownloadUrls: Set<string> }).attachmentDownloadUrls
            .size,
      );
    expect(await urls()).toBe(1);
    await invoke(page, change);
    if (change === 'advance') {
      await expect(page.getByTestId('file-row')).toBeVisible();
      expect(await urls()).toBe(1);
      await page.clock.fastForward(1000);
    }
    await expect.poll(urls).toBe(0);
    expect((await proof(page)).zeroed).toBe(true);
  });

test('Files preview captures the current scope before another render', async ({ page }) => {
  await mount(page, { count: 1, files: true });
  await expect(page.getByTestId('file-row')).toBeVisible();
  await invoke(page, 'advanceWithoutRender');
  await page.getByRole('button', { name: 'Preview', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'image-1.png', exact: true });
  await expect(dialog.locator('img')).toHaveAttribute('src', /^data:image\/png;base64,/);
  expect((await proof(page)).reads).toEqual(['image-1.png:document']);
  expect((await proof(page)).zeroed).toBe(true);
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Preview', exact: true })).toBeFocused();
});
