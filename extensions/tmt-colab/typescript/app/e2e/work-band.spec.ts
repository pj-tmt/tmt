import { openWorkTasks } from '../test/page-actions.js';
import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';

declare global {
  interface Window {
    __workPage?: import('../test/work-page-browser.js').WorkPageHook;
  }
}
async function mountBand(page: Page, name = 'full', focusTask?: string) {
  await page.goto('/');
  await page.evaluate(
    async ({ name, focusTask }) => {
      const path = '/test/work-band-browser.tsx';
      (await import(path)).mount(name, focusTask);
    },
    { name, focusTask },
  );
  return page.locator('#work-band-fixture');
}
async function mountRoute(
  page: Page,
  withWork: boolean,
  owner = false,
  scenario?:
    | 'linked-done'
    | 'linked-missing'
    | 'summary-only'
    | 'external'
    | 'empty'
    | 'child'
    | 'child-orphan'
    | 'attach',
) {
  await page.goto('/');
  await page.evaluate(
    async ([withWork, owner, scenario]) => {
      const path = '/test/work-page-browser.tsx';
      (await import(path)).mountWorkPage(withWork, owner, scenario);
    },
    [withWork, owner, scenario] as const,
  );
  await expect(page.locator('.page-header-actions:visible')).toBeVisible();
}
async function expand(page: Page) {
  const button = page.getByRole('button', { name: 'Expand status panel' });
  if (await button.count()) await button.click();
}
async function openTasks(page: Page) {
  await openWorkTasks(page.getByRole('region', { name: 'Work on this page' }));
}

async function choose(page: Page, menu: RegExp, item: string) {
  await page.getByRole('button', { name: menu }).click();
  await page.getByRole('menuitemradio', { name: item, exact: true }).click();
}
async function openTask(page: Page, id: string) {
  await mountRoute(page, true, true);
  await openTasks(page);
  const band = page.getByRole('region', { name: 'Work on this page' });
  await band.locator('.work-row', { hasText: id }).locator('button').click();
  return band;
}
const calls = (page: Page) => page.evaluate(() => window.__workPage!.calls);

test('collapsed panel uses manual state and native progress, including unknown', async ({
  page,
}) => {
  const host = await mountBand(page);
  await expect(host.locator('.work-page-state')).toHaveText('To do');
  await expect(host.locator('.work-progress')).toHaveText('1 of 7 done');
  await expect(host.getByRole('img')).toHaveAttribute(
    'aria-label',
    'Done 1, In progress 3, To do 2, Unknown 1',
  );
  await expect(host.getByRole('button', { name: 'Expand status panel' })).toHaveAttribute(
    'aria-expanded',
    'false',
  );
  await expect(host.locator('.work-list')).toHaveCount(0);
  await expect(host.getByText('Attach a sub-page')).toHaveCount(0);
});

test('an ordinary page without work still has a manual-state panel and unavailable progress', async ({
  page,
}) => {
  const host = await mountBand(page, 'empty');
  const band = host.getByRole('region', { name: 'Work on this page' });
  await expect(band).toBeVisible();
  await expect(band).toContainText('State unavailable');
  await expect(band).toContainText('Progress unavailable');
  await expect(band.getByRole('img')).toHaveCount(0);
  await expect(band.getByRole('button', { name: 'Expand status panel' })).toHaveAttribute(
    'aria-expanded',
    'false',
  );
});

test('empty progress is zero and complete progress never changes manual state', async ({
  page,
}) => {
  let host = await mountBand(page, 'zero');
  await expect(host.locator('.work-progress')).toHaveText('0 of 0 done');
  await expect(host.locator('.work-segments span')).toHaveCount(0);
  host = await mountBand(page, 'complete');
  await expect(host.locator('.work-progress')).toHaveText('5 of 5 done');
  await expect(host.locator('.work-page-state')).toHaveText('In progress');
});

