import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';
import { readFileSync } from 'node:fs';
import { text } from '../src/strings.js';

test('the actual isolated Worker admits the shared shell values and refuses malformed raw updates', async ({
  page,
}) => {
  const corpus = JSON.parse(
    readFileSync(new URL('../../../contracts/vectors/shell-v1.json', import.meta.url), 'utf8'),
  );
  await page.route('**/src/main.tsx', (route) =>
    route.fulfill({ contentType: 'text/javascript', body: '' }),
  );
  await page.goto('/');
  const result = await page.evaluate(async (cases) => {
    const path = '/test/shell-browser.tsx';
    return (await import(path)).checkCases(cases);
  }, corpus.cases);
  const expected = corpus.cases
    .filter((c: { action?: string }) => !c.action)
    .map((c: { name: string; valid: boolean; document: unknown }) => ({
      name: c.name,
      accepted: c.valid,
      ...(c.valid ? { shell: c.document, source: '' } : {}),
    }));
  expect(result).toEqual(expected);
});

const PLAN = '10000000-0000-4000-8000-0000000000a1';
const NOTES = '10000000-0000-4000-8000-0000000000a2';
const READING = '30000000-0000-4000-8000-0000000000b1';

async function openShell(page: Page, kind = 'full') {
  await page.route('**/src/main.tsx', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `import { mount } from "/test/shell-browser.tsx"; mount(${JSON.stringify(kind)});`,
    }),
  );
  await page.goto('/');
}

