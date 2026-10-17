import { expect, test, type Page } from '@playwright/test';
import { openAllPages } from '../test/all-pages.js';
import { capturePath } from './captures.js';

test.use({ timezoneId: 'UTC' });

async function mount(page: Page, kind = 'few', hash = '/') {
  await page.goto('/');
  await page.clock.setFixedTime(new Date('2026-10-17T09:30:00Z'));
  await page.evaluate(
    async ({ kind, hash }) => {
      const path = '/test/today-browser.tsx';
      await (await import(/* @vite-ignore */ path)).mountToday(kind, hash);
    },
    { kind, hash },
  );
}
/** The fixture mounts beside the hidden app, which renders the same route; scope to the fixture. */
const app = (page: Page) => page.locator('#today-fixture');
/** The hero greeting; the header carries its own title heading. */
const greeting = (page: Page) => app(page).locator('.today h1');
const chapters = (page: Page) => app(page).locator('.today-chapter');
const titles = (page: Page) => chapters(page).locator('.today-title');
const metric = (page: Page, word: string) =>
  app(page).getByRole('list', { name: 'Overview' }).getByRole('listitem').filter({ hasText: word });
const nav = (page: Page) => app(page).getByRole('navigation', { name: 'Home views' });

test('waiting pages come first, then the rest by update time, each with its chapter facts', async ({
  page,
}) => {
  await mount(page);
  await expect(titles(page)).toHaveText([
    'Budget review',
    'Vendor contract',
    'Notes from standup',
    'Research log',
    'Old draft',
  ]);
  const kickers = chapters(page).locator('.today-kicker');
  await expect(kickers).toHaveText([
    '◆ Waiting',
    '◆ Waiting',
    'Updated 2 hours ago',
    'Updated 3 days ago',
    'Updated 5 days ago',
  ]);
  // Waiting chapters name what they wait for, in the shared needs-you band; others only count comments.
  const budget = chapters(page).nth(0);
  await expect(budget.locator('.needs-you-bar')).toContainText('Needs you 1');
  await expect(budget.locator('.today-comments')).toHaveText('3 comments');
  await expect(budget.locator('.today-meta')).toContainText('by Studio agent');
  await expect(budget.locator('.today-label')).toHaveText(['finance', 'q4 plan']);
  await expect(chapters(page).nth(2).locator('.needs-you-bar')).toHaveCount(0);
  // A page without a summary shows only what the list knows: no comment count, no placeholder.
  await expect(chapters(page).nth(4).locator('.today-comments')).toHaveCount(0);
  // A zero count is noise: only a page with comments shows one.
  await expect(chapters(page).nth(2).locator('.today-comments')).toHaveCount(0);
  await expect(chapters(page).nth(3).locator('.today-comments')).toHaveText('5 comments');
  // Archived pages stay in All pages.
  await expect(app(page).getByText('Archived spec')).toHaveCount(0);
  await expect(app(page).locator('.today-fold')).toHaveCount(0);
});

test('the hero reads the clock and the counts; a metric jumps to its first chapter', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 520 });
  await mount(page, 'many');
  await expect(greeting(page)).toHaveText('Good morning');
  await expect(app(page).locator('.today-date')).toHaveText('Saturday, October 17');
  await expect(app(page).locator('.today-count')).toHaveText('◆ 1 thing waits for you');
  await expect(metric(page, 'Needs you')).toHaveText('1Needs you');
  await expect(metric(page, 'Updated today')).toHaveText('1Updated today');
  await expect(metric(page, 'All pages')).toHaveText('15All pages');
  // The last of the recent chapters sits far below the first screen.
  const last = chapters(page).last().locator('a');
  await expect(last).not.toBeInViewport();
  await metric(page, 'Updated today').getByRole('button').click();
  await expect(app(page).locator('[data-today-updated-today] a')).toBeFocused();
  await expect(app(page).locator('[data-today-updated-today] a')).toBeInViewport();
  await expect(chapters(page).and(app(page).locator('[data-updated-today]'))).toHaveCount(1);
  await page.evaluate(() => scrollTo(0, document.body.scrollHeight));
  await metric(page, 'Needs you').getByRole('button').click();
  await expect(app(page).locator('[data-today-waiting] a')).toBeFocused();
  await expect(app(page).locator('[data-today-waiting] a')).toBeInViewport();
  // The location hash holds the mounted route: a jump never writes to it.
  expect(new URL(page.url()).hash).toBe('#/');
});

