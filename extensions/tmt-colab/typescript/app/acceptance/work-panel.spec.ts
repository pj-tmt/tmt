import { mkdirSync } from 'node:fs';
import { expect, test, type Page, type Locator } from '@playwright/test';
import { pairBrowser, startDoor, openReaderLink } from './harness/browser.js';
import { createPage, openPage, run } from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

/** Match the settled shared token, not an in-between light/dark transition frame. */
async function panelTheme(page: Page, chip: Locator, theme: string) {
  await page.mouse.move(0, 0);
  await page.evaluate((value) => {
    document.documentElement.dataset.theme = value;
  }, theme);
  const background = await page.evaluate(() => {
    const probe = document.createElement('span');
    probe.style.backgroundColor = 'var(--tmt-ui-surface-command)';
    document.body.append(probe);
    const color = getComputedStyle(probe).backgroundColor;
    probe.remove();
    return color;
  });
  await expect(chip).toHaveCSS('background-color', background);
}

test.afterEach(disposeActiveWorlds);

test('manual state persists, an empty band offers Add to the owner only, and share readers have no owner controls', async () => {
  await withWorld(async (world) => {
    const created = createPage(world, 'Status panel', '<h1>Status panel source</h1>');
    const colab = (args: string[]) =>
      JSON.parse(run(world, world.binaries.colab, [...args, '--json']));
    const before = colab(['show', created.pageId]);
    expect(before.page.state).toBe('todo');
    expect(before.page.progress).toEqual({ done: 0, inProgress: 0, todo: 0, unknown: 0, total: 0 });
    const door = await startDoor(world);
    const browser = await pairBrowser(world, 'status-owner');
    const page = await openPage(door, browser, created);
    const band = page.getByRole('region', { name: 'Work on this page' });
    // A page without sub-pages: the band says so and offers Add.
    await expect(band.locator('.work-head')).toContainText('No sub-pages yet');
    await expect(band.getByRole('button', { name: 'Add', exact: true })).toBeVisible();
    await band
      .locator('.work-head-chip')
      .getByRole('button', { name: /Page state/ })
      .click();
    await page.getByRole('menuitemradio', { name: 'Done', exact: true }).click();
    await expect(
      band.locator('.work-head-chip').getByRole('button', { name: /Page state/ }),
    ).toContainText('Done');
    const after = colab(['show', created.pageId]);
    expect(after.page.state).toBe('done');
    expect(after.page.progress).toEqual(before.page.progress);
    await page.reload();
    await expect(
      band.locator('.work-head-chip').getByRole('button', { name: /Page state/ }),
    ).toContainText('Done');
    await expect(band.locator('.work-head')).toContainText('No sub-pages yet');
    const captures = process.env.COLAB_STATUS_CAPTURE_DIR;
    if (captures) {
      mkdirSync(captures, { recursive: true });
      for (const width of [390, 1440])
        for (const theme of ['light', 'dark']) {
          await page.setViewportSize({ width, height: 900 });
          await panelTheme(
            page,
            band.locator('.work-head-chip').getByRole('button', { name: /Page state/ }),
            theme,
          );
          await page.screenshot({
            path: `${captures}/status-owner-empty-${width}-${theme}.png`,
          });
        }
    }
    colab(['share', 'mode', created.pageId, 'link', '--yes']);
    const link = colab(['share', 'link', 'add', created.pageId, '--yes']);
    const reader = await openReaderLink(world, door, link.readerPath, 'status-reader');
    const shared = reader.page.getByRole('region', { name: 'Work on this page' });
    await expect(shared.locator('.work-page-state')).toHaveText('Done');
    await expect(shared.locator('.work-head')).toContainText('No sub-pages yet');
    await expect(shared.getByRole('button', { name: /Page state/ })).toHaveCount(0);
    await expect(shared.getByRole('button', { name: 'Add', exact: true })).toHaveCount(0);
    await expect(shared.locator('.work-toggle')).toHaveCount(0);
    expect(
      reader.requests.filter((request) => /summary-details|api\/space/.test(request.url)),
    ).toEqual([]);
    if (captures)
      for (const width of [390, 1440])
        for (const theme of ['light', 'dark']) {
          await reader.page.setViewportSize({ width, height: 900 });
          await panelTheme(reader.page, shared.locator('.work-page-state'), theme);
          await reader.page.screenshot({
            path: `${captures}/status-reader-null-${width}-${theme}.png`,
          });
        }
  });
});