const aside = (page: Page) => page.locator('#shell-aside');
const chapterRows = (page: Page) => page.locator('.shell-row-item[data-stage]');
const rowButton = (page: Page, pageId: string) =>
  page.locator(`.shell-row-item[data-page-id="${pageId}"] button.shell-row`);

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`a shell page reads as a head, one card of counts, a Pages list and the selected page ${width} ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await openShell(page);
      await expect(page.locator('iframe')).toHaveCount(0);
      await expect(page.locator('header').first()).toContainText('Release shell');
      // The page owns its headline; the greeting is its subtitle and names the local time of day
      // and the number waiting (1 + 2), spelled out.
      // (The shell bar still carries the title as well, until the shared top bar drops it, #4779.)
      await expect(page.locator('.tmt-ui-page-title')).toHaveText('Release shell');
      await expect(page.locator('h1.tmt-ui-page-title')).toHaveCount(1);
      await expect(page.locator('.tmt-ui-page-subtitle')).toHaveText(
        /^Good (morning|afternoon|evening)\. Three things need you\.$/,
      );
      await expect(page.locator('.shell-glance > h2')).toHaveText(text.shellPage.glance);
      await expect(page.locator('.shell-summary-line1')).toHaveText('Release is on track');
      await expect(page.locator('.shell-summary-line2')).toHaveText(
        'Two reviews still need a decision before Friday.',
      );
      await expect(page.locator('.shell-summary-meta .agent-chip')).toHaveText('Atlas');
      // The three counts.
      const counts = page.locator('.shell-counts li');
      await expect(counts.locator('.shell-count-number')).toHaveText(['3', '1', '3']);
      await expect(counts.locator('.shell-count-label')).toHaveText([
        'Needs you',
        'In progress',
        'Updated today',
      ]);
      // Waiting pages first, then in progress; the archived page and the vanished one are not chapters.
      const chapters = chapterRows(page);
      await expect(chapters).toHaveCount(3);
      await expect(chapters.locator('.shell-row-title')).toHaveText([
        'Release plan',
        'Design notes',
        'Weekly report',
      ]);
      await expect(chapters.locator('.shell-row-number')).toHaveText(['01', '02', '03']);
      await expect(chapters.locator('.shell-row-status')).toHaveText([
        'Needs you',
        'Needs you',
        'In progress',
      ]);
      await expect(chapters.first().locator('.shell-row-meta')).toContainText('tmt-feat-lead');
      await expect(chapters.first().locator('.shell-row-label')).toHaveText('Design');
      // The first chapter is selected and its passage is the page beside the list; choosing a row
      // shows that chapter's passage: what most needs the owner, else the latest comment.
      await expect(chapters.first().locator('button')).toHaveAttribute('aria-current', 'true');
      await expect(aside(page).locator('.shell-aside-title')).toHaveText('Release plan');
      await expect(aside(page).locator('.shell-passage-lead')).toContainText('To confirm');
      await expect(aside(page).locator('.shell-passage-title')).toHaveText(
        'Review the release scope',
      );
      await expect(aside(page).locator('.shell-passage-more')).toHaveText('1 more waiting');
      await chapters.nth(1).locator('button').click();
      await expect(chapters.nth(1).locator('button')).toHaveAttribute('aria-current', 'true');
      await expect(chapters.first().locator('button')).not.toHaveAttribute('aria-current', 'true');
      await expect(aside(page).locator('.shell-passage-lead')).toContainText('Question for you');
      await expect(aside(page).locator('.shell-passage-title')).toHaveText('Which hosting plan?');
      await chapters.nth(2).locator('button').click();
      await expect(aside(page).locator('.shell-passage-lead')).toHaveText('Latest');
      await expect(aside(page).locator('.shell-passage-text')).toHaveText(
        'Merged the review blocks.',
      );
      await chapters.first().locator('button').click();
      // Older pages follow in the same list, under their own heading, newest first.
      await expect(page.locator('.shell-earlier-heading')).toHaveText('Earlier · 2 pages');
      const earlier = page.locator('.shell-earlier-heading + ol .shell-row-item');
      await expect(earlier.locator('.shell-row-title')).toHaveText([
        'Usage chart round two',
        'Last quarter',
      ]);
      await expect(earlier.locator('.shell-row-number')).toHaveText(['04', '05']);
      await expect(page.locator('.shell-state[data-unavailable]')).toHaveText(
        "1 page can't be shown",
      );
      // One rounded card with a rule edge holds it all; the list and the selected page sit side by
      // side where there is room and stack where there is not; nothing scrolls sideways.
      const geometry = await page.evaluate(() => {
        const rect = (node: Element) => node.getBoundingClientRect();
        const card = document.querySelector('.shell-glance')!;
        const style = getComputedStyle(card);
        const list = rect(document.querySelector('.shell-pages')!);
        const side = rect(document.querySelector('#shell-aside')!);
        const stat = getComputedStyle(document.querySelector('.shell-stat')!);
        const row = getComputedStyle(document.querySelector('.shell-row')!);
        return {
          sideBySide: side.left > list.left + 100 && Math.abs(side.top - list.top) < 4,
          stacked: side.top >= list.bottom - 1,
          pageWidth: rect(document.querySelector('.shell-page')!).width,
          scrollWidth: document.documentElement.scrollWidth,
          clientWidth: document.documentElement.clientWidth,
          cardRadius: Number.parseFloat(style.borderTopLeftRadius),
          cardRule: style.borderTopWidth,
          cardShadow: style.boxShadow,
          statRadius: Number.parseFloat(stat.borderTopLeftRadius),
          rowRadius: row.borderTopLeftRadius,
        };
      });
      expect(width >= 720 ? geometry.sideBySide : geometry.stacked).toBe(true);
      expect(geometry.pageWidth).toBeLessThanOrEqual(1680);
      expect(geometry.scrollWidth).toBeLessThanOrEqual(geometry.clientWidth);
      expect(geometry.cardRadius).toBeGreaterThan(0);
      expect(geometry.statRadius).toBeGreaterThan(0);
      expect(geometry).toMatchObject({ cardRule: '1px', cardShadow: 'none', rowRadius: '0px' });
      await chapters.first().locator('button').focus();
      await page.screenshot({
        path: capturePath(`shell-page-${width}-${theme}.png`),
        fullPage: true,
      });
    });
  }

test('each count jumps to the first row it counts and a zero count is plain text', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 500 });
  await openShell(page);
  const counts = page.locator('.shell-counts li');
  await counts.nth(1).getByRole('button').click();
  await expect(page.locator('.shell-row-item[data-stage="in-progress"] button')).toBeFocused();
  await expect(page.locator('.shell-row-item[data-stage="in-progress"] button')).toHaveAttribute(
    'aria-current',
    'true',
  );
  await counts.nth(0).getByRole('button').click();
  await expect(page.locator('.shell-row-item[data-stage="waiting"] button').first()).toBeFocused();
  await counts.nth(2).getByRole('button').click();
  await expect(page.locator('.shell-row-item[data-today] button').first()).toBeFocused();
  await page.unroute('**/src/main.tsx');
  await openShell(page, 'settled');
  await expect(page.locator('.tmt-ui-page-subtitle')).toHaveText(/Nothing needs you\.$/);
  await expect(page.locator('.shell-counts li').first().getByRole('button')).toHaveCount(0);
  await expect(page.locator('.shell-counts li').nth(1).getByRole('button')).toHaveCount(1);
});

test('rows select with Enter, arrows only move between them, and a row is a button', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await openShell(page);
  const first = rowButton(page, PLAN);
  const second = rowButton(page, NOTES);
  await expect(page.locator('a.shell-row, .shell-row-item a')).toHaveCount(0);
  await first.focus();
  await page.keyboard.press('ArrowDown');
  await expect(second).toBeFocused();
  // Moving focus selects nothing: the page beside the list is still the first one.
  await expect(first).toHaveAttribute('aria-current', 'true');
  await expect(aside(page).locator('.shell-aside-title')).toHaveText('Release plan');
  await page.keyboard.press('Enter');
  await expect(second).toHaveAttribute('aria-current', 'true');
  await expect(first).not.toHaveAttribute('aria-current', 'true');
  await expect(aside(page).locator('.shell-aside-title')).toHaveText('Design notes');
  await page.keyboard.press('ArrowUp');
  await expect(first).toBeFocused();
  await page.keyboard.press('Space');
  await expect(aside(page).locator('.shell-aside-title')).toHaveText('Release plan');
  // An older page is a row as well: it shows its title and the way to open it, no passage.
  await page.locator('.shell-earlier-heading + ol .shell-row').first().click();
  await expect(aside(page).locator('.shell-aside-title')).toHaveText('Usage chart round two');
  await expect(aside(page).locator('.shell-passage')).toHaveCount(0);
  await expect(aside(page).getByRole('link', { name: text.shellPage.openFullPage })).toHaveCount(1);
});

test('a reading item names the pending reading that leads its chapter, and opening goes there', async ({
  page,
}) => {
  // Below the panel width a chapter has no panel: it opens its page.
  await page.setViewportSize({ width: 600, height: 900 });
  await openShell(page, 'named');
  await expect(aside(page).locator('.shell-passage-lead')).toContainText('To read');
  await expect(aside(page).locator('.shell-passage-title')).toHaveText('Decide the scope');
  await expect(aside(page).locator('.shell-passage-why')).toHaveText('Read before the review');
  const open = aside(page).getByRole('link', { name: 'Open to decide →' });
  await expect(open).toHaveAttribute('href', `/#/pages/${PLAN}?read=${READING}`);
  // The full-page link carries no target of its own.
  await expect(
    aside(page).getByRole('link', { name: text.shellPage.openFullPage }),
  ).toHaveAttribute('href', `/#/pages/${PLAN}`);
});