test('expansion pushes content down, shows direct states and legend, and keeps tasks reachable', async ({
  page,
}) => {
  const host = await mountBand(page, 'panel');
  const before = (await host.locator('[data-after]').boundingBox())!.y;
  await expand(page);
  await expect(host.getByRole('list', { name: 'Sub-pages' }).locator('li')).toHaveCount(3);
  await expect(host.locator('.work-legend')).toHaveText('Done 3In progress 1To do 1');
  expect((await host.locator('[data-after]').boundingBox())!.y).toBeGreaterThan(before);
  expect(await host.locator('.work-band').evaluate((node) => getComputedStyle(node).position)).toBe(
    'static',
  );
  await openTasks(page);
  await expect(host.locator('.work-row')).toHaveCount(2);
  await page.getByRole('button', { name: 'Collapse status panel' }).click();
  expect((await host.locator('[data-after]').boundingBox())!.y).toBe(before);
});

test('task deep links open once and preserve detail, acceptance, assignee and controls', async ({
  page,
}) => {
  const host = await mountBand(page, 'full', 'T-2');
  await expect(host.locator('.work-detail')).toContainText('Keep the menu entry');
  await expect(host.locator('.work-acceptance li')).toHaveText([
    'Test: export.spec',
    'pnpm check → passed',
    'Confirmed by Ben',
  ]);
  await expect(host.getByRole('button', { name: 'Edit T-2' })).toBeVisible();
  await page.getByRole('button', { name: 'Tasks 6' }).click();
  await expect(host.locator('.work-list')).toHaveCount(0);
  await page.getByRole('button', { name: 'Tasks 6' }).click();
  await expect(host.locator('.work-row[data-selected=true]')).toContainText('Wire the export');
});

test('missing child list rows stay unknown, known rows use manual state', async ({ page }) => {
  await mountRoute(page, true, false, 'linked-done');
  await expand(page);
  await expect(page.getByRole('list', { name: 'Sub-pages' })).toHaveText('Legal copyDone');
  await mountRoute(page, true, false, 'linked-missing');
  await expand(page);
  await expect(page.getByRole('list', { name: 'Sub-pages' })).toHaveText('Unknown pageUnknown');
});

test('owner manual state uses metadata, persists through stream, and reports refusal', async ({
  page,
}) => {
  await mountRoute(page, true, true);
  await page.evaluate(() => {
    window.__workPage!.holdMetadata = true;
  });
  await choose(page, /Page state/, 'Done');
  await expect(page.getByRole('button', { name: /Page state/ })).toBeDisabled();
  await expect(page.getByRole('button', { name: /Page state/ })).toContainText('To do');
  await expect(
    page.getByRole('status', { name: '' }).filter({ hasText: 'Saving state…' }),
  ).toBeVisible();
  await page.evaluate(() => {
    window.__workPage!.holdMetadata = false;
    window.__workPage!.finishMetadata!();
  });
  await expect(page.getByRole('button', { name: /Page state/ })).toContainText('Done');
  await expect(page.locator('#work-page-fixture .work-progress')).toHaveText('0 of 3 done');
  expect(await calls(page)).toEqual([{ action: 'page.state', body: { state: 'done' } }]);
  await page.evaluate(() => {
    window.__workPage!.refuse = { code: 'COLAB_STALE_BASE', message: 'Read the state again.' };
  });
  await choose(page, /Page state/, 'To do');
  await expect(page.getByRole('alert')).toHaveText('Read the state again.');
  await expect(page.getByRole('button', { name: /Page state/ })).toContainText('Done');
});

test('viewers retain panel and task details without owner controls', async ({ page }) => {
  await mountRoute(page, false);
  await expect(page.locator('#work-page-fixture .work-band')).toBeVisible();
  await expect(page.getByRole('button', { name: /Page state/ })).toHaveCount(0);
  await mountRoute(page, true, false);
  await openTasks(page);
  await page.locator('.work-row button').first().click();
  await expect(page.locator('.work-detail')).toBeVisible();
  await expect(page.locator('.work-controls')).toHaveCount(0);
});

