import { expect, test, type Page } from '@playwright/test';
import { pageAction } from '../test/page-actions.js';
const fixture = '/test/metadata-browser.html';
async function command(page: Page, method: string, value?: unknown) {
  return page.evaluate(
    async ({ method, value }) => {
      const path = '/test/metadata-browser.tsx';
      return (await import(path))[method](value);
    },
    { method, value },
  );
}
async function open(page: Page, name: string) {
  await (await pageAction(page, name)).click();
  return page.getByRole('dialog', {
    name: name === 'Questions…' ? 'Questions' : 'Page creator',
    exact: true,
  });
}
async function ready(page: Page, suffix = '') {
  await page.goto(fixture + suffix);
  await expect(
    page.frameLocator('iframe').getByRole('heading', { name: 'Metadata page' }),
  ).toBeVisible();
}
for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`metadata menu, registry and About at ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await ready(page);
      await expect(
        page.locator('.tmt-ui-header').getByRole('button', { name: /Creator|Set creator/ }),
      ).toHaveCount(0);
      const dialog = await open(page, 'Questions…');
      await expect(dialog.locator('.metadata-question-state')).toHaveText([
        'Open',
        'Answered',
        'Handled',
        'Open',
      ]);
      await expect(dialog.getByText('Removed from page source')).toHaveCount(2);
      const first = dialog
        .locator('li')
        .filter({ has: page.getByText('First question', { exact: true }) });
      await first.getByRole('button', { name: 'Mark handled', exact: true }).click();
      await expect(first.locator('.metadata-question-state')).toHaveText('Handled');
      await first.getByRole('button', { name: 'Reopen', exact: true }).click();
      await expect(first.locator('.metadata-question-state')).toHaveText('Open');
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
      await dialog.getByRole('button', { name: 'Close', exact: true }).click();
      await expect(
        page.locator('.page-header-actions').getByRole('button', { name: 'More', exact: true }),
      ).toBeFocused();
      const creator = await open(page, 'Creator: First creator…');
      await expect(
        creator.getByRole('button', { name: 'Set creator', exact: true }),
      ).toBeDisabled();
      await creator.getByRole('option', { name: 'Next creator', exact: true }).click();
      await creator.getByRole('button', { name: 'Set creator', exact: true }).click();
      await expect(creator).not.toBeVisible();
      await expect(
        page.locator('.page-header-actions').getByRole('button', { name: 'More', exact: true }),
      ).toBeFocused();
      await (await pageAction(page, 'About this page')).click();
      const about = page.locator('.tmt-ui-drawer[data-panel=about]');
      await expect(about.getByText('Next creator', { exact: true })).toBeVisible();
      await expect(about.getByText('Original author', { exact: true })).toHaveCount(2);
      const proof = await command(page, 'proof');
      expect(proof.commands.map((item: { action: string }) => item.action)).toEqual([
        'question.handle',
        'question.reopen',
        'creator.set',
      ]);
    });
  }
test('unavailable directory preserves creator, explicit Retry recovers, stale choice stays visible', async ({
  page,
}) => {
  await ready(page);
  await command(page, 'setMode', 'unavailable');
  const dialog = await open(page, 'Creator: First creator…');
  await expect(dialog.getByText(/Agent directory unavailable/)).toBeVisible();
  await expect(dialog.getByText('Current creator: First creator')).toBeVisible();
  await command(page, 'setMode', 'ready');
  await dialog.getByRole('button', { name: 'Retry', exact: true }).click();
  await dialog.getByRole('option', { name: 'Next creator', exact: true }).click();
  await command(page, 'setMode', 'stale');
  await dialog.getByRole('button', { name: 'Set creator', exact: true }).click();
  await expect(dialog.getByRole('alert')).toContainText('Page changed');
  await expect(dialog.getByText('Selected: Next creator', { exact: true })).toBeVisible();
  expect((await command(page, 'proof')).view.creator.agent.name).toBe('First creator');
});
test('link editor sees states read-only without owner-store CLI guidance', async ({ page }) => {
  await ready(page, '?role=editor');
  const dialog = await open(page, 'Questions…');
  await expect(
    dialog.getByText('Only the page owner can change this here', { exact: true }),
  ).toBeVisible();
  await expect(dialog.getByRole('button', { name: /Mark handled|Reopen/ })).toHaveCount(0);
  await expect(dialog.locator('code')).toHaveCount(0);
  expect((await command(page, 'proof')).commands).toEqual([]);
});
test('unknown discussion is shown as verification, never open questions', async ({ page }) => {
  await ready(page);
  await command(page, 'checking');
  const dialog = await open(page, 'Questions…');
  await expect(dialog.getByText('Verifying question answers…', { exact: true })).toBeVisible();
  await expect(dialog.locator('.metadata-question-state')).toHaveCount(0);
});
test('pending metadata fences reactivation and dismissal, with one overlay', async ({ page }) => {
  await ready(page);
  const dialog = await open(page, 'Creator: First creator…');
  await dialog.getByRole('option', { name: 'Next creator', exact: true }).click();
  await command(page, 'setMode', 'pending');
  const save = dialog.getByRole('button', { name: 'Set creator', exact: true });
  await save.click();
  await expect(save).toHaveAttribute('aria-busy', 'true');
  await save.press('Enter');
  await page.keyboard.press('Escape');
  await expect(dialog).toBeVisible();
  expect((await command(page, 'proof')).commands).toHaveLength(1);
  await expect(page.locator('dialog[open]')).toHaveCount(1);
  await command(page, 'settle');
  await expect(dialog).not.toBeVisible();
});

test('a removed registered question remains pending and opens the registry rather than a fake anchor', async ({
  page,
}) => {
  await ready(page);
  const pending = page.getByRole('button', { name: 'Needs you: 2', exact: true });
  await pending.click();
  await page
    .getByRole('menuitem', { name: 'Removed question · First creator', exact: true })
    .click();
  const dialog = page.getByRole('dialog', { name: 'Questions', exact: true });
  const removed = dialog
    .getByRole('listitem')
    .filter({ has: page.getByText('Removed question', { exact: true }) });
  await expect(removed.getByText('Removed from page source', { exact: true })).toBeVisible();
  await removed.getByRole('button', { name: 'Mark handled', exact: true }).click();
  await expect(removed.locator('.metadata-question-state')).toHaveText('Handled');
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Needs you: 1', exact: true })).toBeVisible();
});

test('a metadata edit preserves a dirty Source draft and original attribution', async ({
  page,
}) => {
  await ready(page);
  await (await pageAction(page, 'Source')).click();
  const source = page.getByRole('textbox', { name: 'Page source', exact: true });
  const base = await source.inputValue();
  const draft = base + '\n<p>A retained unsaved draft</p>';
  await source.fill(draft);
  await page.getByRole('button', { name: 'Close Source', exact: true }).click();
  const dialog = await open(page, 'Creator: First creator…');
  await dialog.getByRole('option', { name: 'Next creator', exact: true }).click();
  await dialog.getByRole('button', { name: 'Set creator', exact: true }).click();
  await expect(dialog).not.toBeVisible();
  await (await pageAction(page, 'Source')).click();
  await expect(source).toHaveValue(draft);
  const proof = await command(page, 'proof');
  expect(proof.view.source).toBe(base);
  expect(proof.view.originalAuthor).toBe('Original author');
  expect(proof.commands).toHaveLength(1);
});

test('creator changes seed new question drafts without remounting dirty drafts or rewriting sent recipients', async ({
  page,
}) => {
  const run = (method: string, value?: unknown) =>
    page.evaluate(
      async ({ method, value }) => {
        const path = '/test/ask-page-browser.tsx';
        return (await import(path))[method](value);
      },
      { method, value },
    );
  await page.goto('/');
  await run('mount', { creator: true });
  await run('questionGroup', { split: true, agents: ['', ''] });
  const rows = page.locator('.question-line');
  await rows.first().click();
  await page.getByLabel('Keep it', { exact: true }).check();
  await expect(
    page.getByRole('button', { name: 'Send answer · to @Agent 1', exact: true }),
  ).toBeVisible();
  await run('changePageCreator', 1);
  await expect(page.getByLabel('Keep it', { exact: true })).toBeChecked();
  await expect(
    page.getByRole('button', { name: 'Send answer · to @Agent 1', exact: true }),
  ).toBeVisible();
  await rows.nth(1).click();
  await page.getByLabel('Your answer', { exact: true }).fill('Keep this second draft');
  await expect(
    page.getByRole('button', { name: 'Send answer · to @Agent 2', exact: true }),
  ).toBeVisible();
  await rows.first().click();
  await page.getByRole('button', { name: 'Send answer · to @Agent 1', exact: true }).click();
  await expect(rows.first().locator('.question-line-state')).toHaveText('Answered');
  await expect.poll(async () => (await run('proof')).sends.length).toBe(1);
  const sent = (await run('proof')).sends;
  expect(sent[0].agentId).toBe('00000000-0000-4000-8000-000000000006');
  await run('changePageCreator', 2);
  await rows.nth(1).click();
  await expect(page.getByLabel('Your answer', { exact: true })).toHaveValue(
    'Keep this second draft',
  );
  await expect(
    page.getByRole('button', { name: 'Send answer · to @Agent 2', exact: true }),
  ).toBeVisible();
  expect((await run('proof')).sends).toEqual(sent);
});

import answerVectors from '../../../contracts/vectors/question-answers-v1.json' with { type: 'json' };
for (const vector of answerVectors.cases)
  test(`native/browser registry parity: ${vector.name}`, async ({ page }) => {
    await ready(page);
    await command(page, 'registryVector', vector);
    const dialog = await open(page, 'Questions…');
    await expect(dialog.locator('.metadata-question-state')).toHaveText(
      vector.expected.map(
        (question) => question.state.slice(0, 1).toUpperCase() + question.state.slice(1),
      ),
    );
    await dialog.getByRole('button', { name: 'Close', exact: true }).click();
    if (vector.pending.length)
      await expect(
        page.getByRole('button', { name: `Needs you: ${vector.pending.length}`, exact: true }),
      ).toBeVisible();
    else await expect(page.locator('.needs-you-header-trigger')).toHaveCount(0);
  });

test('a registered question and a legacy passage with the same ID both remain pending', async ({
  page,
}) => {
  await ready(page);
  await command(page, 'readingCollision');
  await page.getByRole('button', { name: 'Needs you: 3', exact: true }).click();
  await expect(page.getByRole('menu').locator('.needs-you-header-title')).toHaveText([
    'Same ID reading',
    'First question',
    'Removed question',
  ]);
});

test('short phone viewport keeps long registry content reachable and restores dialog focus', async ({
  page,
}) => {
  await page.setViewportSize({ width: 320, height: 568 });
  await ready(page);
  await command(page, 'longLabels');
  const dialog = await open(page, 'Questions…');
  await expect(dialog.locator('.metadata-question-title strong')).toHaveCount(4);
  await expect(
    dialog.getByRole('button', { name: 'Mark handled', exact: true }).last(),
  ).toBeAttached();
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  const more = page
    .locator('.page-header-actions')
    .getByRole('button', { name: 'More', exact: true });
  await expect(more).toBeFocused();
  const creator = await open(page, 'Creator: First creator…');
  const search = creator.getByRole('combobox');
  await search.fill('Next');
  await expect(creator.getByRole('option')).toHaveText(['Next creator']);
  const box = await search.boundingBox();
  expect(box!.width).toBeGreaterThan(200);
  expect(box!.x + box!.width).toBeLessThanOrEqual(320);
  await creator.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(more).toBeFocused();
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(320);
});

test('navigation disposes a pending editor and its late result cannot reopen an overlay', async ({
  page,
}) => {
  await ready(page);
  const dialog = await open(page, 'Creator: First creator…');
  await dialog.getByRole('option', { name: 'Next creator', exact: true }).click();
  await command(page, 'setMode', 'pending');
  await dialog.getByRole('button', { name: 'Set creator', exact: true }).click();
  await expect(dialog.getByRole('button', { name: 'Set creator', exact: true })).toHaveAttribute(
    'aria-busy',
    'true',
  );
  await page.evaluate(() => {
    location.hash = '/';
  });
  await expect(dialog).not.toBeVisible();
  await command(page, 'settle');
  await expect(page.locator('dialog[open]')).toHaveCount(0);
  expect((await command(page, 'proof')).commands).toHaveLength(1);
});
