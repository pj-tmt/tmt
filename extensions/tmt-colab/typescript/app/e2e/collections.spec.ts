import { expect, test, type Page } from '@playwright/test';
const fixture = '/test/collections-browser.tsx';
const run = (page: Page, method: string, argument?: unknown) =>
  page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
async function mount(page: Page, kind = 'manual') {
  await page.goto('/');
  await run(page, 'mount', kind);
  await expect(page.locator('#ask-page-fixture .collections')).toBeVisible();
}
test('owner collections keep repeated and unavailable references, and Seen never writes a page or notifies', async ({
  page,
}) => {
  await mount(page);
  const rows = page.locator('#ask-page-fixture [data-collection-item]');
  await expect(rows).toHaveCount(3);
  await expect(rows.nth(0).locator('.chapter-passage iframe')).toBeVisible();
  await expect(rows.nth(0).getByRole('button', { name: 'Mark read', exact: true })).toHaveCount(0);
  await expect(rows.nth(0).locator('.chapter-passage-kicker')).toHaveCount(0);
  await rows.nth(1).scrollIntoViewIfNeeded();
  await expect(rows.nth(1)).toContainText('Item unavailable');
  await expect(rows.nth(2)).toContainText('Page unavailable');
  await expect(rows.nth(2).getByRole('link')).toHaveCount(0);
  await rows.nth(0).getByRole('button', { name: 'Mark seen', exact: true }).click();
  await expect(rows.nth(0)).toHaveAttribute('data-seen', 'true');
  await expect(rows.nth(0).locator('.chapter-passage iframe')).toBeVisible();
  await expect(rows.nth(0).getByRole('button', { name: 'Mark seen', exact: true })).toHaveCount(0);
  const proof = await run(page, 'proof');
  expect(proof.changes).toEqual([
    {
      op: 'seen',
      id: '00000000-0000-4000-8000-000000000700',
      target: '00000000-0000-4000-8000-000000000710',
    },
  ]);
  expect(proof.records).toEqual([]);
  expect(proof.actions).toEqual([]);
  expect(proof.chapter.acquisitions - proof.chapter.releases).toBeLessThanOrEqual(3);
  await page.getByRole('button', { name: 'Collection menu', exact: true }).click();
  await expect(
    page.getByRole('menuitemradio', { name: 'Sort: Manual order', exact: true }),
  ).toHaveAttribute('aria-checked', 'true');
  const selectedSort = page.getByRole('menuitemradio', { name: 'Sort: Manual order', exact: true });
  const [mark, label, other] = await Promise.all([
    selectedSort.locator('> span[aria-hidden="true"]').boundingBox(),
    selectedSort.locator('> span').first().boundingBox(),
    page
      .getByRole('menuitemradio', { name: 'Sort: Title', exact: true })
      .locator('> span')
      .first()
      .boundingBox(),
  ]);
  expect(mark!.x).toBeLessThan(label!.x);
  expect(other!.x).toBe(label!.x);
  await page.getByRole('menuitem', { name: 'Clear seen', exact: true }).click();
  await expect(rows.nth(0)).not.toHaveAttribute('data-seen');
  await expect(page.getByRole('status')).toContainText('Cleared seen marks in Release reading');
  expect((await run(page, 'proof')).records).toEqual([]);
});
test('synthetic clicks and merely opening a collection cannot mark its items Seen', async ({
  page,
}) => {
  await mount(page);
  await page
    .locator('[data-collection-item]')
    .first()
    .getByRole('button', { name: 'Mark seen', exact: true })
    .evaluate((button: HTMLButtonElement) => button.click());
  expect((await run(page, 'proof')).changes).toEqual([]);
  await expect(page.locator('[data-collection-item][data-seen]')).toHaveCount(0);
});
test('smart results distinguish pending, partial and a complete empty result', async ({ page }) => {
  await mount(page, 'smart-pending');
  await expect(page.locator('#ask-page-fixture .collections')).toContainText('Checking pages…');
  await expect(page.locator('#ask-page-fixture .collections')).not.toContainText(
    'No pages match this collection.',
  );
  await mount(page, 'smart-partial');
  await expect(page.locator('[data-collection-item]')).toHaveCount(1);
  await expect(page.locator('#ask-page-fixture .collections')).toContainText(
    'Checking more pages…',
  );
  await mount(page, 'smart-empty');
  await expect(page.locator('#ask-page-fixture .collections')).toContainText(
    'No pages match this collection.',
  );
  await expect(page.locator('#ask-page-fixture .collections')).not.toContainText('Checking');
});
test('directory navigation is last and empty collections keep the approved wording', async ({
  page,
}) => {
  await mount(page, 'directory');
  await expect(page.locator('#ask-page-fixture .home-nav').getByRole('link').last()).toHaveText(
    'Collections',
  );
  await expect(page.locator('.collection-directory > li')).toHaveCount(2);
  await mount(page, 'empty-directory');
  await expect(page.locator('#ask-page-fixture .collections')).toContainText('No collections yet.');
  await expect(page.locator('#ask-page-fixture .collections')).toContainText(
    'Agents create collections from the CLI.',
  );
  await mount(page, 'empty-manual');
  await expect(page.locator('#ask-page-fixture .collections')).toContainText(
    'No items in this collection yet.',
  );
});
test('an unconfirmed metadata write stays visible and never clears the edit or sends it again', async ({
  page,
}) => {
  await mount(page);
  await run(page, 'refuse', 'unknown');
  await page.getByRole('button', { name: 'Collection menu', exact: true }).click();
  await page.getByRole('menuitem', { name: 'Rename…', exact: true }).click();
  await page.getByRole('textbox', { name: 'Collection name', exact: true }).fill('New reports');
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('unconfirmed');
  await expect(page.getByRole('textbox', { name: 'Collection name', exact: true })).toHaveValue(
    'New reports',
  );
  expect((await run(page, 'proof')).changes).toHaveLength(1);
});
test('a collection page opens at its cosmetic section and returns to the same entry', async ({
  page,
}) => {
  await mount(page);
  const first = page.locator('[data-collection-item]').first();
  const link = first.locator('h2 a');
  await expect(link).toHaveAttribute('href', /section=part%3A2/);
  await link.click();
  await expect(page.getByRole('button', { name: '← Collection', exact: true })).toBeVisible();
  await expect(page.locator('#ask-page-fixture iframe')).toBeVisible();
  await expect(
    page.frameLocator('#ask-page-fixture iframe').getByRole('heading', { name: 'Release figures' }),
  ).toBeVisible();
  await page.getByRole('button', { name: '← Collection', exact: true }).click();
  await expect(page.locator('#ask-page-fixture .collections h1')).toHaveText('Release reading');
  await expect(page.locator('[data-collection-item]').first().locator('h2 a')).toBeFocused();
  expect((await run(page, 'proof')).changes).toEqual([]);
});
for (const kind of ['reading', 'review'] as const)
  test(`collection ${kind} uses the ordinary record and one notice, independent of Seen`, async ({
    page,
  }) => {
    await mount(page, kind);
    const passage = page.locator('[data-collection-item]').first().locator('.chapter-passage');
    const action = passage
      .getByRole('button', {
        name: kind === 'reading' ? 'Mark read' : /Confirm and close/,
        exact: kind === 'reading',
      })
      .first();
    await expect(action).toBeVisible();
    await action.click();
    if (kind === 'reading')
      await expect(passage.locator('[data-inline-reading]').first()).toHaveAttribute(
        'data-reading-state',
        'read',
      );
    else await expect(passage.locator('[data-inline-review]').first()).toContainText('Closed');
    await expect
      .poll(async () => (await run(page, 'proof')).actions)
      .toEqual(['status:resolve', 'notify:Agent 1']);
    const proof = await run(page, 'proof');
    expect(proof.records).toHaveLength(1);
    expect(proof.actions).toEqual(['status:resolve', 'notify:Agent 1']);
    expect(proof.changes).toEqual([]);
    await expect(
      page
        .locator('[data-collection-item]')
        .first()
        .getByRole('button', { name: 'Mark seen', exact: true }),
    ).toBeVisible();
  });