for (const width of [390, 1440])
  for (const scheme of ['light', 'dark'] as const) {
    test(`status panel at ${width} ${scheme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: scheme });
      const host = await mountBand(page, 'panel');
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
      await page.screenshot({ path: capturePath(`status-panel-collapsed-${width}-${scheme}.png`) });
      await page.getByRole('button', { name: 'Expand status panel' }).focus();
      await page.keyboard.press('Enter');
      await expect(host.getByRole('button', { name: 'Collapse status panel' })).toBeFocused();
      await expect(host.getByRole('list', { name: 'Sub-pages' })).toBeVisible();
      await page.screenshot({ path: capturePath(`status-panel-expanded-${width}-${scheme}.png`) });
      await mountBand(page, 'empty');
      await page.screenshot({
        path: capturePath(`status-panel-unavailable-${width}-${scheme}.png`),
      });
    });
  }
test('a state change is sent as one action and shows once the page brings it back', async ({
  page,
}) => {
  const band = await openTask(page, 'T-2');
  await choose(page, /^State/, 'In progress');
  await expect(band.locator('.work-row', { hasText: 'T-2' }).locator('.work-state')).toHaveText(
    'In progress',
  );
  expect(await calls(page)).toEqual([
    { action: 'task.state', body: { taskId: 'T-2', state: 'in-progress' } },
  ]);
});

test('blocking asks for a reason and sends it with the state', async ({ page }) => {
  const band = await openTask(page, 'T-2');
  await choose(page, /^State/, 'Blocked');
  await expect(band.getByRole('button', { name: 'Block', exact: true })).toBeDisabled();
  await band.getByLabel('Reason').fill('Waiting for the legal copy');
  await band.getByRole('button', { name: 'Block', exact: true }).click();
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
  await expect(band.locator('.work-detail-state')).toContainText('sign-off');
  await band.getByRole('button', { name: 'Sign off' }).click();
  await expect(band.locator('.work-detail-state')).not.toContainText('sign-off');
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

const MADE_PAGE = '20000000-0000-4000-8000-000000000009';
async function openOwnerTask(
  page: Page,
  id: string,
  scenario?: 'linked-done' | 'linked-missing' | 'external',
) {
  await mountRoute(page, true, true, scenario);
  if (scenario === 'linked-done') {
    await page.evaluate((id) => {
      location.hash = `/pages/notes?task=${id}`;
    }, id);
    await expect(page.locator('#work-page-fixture .work-detail')).toBeVisible();
    return page.getByRole('region', { name: 'Work on this page' });
  }
  const band = page.getByRole('region', { name: 'Work on this page' });
  await openTasks(page);
  await band.locator('.work-row', { hasText: id }).locator('button').click();
  return band;
}

test('Convert to Colab sends one action and the task then follows the page it made', async ({
  page,
}) => {
  const band = await openOwnerTask(page, 'T-2');
  await band.getByRole('button', { name: 'Convert to Colab' }).click();
  await expect(band.locator('.work-follows a')).toHaveAttribute('href', new RegExp(MADE_PAGE));
  await expect(band.getByRole('button', { name: 'Convert to Colab' })).toBeDisabled();
  expect(await calls(page)).toEqual([{ action: 'task.convert', body: { taskId: 'T-2' } }]);
});

test('a task that follows a page or has an external ID cannot be converted, and says why', async ({
  page,
}) => {
  let band = await openOwnerTask(page, 'T-2', 'linked-done');
  await expect(band.getByRole('button', { name: 'Convert to Colab' })).toBeDisabled();
  await expect(band.getByText('This task already follows a page.')).toBeVisible();
  band = await openOwnerTask(page, 'T-2', 'external');
  await expect(band.getByRole('button', { name: 'Convert to Colab' })).toBeDisabled();
  await expect(band.getByText('A task with an external ID cannot follow a page.')).toBeVisible();
});

test('a page made but not linked is named, and can be linked or opened', async ({ page }) => {
  const band = await openOwnerTask(page, 'T-2');
  await page.evaluate((childPage) => {
    window.__workPage!.refuse = {
      code: 'COLAB_LINK_REFUSED',
      message: 'That page already follows another task.',
      childPage,
    };
  }, MADE_PAGE);
  await band.getByRole('button', { name: 'Convert to Colab' }).click();
  // The refusal sits under the stranded row it belongs to, not in the New task field.
  const stranded = band.locator('.work-stranded');
  await expect(stranded.getByRole('alert')).toHaveText('That page already follows another task.');
  await expect(band.locator('.work-add').getByRole('alert')).toHaveCount(0);
  await expect(band.getByText('A page was made for this task and is not linked.')).toBeVisible();
  await expect(band.getByRole('link', { name: 'Open the page' })).toHaveAttribute(
    'href',
    new RegExp(MADE_PAGE),
  );
  // A refused link answers the same row.
  await page.evaluate(() => {
    window.__workPage!.refuse = { code: 'COLAB_LINK_REFUSED', message: 'Still refused.' };
  });
  await band.getByRole('button', { name: 'Link to that page' }).click();
  await expect(stranded.getByRole('alert')).toHaveText('Still refused.');
  await expect(band.locator('.work-add').getByRole('alert')).toHaveCount(0);
  await band.getByRole('button', { name: 'Link to that page' }).click();
  await expect(band.locator('.work-follows a')).toHaveAttribute('href', new RegExp(MADE_PAGE));
  expect((await calls(page)).map((entry) => entry.action)).toEqual([
    'task.convert',
    'task.link',
    'task.link',
  ]);
  expect((await calls(page))[2].body).toEqual({ taskId: 'T-2', pageId: MADE_PAGE });
});

test('state revision is captured when its menu opens, before a choice is applied', async ({
  page,
}) => {
  await mountRoute(page, true, true);
  await page.getByRole('button', { name: /Page state/ }).click();
  expect(await page.evaluate(() => window.__workPage!.metadataCaptures)).toBe(1);
  await page.getByRole('menuitemradio', { name: 'Done', exact: true }).click();
  await expect(page.getByRole('button', { name: /Page state/ })).toContainText('Done');
  expect(await page.evaluate(() => window.__workPage!.metadataCaptures)).toBe(1);
});

test('a page with no sub-pages keeps tasks behind the toggle', async ({ page }) => {
  await mountRoute(page, true, true);
  const band = page.getByRole('region', { name: 'Work on this page' });
  await band.getByRole('button', { name: 'Expand status panel', exact: true }).click();
  await expect(band.locator('.work-subpages')).toHaveCount(0);
  await expect(band.locator('.work-row')).toHaveCount(0);
  await band.getByRole('button', { name: /^Tasks \d+$/ }).click();
  await expect(band.locator('.work-row')).toHaveCount(3);
  await band.locator('.work-row').filter({ hasText: 'Draft' }).locator('button').first().click();
  await expect(band.locator('.work-controls')).toBeVisible();
});

test('attach is available to owner pages and hidden for observed children and readers', async ({
  page,
}) => {
  await mountRoute(page, true, true);
  await expand(page);
  await expect(page.getByRole('button', { name: 'Attach a sub-page' })).toBeVisible();

  await mountRoute(page, true, true, 'child-orphan');
  await expand(page);
  await expect(page.getByRole('button', { name: 'Attach a sub-page' })).toHaveCount(0);

  await mountRoute(page, true, false);
  await expand(page);
  await expect(page.getByRole('button', { name: 'Attach a sub-page' })).toHaveCount(0);
});

test('attach keeps a created task after refusal and retries that task', async ({ page }) => {
  await mountRoute(page, true, true, 'attach');
  await expand(page);
  await page.getByRole('button', { name: 'Attach a sub-page' }).click();
  const dialog = page.locator('dialog.tmt-ui-dialog[data-state="open"]');
  await dialog
    .getByRole('combobox', { name: 'Page', exact: true })
    .selectOption({ label: 'Legal copy' });
  await dialog.getByLabel('Task title').fill('Follow legal copy');
  await page.evaluate(() => {
    window.__workPage!.refuseLink = true;
  });
  await dialog.getByRole('button', { name: 'Attach', exact: true }).click();
  await expect(dialog.getByRole('alert')).toHaveText('The page already has a parent.');
  await expect(dialog).toContainText('Task T-4 is kept');
  await dialog.getByRole('button', { name: 'Retry link' }).click();
  await expect(dialog).toHaveCount(0);
  expect(await calls(page)).toEqual([
    { action: 'task.add', body: { task: { title: 'Follow legal copy' } } },
    {
      action: 'task.link',
      body: { taskId: 'T-4', pageId: '20000000-0000-4000-8000-000000000002' },
    },
    {
      action: 'task.link',
      body: { taskId: 'T-4', pageId: '20000000-0000-4000-8000-000000000002' },
    },
  ]);
});

test('an uncertain task creation cannot be sent twice', async ({ page }) => {
  await mountRoute(page, true, true, 'attach');
  await expand(page);
  await page.getByRole('button', { name: 'Attach a sub-page' }).click();
  const dialog = page.locator('dialog.tmt-ui-dialog[data-state="open"]');
  await dialog
    .getByRole('combobox', { name: 'Page', exact: true })
    .selectOption({ label: 'Legal copy' });
  await dialog.getByLabel('Task title').fill('Follow legal copy');
  await page.evaluate(() => {
    window.__workPage!.unknownAdd = true;
  });
  await dialog.getByRole('button', { name: 'Attach', exact: true }).click();
  await expect(dialog.getByRole('alert')).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Attach', exact: true })).toBeDisabled();
  expect((await calls(page)).map((entry) => entry.action)).toEqual(['task.add']);
});

for (const width of [390, 1440])
  for (const theme of ['light', 'dark'] as const)
    test(`attach picker capture ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await mountRoute(page, true, true, 'attach');
      await expand(page);
      if (process.env.COLAB_CAPTURE_DIR)
        await page.screenshot({ path: capturePath(`attach-panel-${width}-${theme}.png`) });
      await page.getByRole('button', { name: 'Attach a sub-page' }).click();
      const dialog = page.locator('dialog.tmt-ui-dialog[data-state="open"]');
      await dialog
        .getByRole('combobox', { name: 'Page', exact: true })
        .selectOption({ label: 'Legal copy' });
      await dialog.getByLabel('Task title').fill('Follow legal copy');
      if (process.env.COLAB_CAPTURE_DIR)
        await page.screenshot({ path: capturePath(`attach-picker-${width}-${theme}.png`) });
      await page.evaluate(() => {
        window.__workPage!.refuseLink = true;
      });
      await dialog.getByRole('button', { name: 'Attach', exact: true }).click();
      await expect(dialog.getByRole('alert')).toBeVisible();
      if (process.env.COLAB_CAPTURE_DIR)
        await page.screenshot({ path: capturePath(`attach-refused-${width}-${theme}.png`) });
    });