test.describe('a chapter opens in a panel beside the shell from 900 px', () => {
  const panel = (page: Page) => page.getByRole('dialog', { name: 'Release plan' });
  const trigger = (page: Page) =>
    page.locator(`#shell-aside[data-page-id="${PLAN}"] button[data-opens-panel]`);

  test('Open to decide shows what waits on that page, in the URL, and Esc closes it', async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await openShell(page, 'named');
    await trigger(page).click();
    await expect(panel(page)).toBeVisible();
    await expect(page).toHaveURL(new RegExp(`\\?open=${PLAN}`));
    await expect(panel(page).locator('.shell-panel-needs li').first()).toContainText(
      'Review the release scope',
    );
    await expect(panel(page).locator('.shell-panel-needs li')).toContainText([
      'Review the release scope',
      'Decide the scope',
    ]);
    // Every waiting item has its own action: the reading opens its passage, the others the page
    // (the chapter's anchor belongs to the shell item, not to a review or a question).
    const items = panel(page).locator('.shell-panel-needs li');
    for (let index = 0; index < (await items.count()); index += 1)
      await expect(items.nth(index).getByRole('link')).toHaveCount(1);
    await expect(items.first().getByRole('link')).toHaveText(text.shellPage.openOnPage);
    await expect(items.nth(1).getByRole('link')).toHaveText(text.shellPage.openPassage);
    await expect(
      panel(page).getByRole('link', { name: text.shellPage.openFullPage }),
    ).toHaveAttribute('href', new RegExp(`/#/pages/${PLAN}$`));
    await page.keyboard.press('Escape');
    await expect(panel(page)).toHaveCount(0);
    await expect(page).not.toHaveURL(/open=/);
    await expect(trigger(page)).toBeFocused();
  });

  for (const theme of ['light', 'dark'] as const)
    test(`the panel at 1440 in ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width: 1440, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await openShell(page, 'named');
      await trigger(page).click();
      await expect(panel(page)).toBeVisible();
      await page.screenshot({ path: capturePath(`shell-panel-1440-${theme}.png`) });
    });

  test('a chapter named by an anchor item with nothing waiting offers that passage', async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await openShell(page, 'details-failed');
    // Its details did not come, so the panel opens only from the address, with the item's note.
    await page.evaluate((id) => {
      location.hash = `${location.hash.split('?')[0]}?open=${id}`;
    }, NOTES);
    const link = page.getByRole('dialog').getByRole('link', { name: text.shellPage.openPassage });
    await expect(link).toHaveAttribute('href', `/#/pages/${NOTES}?anchor=decisions`);
  });

  test('Back closes it, Forward opens it again, a reload keeps it, a backdrop click closes it', async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await openShell(page);
    await trigger(page).click();
    await expect(panel(page)).toBeVisible();
    await page.goBack();
    await expect(panel(page)).toHaveCount(0);
    await page.goForward();
    await expect(panel(page)).toBeVisible();
    await page.reload();
    await expect(panel(page)).toBeVisible();
    await page.mouse.click(40, 450);
    await expect(panel(page)).toHaveCount(0);
    await expect(page).not.toHaveURL(/open=/);
  });

  test('a page the shell does not list opens nothing, and narrow windows have no panel at all', async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await openShell(page);
    await page.evaluate(() => {
      location.hash = location.hash.split('?')[0] + '?open=10000000-0000-4000-8000-0000000000ff';
    });
    await expect(chapterRows(page)).toHaveCount(3);
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await page.setViewportSize({ width: 390, height: 844 });
    await expect(page.locator('button[data-opens-panel]')).toHaveCount(0);
    await expect(aside(page).getByRole('link', { name: 'Open to decide →' })).toHaveCount(1);
  });

  test('opening the full page from the selected page and coming back keeps the shell where it was', async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 500 });
    await openShell(page);
    const link = aside(page).getByRole('link', { name: text.shellPage.openFullPage });
    await link.scrollIntoViewIfNeeded();
    const before = await page.evaluate(() => window.scrollY);
    expect(before).toBeGreaterThan(100);
    await link.click();
    await expect(page).toHaveURL(/#\/pages\/[^?]+$/);
    await expect(page.locator('.shell-page')).toHaveCount(0);
    await page.goBack();
    await expect(chapterRows(page)).toHaveCount(3);
    expect(Math.abs((await page.evaluate(() => window.scrollY)) - before)).toBeLessThan(40);
  });
});

