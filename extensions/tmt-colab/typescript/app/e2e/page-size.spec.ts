import { pageAction } from '../test/page-actions.js';
import { expect, test, type Locator, type Page } from '@playwright/test';
import { capturePath } from './captures.js';

/** What a spec changes in the scripted serve: usage and report are partial overrides. */
interface Extra {
  usage?: Record<string, unknown>;
  report?: Record<string, unknown> | null;
  refuseSave?: boolean;
  reader?: boolean;
}
async function mount(page: Page, extra: Extra = {}) {
  await page.goto('/');
  await page.evaluate(async (extra) => {
    const path = '/test/page-size-browser.tsx';
    const fixture = await import(path);
    const { usage, report, ...rest } = extra;
    fixture.mountPageSize({
      ...(usage ? { usage: fixture.usageOf(usage) } : {}),
      ...(report === undefined
        ? {}
        : { report: report === null ? null : fixture.reportOf(report) }),
      ...rest,
    });
  }, extra);
  await expect(page.locator('.page-header-actions:visible')).toBeVisible();
}
const calls = (page: Page) => page.evaluate(() => ({ ...window.__pageSize!.calls }));
async function about(page: Page): Promise<Locator> {
  await (await pageAction(page, 'About this page')).click();
  return page.locator('#page-size-fixture .tmt-ui-drawer[data-panel=about]');
}
async function source(page: Page): Promise<Locator> {
  await (await pageAction(page, 'Source')).click();
  return page.locator('#page-size-fixture .tmt-ui-drawer[data-panel=source]');
}
const section = (drawer: Locator) => drawer.locator('.page-size');

test('the About drawer shows Page size with its numbers and combines on request', async ({
  page,
}) => {
  await mount(page);
  const drawer = await about(page);
  await expect(section(drawer).getByRole('heading', { name: 'Page size' })).toBeVisible();
  await expect(section(drawer)).toContainText('62% full');
  await expect(section(drawer).locator('dt')).toHaveText(['Changes', 'Recent data']);
  await expect(section(drawer).locator('dd')).toHaveText(['124 of 200', '1.2 MiB of 4 MiB']);
  await expect(drawer).not.toContainText('Stored size');
  await section(drawer).getByRole('button', { name: 'Compact page' }).click();
  await expect(section(drawer).getByRole('status')).toHaveText(
    'Combined 124 changes into one. Content is unchanged. Now 0% full, was 62%.',
  );
  await expect(section(drawer)).toContainText('0% full');
  expect(await calls(page)).toMatchObject({ compact: 1, edit: 0 });
});

test('stored size appears once the gzipped state is measured, and other devices are named', async ({
  page,
}) => {
  await mount(page, {
    usage: {
      gzipBytes: 2_411_000,
      stateBytes: 5_000_000,
      otherDeviceUpdates: 3,
      percent: 48,
    },
  });
  const drawer = await about(page);
  await expect(section(drawer).locator('dt')).toHaveText(['Changes', 'Recent data', 'Stored size']);
  await expect(section(drawer).locator('dd').nth(2)).toHaveText('2.3 MiB of 4.77 MiB');
  await expect(section(drawer)).toContainText(
    '3 changes are on other devices. Only those devices can combine them.',
  );
  await page.evaluate(() => {
    window.__pageSize!.usage = { ...window.__pageSize!.usage, otherDeviceUpdates: 1 };
  });
  await drawer.getByRole('button', { name: 'Close About this page' }).click();
  const again = await about(page);
  await expect(section(again)).toContainText(
    '1 change is on another device. Only that device can combine it.',
  );
});

test('a reader is never shown usage or Compact anywhere', async ({ page }) => {
  await mount(page, { reader: true });
  const editor = await source(page);
  await expect(editor.locator('textarea')).toBeVisible();
  await expect(editor.locator('.source-notice')).toHaveCount(0);
  await editor.getByRole('button', { name: 'Close Source', exact: true }).click();
  const drawer = await about(page);
  await expect(drawer).toContainText('About this page');
  await expect(section(drawer)).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Compact page' })).toHaveCount(0);
  expect(await calls(page)).toMatchObject({ usage: 0, compact: 0 });
});