test('attach can link an existing unlinked task without creating another', async ({ page }) => {
  await mountRoute(page, true, true, 'attach');
  await expand(page);
  await page.getByRole('button', { name: 'Attach a sub-page' }).click();
  const dialog = page.locator('dialog.tmt-ui-dialog[data-state="open"]');
  await dialog
    .getByRole('combobox', { name: 'Page', exact: true })
    .selectOption({ label: 'Legal copy' });
  await dialog.getByRole('combobox', { name: 'Task', exact: true }).selectOption('T-2');
  await expect(dialog.getByLabel('Task title')).toHaveCount(0);
  await dialog.getByRole('button', { name: 'Attach', exact: true }).click();
  await expect(dialog).toHaveCount(0);
  expect(await calls(page)).toEqual([
    {
      action: 'task.link',
      body: { taskId: 'T-2', pageId: '20000000-0000-4000-8000-000000000002' },
    },
  ]);
});

test('leaving the page while adding a task does not start a link on the old page', async ({
  page,
}) => {
  await mountRoute(page, true, true, 'attach');
  await expand(page);
  await page.getByRole('button', { name: 'Attach a sub-page' }).click();
  const dialog = page.locator('dialog.tmt-ui-dialog[data-state="open"]');
  await dialog
    .getByRole('combobox', { name: 'Page', exact: true })
    .selectOption({ label: 'Legal copy' });
  await dialog.getByLabel('Task title').fill('Follow legal copy');
  await page.evaluate(() => {
    window.__workPage!.holdAdd = true;
  });
  await dialog.getByRole('button', { name: 'Attach', exact: true }).click();
  await expect.poll(() => page.evaluate(() => !!window.__workPage?.finishAdd)).toBe(true);
  const oldCalls = await page.evaluate(async () => {
    const old = window.__workPage!;
    const path = '/test/work-page-browser.tsx';
    (await import(path)).mountWorkPage(true, true);
    const added = new Promise<void>((resolve) => {
      old.didAdd = resolve;
    });
    old.finishAdd!();
    await added;
    await Promise.resolve();
    await Promise.resolve();
    return old.calls;
  });
  expect(oldCalls.map((entry) => entry.action)).toEqual(['task.add']);
});

