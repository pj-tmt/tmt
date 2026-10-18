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

async function openShell(page: Page, kind = 'full') {
  await page.route('**/src/main.tsx', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `import { mount } from "/test/shell-browser.tsx"; mount(${JSON.stringify(kind)});`,
    }),
  );
  await page.goto('/');
}

for (const width of [1440, 1024, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`a shell page lists its items as a wide grid of links ${width} ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await openShell(page);
      const cells = page.locator('.shell-cell');
      await expect(cells).toHaveCount(6);
      await expect(page.locator('iframe')).toHaveCount(0);
      await expect(page.locator('header')).toContainText('Release shell');
      // Summary: two lines, who and when, and the pages edited after it.
      await expect(page.locator('.shell-summary-line1')).toHaveText('Release is on track');
      await expect(page.locator('.shell-summary-line2')).toHaveText(
        'Two reviews still need a decision before Friday.',
      );
      await expect(page.locator('.shell-summary-meta')).toContainText('Atlas');
      await expect(page.locator('.shell-changed')).toHaveText(
        ' · 1 page changed since this summary',
      );
      // Items keep the shell's order; each is a real link except the vanished page.
      await expect(cells.locator('.shell-cell-kind')).toHaveText([
        'Page',
        'Passage · decisions',
        'Suggested reading',
        'Page',
        'Page · Archived',
        'Page',
      ]);
      const link = (n: number) => cells.nth(n).locator('a');
      await expect(link(0)).toHaveText('Release plan');
      await expect(link(0)).toHaveAttribute('href', `/#/pages/${PLAN}`);
      await expect(link(1)).toHaveText('Design notes');
      await expect(link(2)).toHaveAttribute(
        'href',
        `/#/pages/${PLAN}?read=30000000-0000-4000-8000-000000000001`,
      );
      await expect(cells.nth(3).locator('a')).toHaveCount(0);
      await expect(cells.nth(3)).toContainText(text.shellPage.unavailable);
      await expect(cells.nth(3)).toContainText('This page was deleted.');
      await expect(link(4)).toHaveText('Last quarter');
      await expect(cells.nth(0)).toContainText('Start with the plan.');
      await expect(cells.nth(0)).toContainText('For Reviewers');
      await expect(cells.nth(0)).toContainText('Needs you 2');
      await expect(cells.nth(1)).not.toContainText('Needs you');
      // Wide and borderless: no card, three columns at 1440, two at 1024, one at phone width.
      const geometry = await page.evaluate(() => {
        const rect = (node: Element) => node.getBoundingClientRect();
        const lefts = [...document.querySelectorAll('.shell-cell')].map((c) =>
          Math.round(rect(c).left),
        );
        const style = getComputedStyle(document.querySelector('.shell-cell')!);
        return {
          columns: new Set(lefts).size,
          pageWidth: rect(document.querySelector('.shell-page')!).width,
          scrollWidth: document.documentElement.scrollWidth,
          clientWidth: document.documentElement.clientWidth,
          shadow: style.boxShadow,
          borderLeft: style.borderLeftWidth,
          radius: style.borderRadius,
        };
      });
      expect(geometry.columns).toBe(width >= 1280 ? 3 : width >= 640 ? 2 : 1);
      expect(geometry.pageWidth).toBeLessThanOrEqual(1680);
      expect(geometry.scrollWidth).toBeLessThanOrEqual(geometry.clientWidth);
      expect(geometry).toMatchObject({ shadow: 'none', borderLeft: '0px', radius: '0px' });
      // The long note stays within two lines and keeps its full text in the title attribute.
      const note = cells.nth(5).locator('.shell-cell-note');
      const lines = await note.evaluate(
        (node) =>
          node.getBoundingClientRect().height / parseFloat(getComputedStyle(node).lineHeight),
      );
      expect(lines).toBeLessThanOrEqual(2.01);
      expect(await note.getAttribute('title')).toContain('A long note that has to wrap');
      // Keyboard focus on the first cell is part of the look capture.
      await link(0).focus();
      await expect(link(0)).toBeFocused();
      await page.screenshot({
        path: capturePath(`shell-page-${width}-${theme}.png`),
        fullPage: true,
      });
    });
  }

test('opening an item and coming back lands on the shell with focus on that item', async ({
  page,
}) => {
  await openShell(page);
  const link = page.locator('.shell-cell').nth(5).locator('a');
  await link.click();
  await expect(page).toHaveURL(new RegExp(`#/pages/${NOTES}$`));
  await expect(page.locator('.shell-page')).toHaveCount(0);
  await page.goBack();
  await expect(page.locator('.shell-cell')).toHaveCount(6);
  await expect(link).toBeFocused();
});

test('an anchor item opens its page at that exact element and says when it is not there', async ({
  page,
}) => {
  await openShell(page);
  const anchor = page.locator('.shell-cell').nth(1).locator('a');
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

test('an empty shell, one without a summary and a failed page list stay readable', async ({
  page,
}) => {
  await openShell(page, 'empty');
  await expect(page.getByText(text.shellPage.empty, { exact: true })).toBeVisible();
  await expect(page.locator('.shell-summary')).toHaveCount(0);
  await page.unroute('**/src/main.tsx');
  await openShell(page, 'no-summary');
  await expect(page.locator('.shell-summary')).toHaveCount(0);
  await expect(page.locator('.shell-cell')).toHaveCount(6);
  await page.unroute('**/src/main.tsx');
  await openShell(page, 'pages-failed');
  await expect(page.getByText(text.shellPage.pagesUnavailable, { exact: true })).toBeVisible();
  // Titles are not known, but every item still links to its page.
  await expect(page.locator('.shell-cell a')).toHaveCount(6);
  await expect(page.locator('.shell-cell').first().locator('a')).toHaveAttribute(
    'href',
    `/#/pages/${PLAN}`,
  );
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
    'tmt colab shell read 10000000-0000-4000-8000-000000000001',
  );
  await expect(page.getByRole('button', { name: 'Copy', exact: true })).toHaveCount(0);
  await expect(page.locator('iframe')).toHaveCount(0);
});
