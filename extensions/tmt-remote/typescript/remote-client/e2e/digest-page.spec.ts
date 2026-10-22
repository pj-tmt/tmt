import { expect, test, type Page } from '@playwright/test';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { build } from 'vite';

const assets = new URL('../../../rust/tmt-remote/assets/', import.meta.url);
const origin = 'http://digest.test';
let harness = '';
let page_html = '';
let css = '';
let root = '';

test.beforeAll(async () => {
  root = await mkdtemp(path.join(tmpdir(), 'digest-page-'));
  await build({
    root: new URL('..', import.meta.url).pathname,
    logLevel: 'silent',
    build: {
      outDir: root,
      emptyOutDir: true,
      minify: false,
      lib: {
        entry: new URL('./digest-harness.ts', import.meta.url).pathname,
        formats: ['es'],
        fileName: () => 'harness.js',
      },
    },
  });
  harness = await readFile(path.join(root, 'harness.js'), 'utf8');
  // The door serves the leaf's static CSS then the pages' own, as one sheet.
  css =
    (await readFile(
      new URL('../../../../../design/browser-ui/generated/static.css', import.meta.url),
      'utf8',
    )) + (await readFile(new URL('pages.css', assets), 'utf8'));
  page_html = (await readFile(new URL('digest.html', assets), 'utf8')).replace(
    '<script type="module" src="/sdk/shell.js"></script>',
    '<script type="module" src="/harness.js"></script>',
  );
});
test.afterAll(async () => root && rm(root, { recursive: true, force: true }));

async function open(page: Page, query = '', size = { width: 1280, height: 900 }) {
  await page.setViewportSize(size);
  await page.route(`${origin}/**`, (route) => {
    const url = new URL(route.request().url());
    if (url.pathname === '/harness.js')
      return route.fulfill({ contentType: 'text/javascript', body: harness });
    if (url.pathname === '/sdk/pages.css')
      return route.fulfill({ contentType: 'text/css', body: css });
    return route.fulfill({ contentType: 'text/html', body: page_html });
  });
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(String(error)));
  await page.goto(`${origin}/digest/${query}`);
  return errors;
}

test('the sample report paints every section once, in order, with the sample note', async ({
  page,
}) => {
  const errors = await open(page);
  await expect(page.locator('#digest-sections .digest-section')).toHaveCount(10);
  const headings = await page.locator('#digest-sections h2').allTextContents();
  expect(headings).toEqual([
    'Runway',
    'Who burns: tokens over time',
    'Who burns: last 24h by seat',
    'Idle, stalled or gate-bound',
    'Throughput',
    'Gates and queue',
    'Cost per merged PR',
    'Interaction volume: who talks to whom',
    'Machine',
    'Method and caveats',
  ]);
  // A question the usage read cannot answer says what is missing instead of drawing a zero.
  for (const id of ['throughput', 'gates', 'cost', 'machine']) {
    await expect(page.locator(`#digest-${id} .usage-note`)).toContainText(
      'Not in the usage read yet:',
    );
    await expect(page.locator(`#digest-${id} .tmt-ui-chart`)).toHaveCount(0);
  }
  await expect(page.locator('#usage-note')).toContainText('Sample data');
  expect(errors).toEqual([]);
});

test('the share toggle swaps the stacked chart for a 100 % one and reads shares', async ({
  page,
}) => {
  await open(page);
  const chart = page.locator('#digest-tokens .tmt-ui-chart');
  await expect(chart.locator('.tmt-ui-chart-title')).toHaveText('Tokens by seat');
  await page.getByRole('button', { name: 'Share of total (100%)' }).click();
  await expect(chart.locator('.tmt-ui-chart-title')).toHaveText('Share of tokens by seat');
  await expect(chart.locator('.tmt-ui-chart-readout')).toContainText('%');
  await expect(page.getByRole('button', { name: 'Share of total (100%)' })).toHaveAttribute(
    'aria-pressed',
    'true',
  );
});

test('the plot reads each hour from the keyboard and names every series in its legend', async ({
  page,
}) => {
  await open(page);
  const plot = page.locator('#digest-tokens .tmt-ui-chart-plot');
  const readout = page.locator('#digest-tokens .tmt-ui-chart-readout');
  await plot.focus();
  await page.keyboard.press('Home');
  const first = await readout.textContent();
  await page.keyboard.press('ArrowRight');
  expect(await readout.textContent()).not.toBe(first);
  await page.keyboard.press('End');
  expect(await readout.textContent()).toMatch(/^\d\d:00:/);
  await expect(page.locator('#digest-tokens .tmt-ui-chart-legend-item').first()).toContainText(
    'core-1',
  );
  // The same numbers are in a table for assistive technology.
  expect(await page.locator('#digest-tokens .tmt-ui-chart-data tbody tr').count()).toBeGreaterThan(
    5,
  );
});