test('at 80% the Source panel shows one quiet notice that cannot be dismissed', async ({
  page,
}) => {
  await mount(page, { usage: { updates: 172, percent: 86 } });
  const editor = await source(page);
  const notice = editor.locator('.source-notice');
  await expect(notice).toContainText("86% of this page's room is used.");
  await expect(notice).toContainText('Nearly full');
  await expect(notice.locator('[role=status]')).toHaveCount(1);
  await expect(notice.getByRole('alert')).toHaveCount(0);
  await expect(notice.getByRole('button')).toHaveText(['Compact page']);
  await editor.getByRole('button', { name: 'Close Source', exact: true }).click();
  const drawer = await about(page);
  await expect(section(drawer)).toContainText('◆ Nearly full · 86%');
  // Below 80 the notice leaves with the number.
  await page.evaluate(() => {
    window.__pageSize!.usage = { ...window.__pageSize!.usage, updates: 40, percent: 20 };
  });
  await drawer.getByRole('button', { name: 'Close About this page' }).click();
  const quiet = await source(page);
  await expect(quiet.locator('.source-notice')).toHaveCount(0);
});

test('when combining cannot free room the notice says so and offers Export instead', async ({
  page,
}) => {
  await mount(page, {
    usage: { limiting: 'stored-size', percent: 91, gzipBytes: 4_500_000, stateBytes: 9_000_000 },
  });
  const editor = await source(page);
  const notice = editor.locator('.source-notice');
  await expect(notice).toContainText('Combining cannot free more room.');
  await expect(notice.getByRole('button')).toHaveText(['Export page']);
  await expect(page.getByRole('button', { name: 'Compact page' })).toHaveCount(0);
  await notice.getByRole('button', { name: 'Export page' }).click();
  await expect(page.locator('#page-size-fixture .tmt-ui-drawer[data-panel=export]')).toBeVisible();
  // Nothing local to combine is the same: only other devices hold the changes.
  await mount(page, { usage: { percent: 85, updates: 170, otherDeviceUpdates: 170 } });
  const again = await source(page);
  await expect(again.locator('.source-notice')).toContainText('Combining cannot free more room.');
  expect(await calls(page)).toMatchObject({ compact: 0 });
});

test('each compaction result is worded as the UX rulings say', async ({ page }) => {
  const result = async (extra: Extra) => {
    await mount(page, extra);
    const drawer = await about(page);
    await section(drawer).getByRole('button', { name: 'Compact page' }).click();
    return section(drawer);
  };
  const high = await result({
    usage: { percent: 92 },
    report: { after: { percent: 88 } },
  });
  await expect(high.getByRole('status')).toHaveText(
    'Combined, but this page is still 88% full. Combining keeps every change, so it frees no more room. Export this page and continue in a new one.',
  );
  await expect(high.getByRole('button')).toHaveText(['Export page']);
  const nothing = await result({
    usage: { otherDeviceUpdates: 2, updates: 5 },
    report: {
      compacted: false,
      skipped: 'nothing-to-combine',
      mergedObjects: 0,
      after: { otherDeviceUpdates: 2, updates: 5, percent: 2 },
    },
  });
  await expect(nothing.getByRole('status').first()).toHaveText(
    'Nothing to combine on this device.',
  );
  await expect(nothing).toContainText('2 changes are on other devices.');
  await expect(nothing.getByRole('button')).toHaveCount(0);
  const failed = await result({ report: null });
  await expect(failed.getByRole('status')).toHaveText(
    'Could not combine this page. Nothing was changed. Try again.',
  );
  await expect(failed.getByRole('button')).toHaveText(['Compact page']);
  const notCombined = await result({
    report: { compacted: false, skipped: 'not-combined', mergedObjects: 0 },
  });
  await expect(notCombined.getByRole('status')).toHaveText(
    'Could not combine this page. Nothing was changed. Try again.',
  );
});