test('after a refusal another page can use the same retained task', async ({ page }) => {
  await mountRoute(page, true, true, 'attach');
  await expand(page);
  await page.getByRole('button', { name: 'Attach a sub-page' }).click();
  const dialog = page.locator('dialog.tmt-ui-dialog[data-state="open"]');
  const target = dialog.getByRole('combobox', { name: 'Page', exact: true });
  await target.selectOption({ label: 'Legal copy' });
  await dialog.getByLabel('Task title').fill('Follow a page');
  await page.evaluate(() => {
    window.__workPage!.refuseLink = true;
  });
  await dialog.getByRole('button', { name: 'Attach', exact: true }).click();
  await expect(dialog.getByRole('alert')).toBeVisible();
  await target.selectOption({ label: 'Release plan' });
  await dialog.getByRole('button', { name: 'Retry link' }).click();
  await expect(dialog).toHaveCount(0);
  const sent = await calls(page);
  expect(sent.filter((entry) => entry.action === 'task.add')).toHaveLength(1);
  expect(sent.at(-1)).toEqual({
    action: 'task.link',
    body: { taskId: 'T-4', pageId: '30000000-0000-4000-8000-000000000003' },
  });
});

test('reopening after uncertain task creation restores the form', async ({ page }) => {
  await mountRoute(page, true, true, 'attach');
  await expand(page);
  const open = page.getByRole('button', { name: 'Attach a sub-page' });
  await open.click();
  const dialog = page.locator('dialog.tmt-ui-dialog[data-state="open"]');
  await dialog
    .getByRole('combobox', { name: 'Page', exact: true })
    .selectOption({ label: 'Legal copy' });
  await dialog.getByLabel('Task title').fill('Follow legal copy');
  await page.evaluate(() => {
    window.__workPage!.unknownAdd = true;
  });
  await dialog.getByRole('button', { name: 'Attach', exact: true }).click();
  await expect(dialog.getByRole('alert')).toHaveText(
    'The result is unknown. Check the task list before trying again.',
  );
  await expect(dialog.getByRole('button', { name: 'Attach', exact: true })).toBeDisabled();
  await dialog.getByRole('button', { name: 'Cancel' }).click();
  await open.click();
  await expect(dialog.getByRole('button', { name: 'Attach', exact: true })).toBeEnabled();
});