test('the greeting follows the viewer clock', async ({ page }) => {
  for (const [time, word] of [
    ['2026-10-17T09:30:00Z', 'Good morning'],
    ['2026-10-17T14:00:00Z', 'Good afternoon'],
    ['2026-10-17T20:00:00Z', 'Good evening'],
  ] as const) {
    await page.goto('/');
    await page.clock.setFixedTime(new Date(time));
    await page.evaluate(async () => {
      const path = '/test/today-browser.tsx';
      await (await import(/* @vite-ignore */ path)).mountToday('few');
    });
    await expect(greeting(page)).toHaveText(word);
  }
});

test('more recent pages than chapters fold into one-line rows, newest first', async ({ page }) => {
  await mount(page, 'many');
  // One waiting page plus the ten most recent others.
  await expect(chapters(page)).toHaveCount(11);
  const rows = app(page).getByRole('list', { name: 'Older pages' }).getByRole('listitem');
  await expect(rows).toHaveText([/^Page 11/, /^Page 12/, /^Page 13/, /^Page 14/]);
  await expect(rows.first().locator('a')).toHaveAttribute('href', /pages\//);
});

test('nothing waiting reads as settled; no pages names the create command', async ({ page }) => {
  await mount(page, 'quiet');
  await expect(app(page).locator('.today-count')).toHaveText('✓ Nothing waits for you');
  await expect(metric(page, 'Needs you').getByRole('button')).toBeDisabled();
  await expect(nav(page).getByRole('button', { name: 'Waiting' })).toBeDisabled();
  await expect(chapters(page).locator('.needs-you-bar')).toHaveCount(0);
  await mount(page, 'none');
  await expect(greeting(page)).toHaveText('Good morning');
  await expect(app(page).locator('.today-empty')).toContainText('No pages in this space yet.');
  await expect(app(page).locator('.today-empty code')).toHaveText(
    'tmt colab page create --title "Notes"',
  );
  await expect(app(page).locator('.today-metrics')).toHaveCount(0);
  await expect(app(page).locator('.today-count')).toHaveCount(0);
});

test('the nav moves between Today and All pages; Waiting from All pages jumps once Today shows', async ({
  page,
}) => {
  await mount(page);
  await expect(nav(page).getByRole('link', { name: 'Today' })).toHaveAttribute(
    'aria-current',
    'page',
  );
  await openAllPages(page, app(page));
  expect(new URL(page.url()).hash).toBe('#/all');
  await expect(app(page).locator('.pages > li')).toHaveCount(5);
  await expect(nav(page).getByRole('link', { name: 'All pages' })).toHaveAttribute(
    'aria-current',
    'page',
  );
  await nav(page).getByRole('button', { name: 'Waiting' }).click();
  expect(new URL(page.url()).hash).toBe('#/');
  await expect(app(page).locator('[data-today-waiting] a')).toBeFocused();
  await nav(page).getByRole('link', { name: 'All pages' }).click();
  await nav(page).getByRole('link', { name: 'Today' }).click();
  await expect(greeting(page)).toHaveText('Good morning');
  // The All pages metric is the same destination.
  await metric(page, 'All pages').getByRole('link').click();
  expect(new URL(page.url()).hash).toBe('#/all');
});

test('a home location that still carries list search opens All pages with it', async ({ page }) => {
  await mount(page, 'few', `/?label=${encodeURIComponent(JSON.stringify(['design']))}`);
  await expect(app(page).locator('.pages > li')).toHaveText([/Research log/]);
  const hash = new URL(page.url()).hash;
  expect(hash).toMatch(/^#\/all\?/);
  expect(hash).toContain('label');
  await expect(app(page).locator('.home h1')).toBeVisible();
});

test('a home location whose list search cannot be read stays on Today', async ({ page }) => {
  await mount(page, 'few', '/?label=design&q=123');
  await expect(greeting(page)).toHaveText('Good morning');
  expect(new URL(page.url()).hash).toBe('#/');
});

test('a chapter title opens its page; the search control works from Today', async ({ page }) => {
  await mount(page);
  await app(page).getByRole('button', { name: 'Search pages', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Search pages', exact: true });
  await expect(dialog).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(dialog).toBeHidden();
  await titles(page).nth(2).locator('a').click();
  expect(new URL(page.url()).hash).toMatch(/^#\/pages\/d0000003/);
});

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    for (const kind of [
      'few',
      'readings',
      'reading-one',
      'reading-unknown',
      'long',
      'quiet',
      'none',
      'mixed',
      'many',
      'checking',
      'partial',
      'summary',
      'summary-stale',
    ] as const)
      test(`Today look ${kind} ${width} ${theme}: one column of chapters, no overflow`, async ({
        page,
      }) => {
        await page.setViewportSize({ width, height: 900 });
        await page.emulateMedia({ colorScheme: theme });
        await mount(page, kind);
        await page.evaluate((value) => {
          document.documentElement.dataset.theme = value;
        }, theme);
        await expect(greeting(page)).toBeVisible();
        await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
        const metrics = await page.evaluate(() => {
          const root = document.querySelector('#today-fixture .today')!.getBoundingClientRect();
          const first = document.querySelector('#today-fixture .today-chapter article');
          const main = first?.querySelector('.today-chapter-main')?.getBoundingClientRect();
          const state = first?.querySelector('.today-chapter-state')?.getBoundingClientRect();
          const mark = document.querySelector('#today-fixture .today-chapter .today-mark');
          const rule = document.querySelector('#today-fixture .today-chapter');
          const text = getComputedStyle(document.querySelector('#today-fixture .today h1')!).color;
          const ink = (() => {
            const probe = document.createElement('i');
            probe.style.color = 'var(--tmt-ui-color-selected-text)';
            document.body.append(probe);
            const value = getComputedStyle(probe).color;
            probe.remove();
            return value;
          })();
          const title = document.querySelector('#today-fixture .today-title a');
          const numeral = document.querySelector('#today-fixture .today-numeral');
          return {
            overflow: document.documentElement.scrollWidth > innerWidth,
            width: root.width,
            stacked: main && state ? state.top >= main.bottom - 1 : undefined,
            sideBySide: main && state ? state.left >= main.right - 1 : undefined,
            mark: mark ? getComputedStyle(mark).color : undefined,
            text,
            ink,
            title: title ? getComputedStyle(title).color : undefined,
            numeral: numeral ? getComputedStyle(numeral).color : undefined,
            shadow: rule ? getComputedStyle(rule).boxShadow : 'none',
            ruleWidth: rule ? getComputedStyle(rule).borderTopWidth : '0px',
            ruleStyle: rule ? getComputedStyle(rule).borderTopStyle : 'none',
          };
        });
        expect(metrics.overflow).toBe(false);
        // Hero, chapter titles and numerals use the strongest ink in both themes.
        expect(metrics.text).toBe(metrics.ink);
        if (metrics.title) expect(metrics.title).toBe(metrics.ink);
        if (metrics.numeral) expect(metrics.numeral).toBe(metrics.ink);
        expect(metrics.width).toBeLessThanOrEqual(1120);
        expect(metrics.shadow).toBe('none');
        if (kind === 'few' || kind === 'long') {
          expect(metrics.ruleStyle).toBe('solid');
          // Only the waiting ◆ takes the attention colour.
          expect(metrics.mark).not.toBe(metrics.text);
          if (width === 390) expect(metrics.stacked).toBe(true);
          else expect(metrics.sideBySide).toBe(true);
        }
        await page.screenshot({
          path: capturePath(`today/today-${kind}-${width}-${theme}.png`),
          fullPage: true,
        });
      });

test('known pending items are a lower bound until every active page summary is known', async ({
  page,
}) => {
  await mount(page, 'partial');
  await expect(app(page).locator('.today-count')).toHaveText('◆ 3+ things wait for you');
  await expect(metric(page, 'Needs you')).toHaveText('3+Needs you');
  await expect(metric(page, 'Needs you').getByRole('button')).toBeEnabled();
  await expect(chapters(page).first().locator('.needs-you-bar')).toContainText('Needs you 3');
  await expect(chapters(page).nth(1).locator('.needs-you-bar')).toHaveCount(0);
  await mount(page, 'checking');
  await expect(app(page).locator('.today-count')).toHaveText('Checking pages…');
  await expect(app(page).locator('.today-count .today-mark')).toHaveCount(0);
  await expect(metric(page, 'Needs you')).toHaveText('—Needs you');
  await expect(metric(page, 'Needs you').getByRole('button')).toBeDisabled();
  await expect(chapters(page).locator('.needs-you-bar')).toHaveCount(0);
  // Existing route navigation reads again, without remounting the router or adding a poll.
  const before = await page.evaluate(async () => {
    const fixture = '/test/today-browser.tsx';
    const app = await import(/* @vite-ignore */ fixture);
    app.nextHome('mixed');
    return app.homeReadCount();
  });
  await openAllPages(page, app(page));
  await nav(page).getByRole('link', { name: 'Today', exact: true }).click();
  expect(
    await page.evaluate(async () => {
      const fixture = '/test/today-browser.tsx';
      return (await import(/* @vite-ignore */ fixture)).homeReadCount();
    }),
  ).toBeGreaterThan(before);
  await expect(app(page).locator('.today-count')).toHaveText('◆ 5 things wait for you');
  await expect(metric(page, 'Needs you')).toHaveText('5Needs you');
  await expect(chapters(page).locator('.needs-you-bar')).toHaveText([
    '◆ Needs you 4',
    '◆ Needs you 1',
  ]);
});

for (const kind of ['readings', 'reading-one', 'reading-unknown'] as const) {
  test(`chapter reading links are cosmetic and bounded (${kind})`, async ({ page }) => {
    await mount(page, kind);
    const first = chapters(page).first();
    await expect(first.locator('.needs-you-bar')).toContainText(
      kind === 'reading-one' ? 'Needs you 1' : 'Needs you 5',
    );
    await expect
      .poll(() =>
        page.evaluate(async () => {
          const fixture = '/test/today-browser.tsx';
          return (await import(/* @vite-ignore */ fixture)).readingDetailReads();
        }),
      )
      .toEqual(['d0000001-1111-4111-8111-000000000001']);
    if (kind === 'reading-unknown') {
      await expect(first.locator('.today-readings')).toHaveCount(0);
      await expect(first.locator('.today-comments')).toHaveText('2 comments');
      return;
    }
    const list = first.locator('.today-chapter-state .today-readings');
    await expect(list.locator('.today-readings-kicker')).toHaveText('Please read');
    await expect(list.getByRole('listitem')).toHaveCount(kind === 'reading-one' ? 1 : 3);
    const link = list.getByRole('link', { name: 'Deployment plan', exact: true });
    await expect(link).toHaveAttribute('href', /read=20000000-0000-4000-8000-000000000001/);
    if (kind === 'readings') {
      const more = list.getByRole('link', { name: '+2 more', exact: true });
      await expect(more).toHaveAttribute('href', (await link.getAttribute('href')) as string);
      const comments = first.locator('.today-comments');
      const textStyle = (node: Element) => {
        const style = getComputedStyle(node);
        return { color: style.color, fontSize: style.fontSize };
      };
      expect(await more.evaluate(textStyle)).toEqual(await comments.evaluate(textStyle));
      const reason = list.locator('.today-reading-reason').nth(1);
      expect(await reason.evaluate((node) => getComputedStyle(node).textOverflow)).toBe('ellipsis');
    }
    await expect(first.locator('.today-comments')).toHaveText('2 comments');
    await expect(first.getByRole('button', { name: 'Mark read' })).toHaveCount(0);
  });
}

const summary = (page: Page) => app(page).locator('.today-summary');

test('the maintainer summary sits under the greeting with who wrote it and when', async ({
  page,
}) => {
  await mount(page, 'summary');
  await expect(summary(page)).toBeVisible();
  await expect(summary(page).locator('.today-summary-line')).toHaveText([
    'Two pages changed',
    'Budget review waits on you',
  ]);
  await expect(summary(page).locator('.today-summary-meta')).toHaveText(
    'Studio agent · 1 hour ago',
  );
  await expect(summary(page).locator('.today-summary-note')).toHaveCount(0);
  await expect(summary(page)).toHaveAccessibleName('Since the last update');
  // Under the greeting, above the counts.
  const box = async (locator: ReturnType<typeof summary>) => (await locator.boundingBox())!;
  const above = await box(greeting(page));
  const mid = await box(summary(page));
  const below = await box(app(page).locator('.today-count'));
  expect(mid.y).toBeGreaterThanOrEqual(above.y + above.height - 1);
  expect(below.y).toBeGreaterThanOrEqual(mid.y + mid.height - 1);
});

test('a summary older than the newest page change stays, with a muted note and never "no changes"', async ({
  page,
}) => {
  await mount(page, 'summary-stale');
  await expect(summary(page).locator('.today-summary-line')).toHaveText([
    'Two pages changed',
    'Budget review waits on you',
  ]);
  await expect(summary(page).locator('.today-summary-meta')).toHaveText(
    'Studio agent · 5 hours ago',
  );
  await expect(summary(page).locator('.today-summary-note')).toHaveText(
    'Pages have changed since this was written.',
  );
  await expect(app(page).getByText(/no changes/i)).toHaveCount(0);
});

test('a summary without a writer shows only when it was written; none shows no block', async ({
  page,
}) => {
  await mount(page, 'summary-anonymous');
  await expect(summary(page).locator('.today-summary-meta')).toHaveText('1 hour ago');
  await mount(page, 'few');
  await expect(chapters(page).first()).toBeVisible();
  await expect(summary(page)).toHaveCount(0);
});