test('a collection answer records and sends once through the existing answer owner', async ({
  page,
}) => {
  await mount(page, 'question');
  const row = page.locator('#ask-page-fixture [data-collection-item]').first();
  const passage = row.locator('.chapter-passage');
  await passage
    .getByRole('textbox', { name: 'Your answer', exact: true })
    .fill('Use the clearer heading.');
  await passage.getByRole('button', { name: /^Send answer/ }).click();
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
  const proof = await run(page, 'proof');
  expect(proof.records).toHaveLength(1);
  expect(proof.changes).toEqual([]);
  await expect(row.getByRole('button', { name: 'Mark seen', exact: true })).toBeVisible();
});

test('collection navigation retires a busy frame but joins the same one notice and binding', async ({
  page,
}) => {
  await mount(page, 'reading-busy');
  const row = page.locator('#ask-page-fixture [data-collection-item]').first();
  const passage = row.locator('.chapter-passage');
  await passage.getByRole('button', { name: 'Mark read', exact: true }).first().click();
  await expect
    .poll(async () => (await run(page, 'proof')).actions)
    .toEqual(['status:resolve', 'notify:Agent 1']);
  await row.locator('h2 a').click();
  await expect(page.getByRole('button', { name: '← Collection', exact: true })).toBeVisible();
  await expect(page.locator('#ask-page-fixture [data-chapter-passage] iframe')).toHaveCount(0);
  // Scrolling to either control can legitimately replace an intersecting reference. The
  // cleanup assertion covers every lease actually admitted, including those replacements.
  const admitted = (await run(page, 'proof')).chapter;
  expect(admitted.acquisitions).toBeGreaterThan(admitted.releases);
  await run(page, 'finishStatusNotice');
  await expect
    .poll(async () => (await run(page, 'proof')).chapter.releases)
    .toBe(admitted.acquisitions);
  expect((await run(page, 'proof')).chapter.acquisitions).toBe(admitted.acquisitions);
  await expect(passage).toHaveCount(0);
  const proof = await run(page, 'proof');
  expect(proof.records).toHaveLength(1);
  expect(proof.actions).toEqual(['status:resolve', 'notify:Agent 1']);
  expect(proof.changes).toEqual([]);
});

test('collection progress leaves Today needs-you counts and chapter order alone', async ({
  page,
}) => {
  await mount(page);
  await page
    .locator('#ask-page-fixture [data-collection-item]')
    .first()
    .getByRole('button', { name: 'Mark seen', exact: true })
    .click();
  await expect(page.locator('#ask-page-fixture [data-collection-item]').first()).toHaveAttribute(
    'data-seen',
    'true',
  );
  await page
    .locator('#ask-page-fixture .home-nav')
    .getByRole('link', { name: 'Today', exact: true })
    .click();
  await expect(page.locator('#ask-page-fixture .today-count')).toContainText(
    '2 things wait for you',
  );
  await expect(page.locator('#ask-page-fixture .today-title a')).toHaveText('Release plan');
  expect((await run(page, 'proof')).records).toEqual([]);
  expect((await run(page, 'proof')).actions).toEqual([]);
});