test('uncertain linking resolves success from admitted work', async ({ page }) => {
  await mountRoute(page, true, true, 'attach');
  await expand(page);
  await page.getByRole('button', { name: 'Attach a sub-page' }).click();
  const dialog = page.locator('dialog.tmt-ui-dialog[data-state="open"]');
  await dialog
    .getByRole('combobox', { name: 'Page', exact: true })
    .selectOption({ label: 'Legal copy' });
  await dialog.getByRole('combobox', { name: 'Task', exact: true }).selectOption('T-2');
  await page.evaluate(() => {
    window.__workPage!.unknownLink = true;
  });
  await dialog.getByRole('button', { name: 'Attach', exact: true }).click();
  await expect(dialog.getByRole('button', { name: 'Retry link' })).toBeDisabled();
  await page.evaluate(() => window.__workPage!.settle!('linked'));
  await expect(dialog).toHaveCount(0);
});

test('reopening after an uncertain link keeps the task and enables retry', async ({ page }) => {
  await mountRoute(page, true, true, 'attach');
  await expand(page);
  const open = page.getByRole('button', { name: 'Attach a sub-page' });
  await open.click();
  const dialog = page.locator('dialog.tmt-ui-dialog[data-state="open"]');
  await dialog
    .getByRole('combobox', { name: 'Page', exact: true })
    .selectOption({ label: 'Legal copy' });
  await dialog.getByRole('combobox', { name: 'Task', exact: true }).selectOption('T-2');
  await page.evaluate(() => {
    window.__workPage!.unknownLink = true;
  });
  await dialog.getByRole('button', { name: 'Attach', exact: true }).click();
  await dialog.getByRole('button', { name: 'Cancel' }).click();
  await open.click();
  await expect(dialog).toContainText('Task T-2 is kept');
  await expect(dialog.getByRole('button', { name: 'Retry link' })).toBeEnabled();
  await page.evaluate(() => window.__workPage!.settle!('linked'));
  await expect(dialog).toHaveCount(0);
});

