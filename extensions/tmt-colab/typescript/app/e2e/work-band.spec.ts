import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';

declare global {
  interface Window {
    __workPage?: import('../test/work-page-browser.js').WorkPageHook;
  }
}

async function mountBand(page: Page, name: string) {
  await page.goto('/');
  await page.evaluate(async (name) => {
    const path = '/test/work-band-browser.tsx';
    (await import(path)).mount(name);
  }, name);
  return page.locator('#work-band-fixture');
}

test('the band shows the summary, progress, who works and sign-off, with the list closed', async ({
  page,
}) => {
  const host = await mountBand(page, 'full');
  const band = host.getByRole('region', { name: 'Work on this page' });
  await expect(band.locator('.work-line1')).toHaveText('Building the export');
  await expect(band.locator('.work-line2')).toHaveText('Two tasks left before review');
  await expect(band.locator('.work-stage')).toHaveText('In progress');
  await expect(band.locator('.work-progress')).toHaveText('1 of 7 done');
  await expect(band.locator('.work-working .agent-chip-name')).toHaveText('Atlas');
  await expect(band.locator('.work-signoff')).toHaveText('1 waiting for sign-off');
  await expect(band.getByRole('button', { name: 'Show 7 tasks' })).toHaveAttribute(
    'aria-expanded',
    'false',
  );
  await expect(band.locator('.work-list')).toHaveCount(0);
});

test('opening the list pushes the page down in place; the band does not stick', async ({
  page,
}) => {
  const host = await mountBand(page, 'full');
  const after = host.locator('[data-after]');
  const closed = (await after.boundingBox())!.y;
  await host.getByRole('button', { name: 'Show 7 tasks' }).click();
  await expect(host.locator('.work-row')).toHaveCount(7);
  expect((await after.boundingBox())!.y).toBeGreaterThan(closed + 100);
  await expect(host.getByRole('button', { name: 'Hide tasks' })).toHaveAttribute(
    'aria-expanded',
    'true',
  );
  expect(await host.locator('.work-band').evaluate((n) => getComputedStyle(n).position)).toBe(
    'static',
  );
  await host.getByRole('button', { name: 'Hide tasks' }).click();
  await expect(host.locator('.work-list')).toHaveCount(0);
  expect((await after.boundingBox())!.y).toBe(closed);
});

test('a task opens its detail in the list and closes again, with the action slot beside it', async ({
  page,
}) => {
  const host = await mountBand(page, 'full');
  await host.getByRole('button', { name: 'Show 7 tasks' }).click();
  const row = host.locator('.work-row[data-state=in-progress]');
  await expect(row).toContainText('Wire the export to the page menu');
  await row.getByRole('button').click();
  const detail = host.getByRole('region', { name: 'Task T-2' });
  await expect(detail).toContainText('Keep the menu entry');
  await expect(detail.locator('.work-acceptance li')).toHaveText([
    'Test: export.spec',
    'pnpm check → passed',
    'Confirmed by Ben',
  ]);
  await expect(detail.getByRole('button', { name: 'Edit T-2' })).toBeVisible();
  await detail.getByRole('button', { name: 'Close' }).click();
  await expect(detail).toHaveCount(0);
});

test('state and assignee sit in fixed columns; the selected task is marked without link colour', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const host = await mountBand(page, 'full');
  await host.getByRole('button', { name: 'Show 7 tasks' }).click();
  const columns = await host.locator('.work-row').evaluateAll((rows) =>
    rows.map((row) => {
      const x = (selector: string) => row.querySelector(selector)!.getBoundingClientRect();
      return { state: x('.work-state').left, assignee: x('.work-assignee').left };
    }),
  );
  expect(new Set(columns.map((row) => row.state)).size).toBe(1);
  expect(new Set(columns.map((row) => row.assignee)).size).toBe(1);
  // The sign-off mark sits on the task that waits for it, and only there.
  await expect(host.locator('.work-row .work-signoff-mark')).toHaveCount(1);
  await expect(host.locator('.work-row[data-state=in-review] .work-signoff-mark')).toHaveText(
    '◆ sign-off',
  );
  const row = host.locator('.work-row', { hasText: 'Review the file format' });
  const button = row.getByRole('button');
  const plain = await button.evaluate((node) => ({
    background: getComputedStyle(node).backgroundColor,
  }));
  await button.click();
  const selected = await button.evaluate((node) => {
    const style = getComputedStyle(node);
    const text = getComputedStyle(node.querySelector('.work-title')!);
    return { background: style.backgroundColor, weight: style.fontWeight, color: text.color };
  });
  expect(selected.background).not.toBe(plain.background);
  expect(selected.weight).toBe('500');
  const body = await host.evaluate((node) => getComputedStyle(node).color);
  expect(selected.color).toBe(body);
});