test('opening a chapter and coming back lands on the shell with that chapter selected and focused', async ({
  page,
}) => {
  await openShell(page);
  await rowButton(page, NOTES).click();
  await aside(page).getByRole('link', { name: text.shellPage.openFullPage }).click();
  await expect(page).toHaveURL(new RegExp(`#/pages/${NOTES}$`));
  await expect(page.locator('.shell-page')).toHaveCount(0);
  await page.goBack();
  await expect(chapterRows(page)).toHaveCount(3);
  await expect(rowButton(page, NOTES)).toBeFocused();
  await expect(rowButton(page, NOTES)).toHaveAttribute('aria-current', 'true');
  await expect(aside(page).locator('.shell-aside-title')).toHaveText('Design notes');
});

test('an anchor item opens its page at that exact element and says when it is not there', async ({
  page,
}) => {
  await page.setViewportSize({ width: 600, height: 900 });
  await openShell(page);
  // The notes page is named by an anchor item: its passage opens the page at that element.
  await rowButton(page, NOTES).click();
  const anchor = aside(page).getByRole('link', { name: 'Open to decide →' });
  await expect(anchor).toHaveAttribute('href', `/#/pages/${NOTES}?anchor=decisions`);
  await anchor.click();
  await expect(page).toHaveURL(new RegExp(`#/pages/${NOTES}\\?anchor=decisions$`));
  await expect(page.locator('iframe')).toHaveCount(1);
  await expect(page.getByRole('status').filter({ hasText: text.anchorMissing })).toHaveCount(0);
  const frame = page.frameLocator('iframe');
  await expect(frame.locator('#decisions')).toBeVisible();
  await expect.poll(() => page.evaluate(() => window.scrollY)).toBeGreaterThan(1500);
  // The fixture page has no element for a name that is not in it: the page opens, with a note.
  await page.evaluate((id) => {
    location.hash = `/pages/${id}?anchor=absent`;
  }, NOTES);
  await expect(page.getByText(text.anchorMissing, { exact: true })).toBeVisible();
});