test('the attach picker does not offer shell pages', async ({ page }) => {
  await mountRoute(page, true, true, 'attach');
  await expand(page);
  await page.getByRole('button', { name: 'Attach a sub-page' }).click();
  const dialog = page.locator('dialog.tmt-ui-dialog[data-state="open"]');
  const target = dialog.getByRole('combobox', { name: 'Page', exact: true });
  await expect(target.locator('option', { hasText: 'Dashboard' })).toHaveCount(0);
  await expect(target.locator('option', { hasText: 'Already attached' })).toHaveCount(0);
  await expect(target.locator('option', { hasText: 'Has child' })).toHaveCount(0);
});

test('status band exposes the spec meter and keeps tasks behind their chip', async ({ page }) => {
  const host = await mountBand(page, 'panel');
  await expect(host.locator('.work-head .tmt-ui-meter')).toBeVisible();
  await expect(host.locator('.work-body')).toHaveCSS('overflow-y', 'hidden');
  const done = host.locator('.work-segments [data-state=done]');
  const working = host.locator('.work-segments [data-state=in-progress]');
  expect((await done.boundingBox())!.width).toBeGreaterThan(0);
  expect((await working.boundingBox())!.height).toBeGreaterThan(0);
  expect(await working.evaluate((node) => getComputedStyle(node).backgroundColor)).not.toBe(
    'rgba(0, 0, 0, 0)',
  );
  await expect(host.getByRole('button', { name: 'Expand status panel' })).toHaveClass(
    /tmt-ui-action/,
  );
  await expand(page);
  await expect(host.locator('.work-body')).toHaveCSS('overflow-y', 'auto');
  await expect(host.getByRole('button', { name: /Tasks/ })).toHaveClass(/tmt-ui-count-chip/);
  await expect(host.locator('.work-tasks')).toHaveCount(0);
});

test('the expanded head stays fixed while its long body scrolls', async ({ page }) => {
  const host = await mountBand(page, 'long');
  await expand(page);
  const head = host.locator('.work-head');
  const body = host.locator('.work-body');
  const before = await head.boundingBox();
  expect(await body.evaluate((node) => node.scrollHeight > node.clientHeight)).toBe(true);
  await body.evaluate((node) => {
    node.scrollTop = node.scrollHeight;
  });
  expect(await body.evaluate((node) => node.scrollTop)).toBeGreaterThan(0);
  expect((await head.boundingBox())!.y).toBe(before!.y);
});

test('expansion animates unless reduced motion is requested', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  const host = await mountBand(page, 'panel');
  const panel = host.locator('.work-band');
  const collapsed = (await panel.boundingBox())!.height;
  await expand(page);
  const frames = await panel.evaluate(async (element) => {
    const heights: number[] = [];
    await new Promise<void>((resolve) => {
      const start = performance.now();
      const frame = () => {
        heights.push(element.getBoundingClientRect().height);
        if (performance.now() - start < 500) requestAnimationFrame(frame);
        else resolve();
      };
      requestAnimationFrame(frame);
    });
    return heights;
  });
  const expanded = frames.at(-1)!;
  expect(frames.some((height) => height > collapsed && height < expanded)).toBe(true);
  await page.emulateMedia({ reducedMotion: 'reduce' });
  expect(
    await host
      .locator('.work-body-transition')
      .evaluate((node) => getComputedStyle(node).transitionDuration),
  ).toMatch(/^0s(?:, 0s)*$/);
  await page.getByRole('button', { name: 'Collapse status panel' }).click();
  await expect.poll(async () => (await panel.boundingBox())!.height).toBe(collapsed);
});