test('a save refused for a full page offers Compact, keeps the draft and never saves again by itself', async ({
  page,
}) => {
  await mount(page, { refuseSave: true, usage: { updates: 200, percent: 100 } });
  const editor = await source(page);
  const box = editor.locator('textarea');
  await box.fill('<h1>My unsaved edit</h1>');
  await editor.getByRole('button', { name: 'Save' }).click();
  const notice = editor.locator('.source-notice[role], .source-notice').last();
  await expect(editor).toContainText('Not saved');
  await expect(editor).toContainText('This page is full. Compact it, then save again.');
  // The refusal carries the way out, so the quiet notice does not repeat it above.
  await expect(editor.getByRole('button', { name: 'Compact page' })).toHaveCount(1);
  expect(await calls(page)).toMatchObject({ edit: 1, compact: 0 });
  await editor.getByRole('button', { name: 'Compact page' }).last().click();
  await expect(editor).toContainText('Compacted. Save again to keep your edit.');
  await expect(box).toHaveValue('<h1>My unsaved edit</h1>');
  // Compacting did not save, and nothing saves until the person presses Save.
  await page.waitForTimeout(300);
  expect(await calls(page)).toMatchObject({ edit: 1, compact: 1 });
  await page.evaluate(() => {
    window.__pageSize!.refuseSave = false;
  });
  await editor.getByRole('button', { name: 'Save' }).click();
  await expect.poll(async () => (await calls(page)).edit).toBe(2);
  await expect(editor).not.toContainText('Not saved');
  void notice;
});

test('a refusal that combining cannot fix offers Export, and a failed Compact keeps the draft', async ({
  page,
}) => {
  await mount(page, {
    refuseSave: true,
    usage: { limiting: 'stored-size', percent: 100, gzipBytes: 5_000_000, stateBytes: 9_000_000 },
  });
  const editor = await source(page);
  await editor.locator('textarea').fill('<p>kept</p>');
  await editor.getByRole('button', { name: 'Save' }).click();
  await expect(editor).toContainText(
    'This page is full, and combining cannot free room. Export it and continue in a new page.',
  );
  await expect(editor.getByRole('button', { name: 'Compact page' })).toHaveCount(0);
  await expect(editor.getByRole('button', { name: 'Export page' }).last()).toBeVisible();

  await mount(page, { refuseSave: true, report: null, usage: { updates: 200, percent: 100 } });
  const second = await source(page);
  const box = second.locator('textarea');
  await box.fill('<p>kept too</p>');
  await second.getByRole('button', { name: 'Save' }).click();
  await second.getByRole('button', { name: 'Compact page' }).last().click();
  await expect(second).toContainText(
    'Could not combine this page. Nothing was changed. Try again.',
  );
  await expect(box).toHaveValue('<p>kept too</p>');
  expect(await calls(page)).toMatchObject({ edit: 1, compact: 1 });
});

test('a save refused for a full page tells a device that cannot compact who can', async ({
  page,
}) => {
  await mount(page, { refuseSave: true, reader: true });
  const editor = await source(page);
  await editor.locator('textarea').fill('<p>draft</p>');
  await editor.getByRole('button', { name: 'Save' }).click();
  await expect(editor).toContainText('Not saved');
  await expect(editor).toContainText('An editor must compact this page.');
  await expect(editor.getByRole('button', { name: 'Compact page' })).toHaveCount(0);
  await expect(editor.getByRole('button', { name: 'Export page' })).toHaveCount(0);
  await expect(editor.locator('textarea')).toHaveValue('<p>draft</p>');
  expect(await calls(page)).toMatchObject({ edit: 1, compact: 0, usage: 0 });
});