test('opening a page with sub-pages reads details once and never shows a wrong progress frame', async () => {
  await withWorld(async (world) => {
    const parent = createPage(world, 'Status parent', '<h1>Status parent source</h1>');
    const child = createPage(world, 'Status child', '<h1>Status child source</h1>');
    const colab = (args: string[]) =>
      JSON.parse(run(world, world.binaries.colab, [...args, '--json']));
    colab(['subpage', 'attach', parent.pageId, child.pageId]);
    const door = await startDoor(world);
    const browser = await pairBrowser(world, 'progress-open');
    let reads = 0;
    browser.context.on('request', (request) => {
      if (new URL(request.url()).pathname.endsWith(`/api/pages/${parent.pageId}/summary-details`))
        reads++;
    });
    await browser.context.addInitScript(() => {
      const frames: string[] = [];
      Object.assign(window, { statusProgressFrames: frames });
      new MutationObserver(() => {
        const value = document.querySelector('.work-band .work-progress .work-wide')?.textContent;
        if (value) frames.push(value);
      }).observe(document, { childList: true, subtree: true, characterData: true });
    });
    const page = await openPage(door, browser, parent);
    const band = page.getByRole('region', { name: 'Work on this page' });
    await expect(band.locator('.work-progress .work-wide')).toHaveText('0 of 1 done');
    await band.getByRole('button', { name: 'Expand work panel', exact: true }).click();
    await expect(band.getByRole('link', { name: 'Status child', exact: true })).toBeVisible();
    expect(reads).toBe(1);
    const frames = await page.evaluate(
      () => (window as unknown as { statusProgressFrames: string[] }).statusProgressFrames,
    );
    expect(frames.length).toBeGreaterThan(0);
    for (const frame of frames) expect(frame).toBe('0 of 1 done');
  });
});

test('sub-pages attach, get created and detach at any depth through the owner admission', async () => {
  await withWorld(async (world) => {
    const top = createPage(world, 'Attach top', '<h1>Top</h1>');
    const middle = createPage(world, 'Attach middle', '<h1>Middle</h1>');
    const grand = createPage(world, 'Grand page', '<h1>Grand</h1>');
    const colab = (args: string[]) =>
      JSON.parse(run(world, world.binaries.colab, [...args, '--json']));
    colab(['subpage', 'attach', top.pageId, middle.pageId]);
    const door = await startDoor(world);
    const browser = await pairBrowser(world, 'attach-owner');
    // The middle page already has a parent and still takes a child of its own (#5820).
    const page = await openPage(door, browser, middle);
    const band = page.getByRole('region', { name: 'Work on this page' });
    await band.getByRole('button', { name: 'Add', exact: true }).click();
    const dialog = page.getByRole('dialog', { name: 'Add a sub-page to “Attach middle”' });
    await dialog.getByRole('combobox').fill('Attach');
    await expect(dialog.getByRole('option', { name: /Attach top/ })).toContainText(
      'Would loop: it is above this page',
    );
    await dialog.getByRole('combobox').fill('Grand');
    await dialog.getByRole('option', { name: /Grand page/ }).click();
    await expect(dialog).toHaveCount(0);
    const listed = () => colab(['subpage', 'ls', middle.pageId]).children as string[];
    expect(listed()).toEqual([grand.pageId]);
    await band.getByRole('button', { name: 'Expand work panel' }).click();
    await expect(band.getByRole('link', { name: 'Grand page', exact: true })).toBeVisible();
    // Create makes the page with the typed name and attaches it.
    await band.locator('.work-add-row').getByRole('button', { name: 'Add', exact: true }).click();
    await dialog.getByRole('combobox').fill('Made here');
    await page.keyboard.press('Enter');
    await expect(dialog).toHaveCount(0);
    const [, made] = listed();
    expect(made).toBeDefined();
    expect(colab(['show', made]).page.title).toBe('Made here');
    // Detach removes the link only: the page stays.
    await band.getByRole('link', { name: 'Grand page', exact: true }).hover();
    await band.getByRole('button', { name: 'More for Grand page' }).click();
    await page.getByRole('menuitem', { name: 'Detach' }).click();
    await expect(band.getByRole('link', { name: 'Grand page', exact: true })).toHaveCount(0);
    expect(listed()).toEqual([made]);
    expect(colab(['show', grand.pageId]).page.title).toBe('Grand page');
  });
});