test('blocked, sign-off, follow-up page and tracker id each show where they belong', async ({
  page,
}) => {
  const host = await mountBand(page, 'full');
  await host.getByRole('button', { name: 'Show 7 tasks' }).click();
  await host.locator('.work-row[data-state=blocked] button').click();
  await expect(host.locator('.work-detail-state')).toHaveText('Blocked · Waiting on licence');
  await host.locator('.work-row[data-state=in-review] button').click();
  await expect(host.locator('.work-detail-state')).toHaveText('In review · ◆ sign-off');
  await host
    .locator('.work-row', { hasText: 'Follow-up page for sizes' })
    .locator('button')
    .click();
  await expect(host.locator('.work-follows a')).toHaveAttribute('href', /#\/pages\/2000/);
  await host.locator('.work-row', { hasText: 'Tracked elsewhere' }).locator('button').click();
  await expect(host.locator('.work-external')).toHaveText('Tracked as GH-123');
});

test('a page that follows a task shows the way back above its summary', async ({ page }) => {
  const host = await mountBand(page, 'parent');
  const link = host.locator('.work-parent a');
  await expect(link).toHaveText('Follows T-6 · Follow-up page for sizes');
  await expect(link).toHaveAttribute('href', /#\/pages\/2000/);
});

test('a summary alone shows two lines; no summary and no tasks shows nothing', async ({ page }) => {
  let host = await mountBand(page, 'summary-only');
  await expect(host.locator('.work-line1')).toHaveText('Reading notes');
  await expect(host.locator('.work-toggle')).toHaveCount(0);
  await expect(host.locator('.work-facts')).toHaveCount(0);
  host = await mountBand(page, 'empty');
  await expect(host.locator('.work-band')).toHaveCount(0);
});

async function mountRoute(page: Page, withWork: boolean, owner = false) {
  await page.goto('/');
  await page.evaluate(
    async ([withWork, owner]) => {
      const path = '/test/work-page-browser.tsx';
      (await import(path)).mountWorkPage(withWork, owner);
    },
    [withWork, owner],
  );
  await expect(page.locator('.page-header-actions:visible')).toBeVisible();
}

test('the page route draws the band under the header and follows live work', async ({ page }) => {
  await mountRoute(page, true);
  const band = page.getByRole('region', { name: 'Work on this page' });
  await expect(band.locator('.work-line1')).toHaveText('Release notes');
  await expect(band.locator('.work-progress')).toHaveText('0 of 2 done');
  const before = (await band.boundingBox())!;
  const header = (await page.locator('.page-header-actions:visible').boundingBox())!;
  expect(before.y).toBeGreaterThanOrEqual(header.y);
  await page.evaluate(() => window.__workPage!.finish());
  await expect(band.locator('.work-progress')).toHaveText('1 of 2 done');
});

test('a page that never had work draws no band', async ({ page }) => {
  await mountRoute(page, false);
  await expect(page.locator('.work-band')).toHaveCount(0);
});

for (const [width, height] of [
  [1440, 900],
  [390, 844],
] as const) {
  for (const scheme of ['light', 'dark'] as const) {
    test(`look at ${width} in ${scheme}`, async ({ page }) => {
      await page.setViewportSize({ width, height });
      await page.emulateMedia({ colorScheme: scheme });
      const host = await mountBand(page, 'full');
      await host.getByRole('button', { name: 'Show 7 tasks' }).click();
      await host.locator('.work-row[data-state=in-progress] button').click();
      expect(
        await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth),
      ).toBe(true);
      await page.screenshot({ path: capturePath(`work-band-${width}-${scheme}.png`) });
    });
  }
}

async function openTask(page: Page, id: string) {
  await mountRoute(page, true, true);
  const band = page.getByRole('region', { name: 'Work on this page' });
  await band.getByRole('button', { name: /Show \d+ tasks/ }).click();
  await band.locator('.work-row', { hasText: id }).locator('button').click();
  return band;
}
const calls = (page: Page) => page.evaluate(() => window.__workPage!.calls);

test('a device that cannot change work sees no controls', async ({ page }) => {
  await mountRoute(page, true, false);
  const band = page.getByRole('region', { name: 'Work on this page' });
  await band.getByRole('button', { name: 'Show 2 tasks' }).click();
  await band.locator('.work-row button').first().click();
  await expect(band.locator('.work-detail')).toBeVisible();
  await expect(band.locator('.work-controls')).toHaveCount(0);
  await expect(band.locator('.work-add')).toHaveCount(0);
});

test('a state change is sent as one action and shows once the page brings it back', async ({
  page,
}) => {
  const band = await openTask(page, 'T-2');
  await band.getByLabel('State').selectOption('in-progress');
  await expect(band.locator('.work-row', { hasText: 'T-2' }).locator('.work-state')).toHaveText(
    'In progress',
  );
  expect(await calls(page)).toEqual([
    { action: 'task.state', body: { taskId: 'T-2', state: 'in-progress' } },
  ]);
});

test('blocking asks for a reason and sends it with the state', async ({ page }) => {
  const band = await openTask(page, 'T-2');
  await band.getByLabel('State').selectOption('blocked');
  await expect(band.getByRole('button', { name: 'Block' })).toBeDisabled();
  await band.getByLabel('Reason').fill('Waiting for the legal copy');
  await band.getByRole('button', { name: 'Block' }).click();
  await expect(band.locator('.work-detail-state')).toContainText(
    'Blocked · Waiting for the legal copy',
  );
  expect(await calls(page)).toEqual([
    {
      action: 'task.state',
      body: { taskId: 'T-2', state: 'blocked', reason: 'Waiting for the legal copy' },
    },
  ]);
});

test('sign-off closes a flagged task; sending it back reopens it', async ({ page }) => {
  const band = await openTask(page, 'T-3');
  await expect(band.locator('.work-signoff')).toHaveText('1 waiting for sign-off');
  await band.getByRole('button', { name: 'Sign off' }).click();
  await expect(band.locator('.work-signoff')).toHaveCount(0);
  await expect(band.locator('.work-row', { hasText: 'T-3' }).locator('.work-state')).toHaveText(
    'Done',
  );
  expect(await calls(page)).toEqual([
    { action: 'task.state', body: { taskId: 'T-3', state: 'done' } },
  ]);
});

test('assigning names the assignee, and unassigning clears it', async ({ page }) => {
  const band = await openTask(page, 'T-2');
  await band.getByLabel('Name').fill('Atlas');
  await band.getByRole('button', { name: 'Assign', exact: true }).click();
  await expect(band.locator('.work-detail-assignee .agent-chip-name')).toHaveText('Atlas');
  await band.getByRole('button', { name: 'Unassign' }).click();
  await expect(band.locator('.work-detail-assignee')).toHaveCount(0);
  expect(await calls(page)).toEqual([
    { action: 'task.assign', body: { taskId: 'T-2', assignee: { kind: 'agent', name: 'Atlas' } } },
    { action: 'task.assign', body: { taskId: 'T-2' } },
  ]);
});

test("a refusal is told in the serve's words and changes nothing", async ({ page }) => {
  const band = await openTask(page, 'T-2');
  await page.evaluate(() => {
    window.__workPage!.refuse = {
      code: 'COLAB_WIP_LIMIT',
      message: 'Atlas would hold T-1, T-2 (limit 1)',
    };
  });
  await band.getByLabel('Name').fill('Atlas');
  await band.getByRole('button', { name: 'Assign', exact: true }).click();
  await expect(band.getByRole('alert')).toHaveText('Atlas would hold T-1, T-2 (limit 1)');
  await expect(band.locator('.work-detail-assignee')).toHaveCount(0);
  // The next action starts clean.
  await band.getByRole('button', { name: 'Assign', exact: true }).click();
  await expect(band.getByRole('alert')).toHaveCount(0);
  await expect(band.locator('.work-detail-assignee .agent-chip-name')).toHaveText('Atlas');
});

test('editing sends the whole editable set, keeping what the form does not show', async ({
  page,
}) => {
  const band = await openTask(page, 'T-3');
  await band.getByRole('button', { name: 'Edit', exact: true }).click();
  await band.getByLabel('Title').fill('Copy, final');
  await band.getByLabel('Detail').fill('Two paragraphs.');
  await band.getByRole('button', { name: 'Save' }).click();
  await expect(band.locator('.work-detail h3')).toContainText('Copy, final');
  await expect(band.locator('.work-detail-text')).toHaveText('Two paragraphs.');
  expect(await calls(page)).toEqual([
    {
      action: 'task.set',
      body: { taskId: 'T-3', title: 'Copy, final', detail: 'Two paragraphs.', needsSignoff: true },
    },
  ]);
});

test('removing needs a second click, and adding names no ID', async ({ page }) => {
  const band = await openTask(page, 'T-2');
  await band.getByRole('button', { name: 'Remove task' }).click();
  expect(await calls(page)).toEqual([]);
  await band.getByRole('button', { name: 'Remove T-2' }).click();
  await expect(band.locator('.work-row')).toHaveCount(2);
  await band.getByLabel('New task').fill('Publish');
  await band.getByRole('button', { name: 'Add task' }).click();
  await expect(band.locator('.work-row', { hasText: 'Publish' })).toContainText('T-4');
  expect((await calls(page)).map((entry) => entry.action)).toEqual(['task.remove', 'task.add']);
  expect((await calls(page))[1].body).toEqual({ task: { title: 'Publish' } });
});

for (const [width, height] of [
  [1440, 900],
  [390, 844],
] as const) {
  for (const scheme of ['light', 'dark'] as const) {
    test(`the owner's controls at ${width} in ${scheme}`, async ({ page }) => {
      await page.setViewportSize({ width, height });
      await page.emulateMedia({ colorScheme: scheme });
      const band = await openTask(page, 'T-3');
      await page.evaluate(() => {
        window.__workPage!.refuse = {
          code: 'COLAB_WIP_LIMIT',
          message: 'Atlas would hold T-1, T-2 (limit 1)',
        };
      });
      await band.getByLabel('Name').fill('Atlas');
      await band.getByRole('button', { name: 'Assign', exact: true }).click();
      await expect(band.getByRole('alert')).toBeVisible();
      expect(
        await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth),
      ).toBe(true);
      await band.scrollIntoViewIfNeeded();
      await page.screenshot({ path: capturePath(`work-controls-${width}-${scheme}.png`) });
    });
  }
}