test('the matrix keeps self blank, a known none as a dot and counts only talk and reply', async ({
  page,
}) => {
  await open(page);
  const rows = page.locator('#digest-exchange tbody tr');
  expect(await rows.count()).toBeGreaterThan(3);
  await expect(rows.first().locator('td.tmt-ui-chart-cell-self')).toHaveText('');
  const text = await page.locator('#digest-exchange tbody').innerText();
  expect(text).toContain('·');
  expect(text).not.toContain('99');
});

test('only the 24 hour report has an activity view; 7d and 30d say nothing about hours', async ({
  page,
}) => {
  await open(page);
  await expect(page.locator('#digest-activity .tmt-ui-chart-strip-row')).toHaveCount(6);
  await page.getByRole('tab', { name: '7d' }).click();
  await expect(page.locator('#digest-tokens')).toBeVisible();
  await expect(page.locator('#digest-activity')).toHaveCount(0);
  await expect(page.locator('#digest-ranking h2')).toHaveText('Who burns: last 7d by seat');
});

test('the allowance shows the latest weekly window and the history chart when it is served', async ({
  page,
}) => {
  await open(page);
  const table = page.locator('#digest-allowance table.usage-table');
  await expect(table.locator('tbody tr')).toHaveCount(2);
  await expect(table.locator('tbody tr').first()).toContainText('Claude');
  await expect(page.locator('#digest-allowance .tmt-ui-chart')).toHaveCount(2);
});

for (const width of [390, 768, 1440]) {
  test(`one scroll at ${width}px: nothing makes the page scroll sideways`, async ({ page }) => {
    await open(page, '', { width, height: 900 });
    await expect(page.locator('#digest-sections .digest-section')).toHaveCount(10);
    await page.waitForTimeout(200);
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
    );
    expect(overflow).toBeLessThanOrEqual(0);
  });
}

for (const scheme of ['light', 'dark'] as const) {
  test(`${scheme} theme: chart text and cells stay readable`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: scheme });
    await open(page);
    const ratio = await page.evaluate(() => {
      // A mixed colour computes as `color(srgb r g b)` with 0..1 channels; others as `rgb()`.
      const parse = (value: string) => {
        const channels = value
          .match(/[\d.]+/g)!
          .slice(0, 3)
          .map(Number);
        return value.startsWith('color(') ? channels.map((c) => c * 255) : channels;
      };
      const lum = ([r, g, b]: number[]) => {
        const lin = (c: number) =>
          (c /= 255) <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
        return 0.2126 * lin(r!) + 0.7152 * lin(g!) + 0.0722 * lin(b!);
      };
      const worst: number[] = [];
      const bad: string[] = [];
      for (const cell of document.querySelectorAll('#digest-exchange td.tmt-ui-chart-cell')) {
        if (!cell.textContent) continue;
        const style = getComputedStyle(cell);
        const [hi, lo] = [lum(parse(style.color)), lum(parse(style.backgroundColor))].sort(
          (a, b) => b - a,
        );
        worst.push((hi! + 0.05) / (lo! + 0.05));
        if ((hi! + 0.05) / (lo! + 0.05) < 4.5)
          bad.push(`${cell.className}|${cell.textContent}|${style.color}|${style.backgroundColor}`);
      }
      return { min: Math.min(...worst), sample: bad.slice(0, 3) };
    });
    expect(ratio.min, JSON.stringify(ratio.sample)).toBeGreaterThanOrEqual(4.5);
  });
}

test('the product source keeps the honest empty state and draws no chart', async ({ page }) => {
  const errors = await open(page, '?product');
  await expect(page.locator('#usage-note')).toContainText('not in the browser yet');
  await expect(page.locator('#usage-note code')).toHaveText('tmt ops ui');
  await expect(page.locator('#digest-sections .tmt-ui-chart')).toHaveCount(0);
  await expect(page.locator('#digest-sections .digest-section')).toHaveCount(0);
  expect(errors).toEqual([]);
});