test('a shell reads the details of at most twenty chapters', async ({ page }) => {
  await openShell(page, 'details-many');
  await expect(chapterRows(page)).toHaveCount(25);
  await expect.poll(() => page.evaluate(() => window.detailCalls.length)).toBe(20);
  expect(await page.evaluate(() => new Set(window.detailCalls).size)).toBe(20);
  await page.waitForTimeout(300);
  expect(await page.evaluate(() => window.detailCalls.length)).toBe(20);
});

test('a failed details read leaves each chapter with its note and no passage, never loading forever', async ({
  page,
}) => {
  await openShell(page, 'details-failed');
  await expect(chapterRows(page)).toHaveCount(3);
  await expect(page.locator('.shell-passage-pending')).toHaveCount(0);
  await expect(aside(page).locator('.shell-passage-note')).toHaveText('Start with the plan.');
  await expect(aside(page).locator('.shell-passage-lead')).toHaveCount(0);
  await expect(
    aside(page).getByRole('link', { name: text.shellPage.openFullPage }),
  ).toHaveAttribute('href', `/#/pages/${PLAN}`);
});

test('the focus ring on a row stays inside the list card, whose edge clips anything outside it', async ({
  page,
}) => {
  await openShell(page, 'full');
  const geometry = await page.evaluate(() => {
    const row = document.querySelector<HTMLElement>('.shell-row')!;
    row.focus({ focusVisible: true } as FocusOptions);
    const style = getComputedStyle(row);
    const box = row.getBoundingClientRect();
    const card = document.querySelector('.shell-pages')!.getBoundingClientRect();
    const reach = Number.parseFloat(style.outlineOffset) + Number.parseFloat(style.outlineWidth);
    return {
      matches: row.matches(':focus-visible'),
      width: Number.parseFloat(style.outlineWidth),
      left: box.left - reach - card.left,
      right: card.right - (box.right + reach),
    };
  });
  expect(geometry.matches).toBe(true);
  expect(geometry.width).toBeGreaterThan(0);
  expect(geometry.left).toBeGreaterThanOrEqual(0);
  expect(geometry.right).toBeGreaterThanOrEqual(0);
});

test('an empty shell, one without a summary and a failed page list stay readable', async ({
  page,
}) => {
  await openShell(page, 'empty');
  await expect(page.getByText(text.shellPage.empty, { exact: true })).toBeVisible();
  await expect(page.locator('.tmt-ui-page-subtitle')).toHaveText(/Nothing needs you\.$/);
  await expect(page.locator('.shell-row-item')).toHaveCount(0);
  await page.unroute('**/src/main.tsx');
  await openShell(page, 'no-summary');
  await expect(page.locator('.shell-summary-line1')).toHaveCount(0);
  await expect(page.getByText(text.shellPage.noSummary, { exact: true })).toBeVisible();
  await expect(chapterRows(page)).toHaveCount(3);
  await page.unroute('**/src/main.tsx');
  await openShell(page, 'not-admitted');
  await expect(page.getByText(text.shellPage.notShown, { exact: true })).toBeVisible();
  await expect(page.locator('.shell-row-item')).toHaveCount(0);
  await page.unroute('**/src/main.tsx');
  await openShell(page, 'pages-failed');
  await expect(page.getByText(text.shellPage.pagesUnavailable, { exact: true })).toBeVisible();
  // Titles are not known, but every page is still a row, in the older list, and opens from the
  // selected page.
  await expect(page.locator('.shell-earlier-heading + ol .shell-row-item')).toHaveCount(7);
  await expect(chapterRows(page)).toHaveCount(0);
  await expect(
    aside(page).getByRole('link', { name: text.shellPage.openFullPage }),
  ).toHaveAttribute('href', `/#/pages/${PLAN}`);
});

test('an editor reader link shows the shell notice as read-only, never a Source control', async ({
  page,
}) => {
  await page.route('**/src/main.tsx', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: 'import { mountReader } from "/test/shell-browser.tsx"; mountReader();',
    }),
  );
  await page.goto('/');
  await expect(page.getByText(text.shellPageNotice, { exact: true })).toBeVisible();
  await expect(page.locator('.chip')).toHaveText(text.readerOnly);
  await expect(page.getByTestId('source-toggle')).toHaveCount(0);
  await expect(page.locator('code')).toHaveText(
    'tmt colab dashboard read 10000000-0000-4000-8000-000000000001',
  );
  await expect(page.getByRole('button', { name: 'Copy', exact: true })).toHaveCount(0);
  await expect(page.locator('iframe')).toHaveCount(0);
});
