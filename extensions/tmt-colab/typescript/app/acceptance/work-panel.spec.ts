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

test('manual state persists independently of native progress and share readers have no progress or owner controls', async () => {
  await withWorld(async (world) => {
    const created = createPage(world, 'Status panel', '<h1>Status panel source</h1>');
    const colab = (args: string[]) =>
      JSON.parse(run(world, world.binaries.colab, [...args, '--json']));
    colab(['task', 'add', created.pageId, '--title', 'First task']);
    colab(['task', 'add', created.pageId, '--title', 'Second task']);
    colab(['task', 'state', created.pageId, 'T-1', 'done']);
    const before = colab(['show', created.pageId]);
    expect(before.page.state).toBe('todo');
    expect(before.page.progress).toEqual({ done: 1, inProgress: 0, todo: 1, unknown: 0, total: 2 });
    const door = await startDoor(world);
    const browser = await pairBrowser(world, 'status-owner');
    const page = await openPage(door, browser, created);
    const band = page.getByRole('region', { name: 'Work on this page' });
    await expect(band.locator('.work-progress')).toHaveText('1 of 2 done');
    await band.getByRole('button', { name: /Page state/ }).click();
    await page.getByRole('menuitemradio', { name: 'Done', exact: true }).click();
    await expect(band.getByRole('button', { name: /Page state/ })).toContainText('Done');
    const after = colab(['show', created.pageId]);
    expect(after.page.state).toBe('done');
    expect(after.page.progress).toEqual(before.page.progress);
    await page.reload();
    await expect(band.getByRole('button', { name: /Page state/ })).toContainText('Done');
    await expect(band.locator('.work-progress')).toHaveText('1 of 2 done');
    const captures = process.env.COLAB_STATUS_CAPTURE_DIR;
    if (captures) {
      mkdirSync(captures, { recursive: true });
      for (const width of [390, 1440])
        for (const theme of ['light', 'dark']) {
          await page.setViewportSize({ width, height: 900 });
          await panelTheme(page, band.getByRole('button', { name: /Page state/ }), theme);
          await page.screenshot({
            path: `${captures}/status-owner-collapsed-${width}-${theme}.png`,
          });
          await band.getByRole('button', { name: 'Expand status panel' }).click();
          await page.screenshot({
            path: `${captures}/status-owner-expanded-${width}-${theme}.png`,
          });
          await band.getByRole('button', { name: 'Collapse status panel' }).click();
        }
    }
    colab(['share', 'mode', created.pageId, 'link', '--yes']);
    const link = colab(['share', 'link', 'add', created.pageId, '--yes']);
    const reader = await openReaderLink(world, door, link.readerPath, 'status-reader');
    const shared = reader.page.getByRole('region', { name: 'Work on this page' });
    await expect(shared.locator('.work-page-state')).toHaveText('Done');
    await expect(shared.locator('.work-progress')).toHaveText('Progress unavailable');
    await expect(shared.getByRole('button', { name: /Page state/ })).toHaveCount(0);
    await shared.getByRole('button', { name: 'Expand status panel' }).click();
    await expect(shared.getByText('Attach a sub-page')).toHaveCount(0);
    await expect(shared.getByRole('button', { name: /Edit|Add task/ })).toHaveCount(0);
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

test('opening a page with sub-pages reads details once without an unavailable progress frame', async () => {
  await withWorld(async (world) => {
    const parent = createPage(world, 'Status parent', '<h1>Status parent source</h1>');
    const child = createPage(world, 'Status child', '<h1>Status child source</h1>');
    const colab = (args: string[]) =>
      JSON.parse(run(world, world.binaries.colab, [...args, '--json']));
    colab(['task', 'add', parent.pageId, '--title', 'Child work']);
    colab(['task', 'link', parent.pageId, 'T-1', child.pageId]);
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
        const value = document.querySelector('.work-band .work-progress')?.textContent;
        if (value) frames.push(value);
      }).observe(document, { childList: true, subtree: true, characterData: true });
    });
    const page = await openPage(door, browser, parent);
    const band = page.getByRole('region', { name: 'Work on this page' });
    await expect(band.locator('.work-progress')).toHaveText('0 of 1 done');
    await band.getByRole('button', { name: 'Expand status panel', exact: true }).click();
    await expect(band.getByRole('link', { name: 'Status child', exact: true })).toBeVisible();
    expect(reads).toBe(1);
    const frames = await page.evaluate(
      () => (window as unknown as { statusProgressFrames: string[] }).statusProgressFrames,
    );
    expect(frames.length).toBeGreaterThan(0);
    expect(frames).not.toContain('Progress unavailable');
  });
});

test('attach keeps a new task after owner refusal and links that same task on retry', async () => {
  await withWorld(async (world) => {
    const parent = createPage(world, 'Attach target', '<h1>Parent</h1>');
    const other = createPage(world, 'Other parent', '<h1>Other</h1>');
    const child = createPage(world, 'Existing child', '<h1>Child</h1>');
    const colab = (args: string[]) =>
      JSON.parse(run(world, world.binaries.colab, [...args, '--json']));
    colab(['task', 'add', other.pageId, '--title', 'Existing parent task']);
    colab(['task', 'link', other.pageId, 'T-1', child.pageId]);
    const door = await startDoor(world);
    const browser = await pairBrowser(world, 'attach-owner');
    const page = await openPage(door, browser, parent);
    const band = page.getByRole('region', { name: 'Work on this page' });
    await band.getByRole('button', { name: 'Expand status panel' }).click();
    await band.getByRole('button', { name: 'Attach a sub-page' }).click();
    const dialog = page.locator('dialog.tmt-ui-dialog[data-state="open"]');
    const choices = dialog.getByRole('combobox', { name: 'Page', exact: true });
    await expect(choices.locator('option', { hasText: 'Existing child' })).toHaveCount(1);
    await choices.selectOption({ label: 'Existing child' });
    await dialog.getByLabel('Task title').fill('Follow existing child');
    await dialog.getByRole('button', { name: 'Attach', exact: true }).click();
    await expect(dialog.getByRole('alert')).toBeVisible();
    await expect(dialog).toContainText('Task T-1 is kept');
    let tasks = colab(['task', 'ls', parent.pageId]).work.tasks;
    expect(tasks['T-1'].title).toBe('Follow existing child');
    expect(tasks['T-1'].child).toBeUndefined();
    colab(['task', 'unlink', other.pageId, 'T-1']);
    await dialog.getByRole('button', { name: 'Retry link' }).click();
    await expect(dialog).toHaveCount(0);
    tasks = colab(['task', 'ls', parent.pageId]).work.tasks;
    expect(Object.keys(tasks)).toEqual(['T-1']);
    expect(tasks['T-1'].child).toBe(child.pageId);
  });
});