for (const [width, theme] of [
  [1440, 'light'],
  [390, 'dark'],
] as const)
  test(`captures: page size states ${width} ${theme}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 844 });
    await page.emulateMedia({ colorScheme: theme });
    await page.addInitScript((value) => {
      document.documentElement.dataset.theme = value;
    }, theme);
    const shot = async (name: string) =>
      page.screenshot({ path: capturePath(`2846-${name}-${width}-${theme}.png`) });
    await mount(page);
    let drawer = await about(page);
    await expect(section(drawer)).toBeVisible();
    await shot('below-80');
    await mount(page, { usage: { updates: 172, percent: 86 } });
    const editor = await source(page);
    await expect(editor.locator('.source-notice')).toBeVisible();
    await shot('notice-86');
    await editor.getByRole('button', { name: 'Close Source', exact: true }).click();
    drawer = await about(page);
    await shot('about-86');
    await mount(page, { usage: { updates: 200, percent: 100 } });
    drawer = await about(page);
    await shot('full-100');
    await mount(page, { usage: { limiting: 'stored-size', percent: 91, gzipBytes: 4_500_000 } });
    drawer = await about(page);
    await shot('cannot-free');
    // No horizontal scroll at the narrow width.
    expect(
      await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth),
    ).toBe(true);
  });

for (const [width, theme] of [
  [1440, 'light'],
  [390, 'dark'],
] as const)
  test(`captures: results and refusals ${width} ${theme}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 844 });
    await page.emulateMedia({ colorScheme: theme });
    await page.addInitScript((value) => {
      document.documentElement.dataset.theme = value;
    }, theme);
    const shot = async (name: string) =>
      page.screenshot({ path: capturePath(`2846-${name}-${width}-${theme}.png`) });
    const result = async (name: string, extra: Extra) => {
      await mount(page, extra);
      const drawer = await about(page);
      await section(drawer).getByRole('button', { name: 'Compact page' }).click();
      await expect(section(drawer).getByRole('status').first()).toBeVisible();
      await shot(name);
    };
    await result('result-combined', {});
    // What a page full of text it keeps looks like: combining empties the tail, the stored size stays.
    const stored = { limiting: 'stored-size', gzipBytes: 4_400_000, stateBytes: 9_000_000 };
    await result('result-still-high', {
      usage: { updates: 180, percent: 90, limiting: 'changes' },
      report: { after: { ...stored, updates: 0, tailBytes: 0, percent: 88 } },
    });
    const nothing = {
      compacted: false,
      skipped: 'nothing-to-combine',
      mergedObjects: 0,
    };
    await result('result-nothing', {
      usage: { updates: 5 },
      report: { ...nothing, after: { updates: 5, percent: 2 } },
    });
    await result('result-nothing-other-devices', {
      usage: { otherDeviceUpdates: 2, updates: 5 },
      report: { ...nothing, after: { otherDeviceUpdates: 2, updates: 5, percent: 2 } },
    });
    await result('result-failed', { report: null });
    // The Save refusal: tail cause, budget cause, a device that cannot compact, then Compacted.
    const refused = async (name: string, extra: Extra, draft: string) => {
      await mount(page, { refuseSave: true, ...extra });
      const editor = await source(page);
      await editor.locator('textarea').fill(draft);
      await editor.getByRole('button', { name: 'Save' }).click();
      await expect(editor).toContainText('Not saved');
      await shot(name);
      return editor;
    };
    const tail = await refused(
      'refusal-tail',
      { usage: { updates: 200, percent: 100 } },
      '<h1>My unsaved edit</h1>',
    );
    await tail.getByRole('button', { name: 'Compact page' }).last().click();
    await expect(tail).toContainText('Compacted. Save again to keep your edit.');
    await expect(tail.locator('textarea')).toHaveValue('<h1>My unsaved edit</h1>');
    await shot('refusal-compacted');
    await refused(
      'refusal-budget',
      {
        usage: {
          limiting: 'stored-size',
          percent: 100,
          gzipBytes: 5_000_000,
          stateBytes: 9_000_000,
        },
      },
      '<h1>My unsaved edit</h1>',
    );
    await refused('refusal-reader', { reader: true }, '<h1>My unsaved edit</h1>');
    // The Source notice when combining cannot help.
    await mount(page, {
      usage: { limiting: 'stored-size', percent: 91, gzipBytes: 4_500_000, stateBytes: 9_000_000 },
    });
    const editor = await source(page);
    await expect(editor.locator('.source-notice')).toContainText(
      'Combining cannot free more room.',
    );
    await shot('notice-cannot-free');
    expect(
      await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth),
    ).toBe(true);
  });
