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
let shell_harness = '';
let shell_html = '';

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
      rollupOptions: { output: { inlineDynamicImports: true } },
    },
  });
  harness = await readFile(path.join(root, 'harness.js'), 'utf8');
  // The whole web app with the sample source swapped in for the product one.
  const sample = new URL('./digest-sample-source.ts', import.meta.url).pathname;
  await build({
    root: new URL('..', import.meta.url).pathname,
    logLevel: 'silent',
    plugins: [
      {
        name: 'digest-sample-source',
        enforce: 'pre',
        resolveId(source, importer) {
          if (
            source === './digest-source.js' &&
            /\/src\/(digest-page|shell-app)\.ts$/.test(importer ?? '')
          )
            return sample;
        },
      },
    ],
    build: {
      outDir: root,
      emptyOutDir: false,
      minify: false,
      lib: {
        entry: new URL('./shell-harness.ts', import.meta.url).pathname,
        formats: ['es'],
        fileName: () => 'shell-harness.js',
      },
      rollupOptions: { output: { inlineDynamicImports: true } },
    },
  });
  shell_harness = await readFile(path.join(root, 'shell-harness.js'), 'utf8');
  shell_html = (await readFile(new URL('shell.html', assets), 'utf8')).replace(
    '<script type="module" src="/sdk/shell.js"></script>',
    '<script type="module" src="/shell-harness.js"></script>',
  );
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
    if (url.pathname === '/shell-harness.js')
      return route.fulfill({ contentType: 'text/javascript', body: shell_harness });
    if (url.pathname === '/sdk/pages.css')
      return route.fulfill({ contentType: 'text/css', body: css });
    return route.fulfill({ contentType: 'text/html', body: page_html });
  });
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(String(error)));
  await page.goto(`${origin}/digest/${query}`);
  return errors;
}

async function openShell(page: Page, size: { width: number; height: number }, query = '') {
  await page.setViewportSize(size);
  await page.route(`${origin}/**`, (route) => {
    const url = new URL(route.request().url());
    if (url.pathname === '/shell-harness.js')
      return route.fulfill({ contentType: 'text/javascript', body: shell_harness });
    if (url.pathname === '/sdk/pages.css')
      return route.fulfill({ contentType: 'text/css', body: css });
    if (url.pathname.endsWith('.js') || url.pathname.endsWith('.css'))
      return route.fulfill({ status: 404, body: '' });
    return route.fulfill({ contentType: 'text/html', body: shell_html });
  });
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(String(error)));
  await page.goto(`${origin}/digest/${query}`);
  await expect(page.locator('#digest-sections .digest-card').first()).toBeVisible();
  return errors;
}

for (const [name, size] of [
  ['1440', { width: 1440, height: 900 }],
  ['390', { width: 390, height: 844 }],
] as const) {
  test(`in the app shell at ${name}px the page has the rail, the header and the shell's width`, async ({
    page,
  }) => {
    const errors = await openShell(page, size);
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('Digest');
    // The rail is a drawer on a phone; it is part of the page from tablet width up.
    if (name === '1440')
      await expect(page.getByRole('navigation', { name: 'TMT', exact: true })).toBeVisible();
    const width = await page.locator('#digest-sections').evaluate((node) => node.clientWidth);
    if (name === '1440') {
      expect(width).toBeGreaterThan(1000);
      // Two provider cards side by side; the token chart beside the ranking; the matrix beside
      // the data coverage.
      const beside = async (left: string, right: string) => {
        const a = (await page.locator(left).boundingBox())!;
        const b = (await page.locator(right).boundingBox())!;
        expect(b.x, `${right} beside ${left}`).toBeGreaterThan(a.x + a.width - 1);
        expect(Math.abs(b.y - a.y), `${right} on the same row`).toBeLessThan(2);
      };
      await beside('#digest-provider-codex', '#digest-provider-claude');
      await beside('#digest-tokens .digest-token-chart', '#digest-ranking');
      const tokens = (await page.locator('#digest-tokens').boundingBox())!;
      const messages = (await page.locator('#digest-exchange').boundingBox())!;
      expect(messages.y).toBeGreaterThanOrEqual(tokens.y + tokens.height);
      expect(Math.abs(messages.width - tokens.width)).toBeLessThan(1);
    } else {
      const head = (await page.locator('.digest-head').boundingBox())!;
      const title = (await page.locator('.digest-head > div').first().boundingBox())!;
      expect(Math.abs(title.width - head.width)).toBeLessThan(1);
      // The next provider peeks into the phone's horizontal snap carousel.
      const first = (await page.locator('#digest-provider-codex').boundingBox())!;
      const next = (await page.locator('#digest-provider-claude').boundingBox())!;
      const viewport = (await page.locator('.digest-provider-groups').boundingBox())!;
      expect(Math.abs(first.y - next.y)).toBeLessThan(1);
      expect(next.x).toBeLessThan(viewport.x + viewport.width);
      expect(next.x + next.width).toBeGreaterThan(viewport.x + viewport.width);
    }
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
    );
    expect(overflow).toBeLessThanOrEqual(0);
    if (name === '390') {
      await page.getByRole('tab', { name: '30d' }).click();
      await expect(page.locator('#digest-tokens .digest-total-label')).toContainText('30d');
      const fits = await page.evaluate(
        () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
      );
      expect(fits).toBeLessThanOrEqual(0);
      await page.getByRole('tab', { name: '24h' }).click();
      await expect(page.locator('#digest-tokens .digest-total-label')).toContainText('24h');
      await expect(page.locator('#digest-activity')).toHaveCount(0);
    }
    const dir = process.env.DIGEST_CAPTURE_DIR;
    if (dir) {
      await page.waitForTimeout(300);
      await page.screenshot({
        path: path.join(dir, `digest-shell-${name}-light.png`),
        fullPage: true,
      });
    }
    expect(errors).toEqual([]);
  });
}

test('hovering a large-fleet plot names the six leading seats and neutral Other', async ({
  page,
}) => {
  await open(page, '?seats=24');
  const plot = page.locator('#digest-tokens .tmt-ui-chart-plot');
  const tip = page.locator('#digest-tokens .tmt-ui-chart-tip');
  await expect(tip).toBeHidden();
  const box = (await plot.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await expect(tip).toBeVisible();
  await expect(tip.locator('.tmt-ui-chart-tip-row')).toHaveCount(7);
  await expect(tip).toContainText('Other');
  const fills = await tip
    .locator('.tmt-ui-chart-swatch')
    .evaluateAll((nodes) => nodes.map((node) => getComputedStyle(node).backgroundColor));
  expect(new Set(fills).size).toBe(7);
  await page.mouse.move(box.x - 40, box.y - 40);
  await expect(tip).toBeHidden();
});

test('the sample report paints every card once, in the reference order, with the sample badge', async ({
  page,
}) => {
  const errors = await open(page);
  await expect(page.locator('#digest-sections .digest-card')).toHaveCount(4);
  const headings = await page.locator('#digest-sections h2').allTextContents();
  expect(headings).toEqual(['Allowance', 'Token usage', 'Messages between seats', 'Data coverage']);
  await expect(page.locator('#usage-badge')).toHaveText('Sample data');
  await expect(page.locator('#usage-note')).toContainText('Usage preview');
  // Questions the usage read cannot answer: one row each with a badge, never a zero or a chart.
  await expect(page.locator('#digest-missing .digest-gap dt')).toHaveText([
    'Throughput',
    'Gates and queue',
    'Cost per merged PR',
    'Machine load',
  ]);
  await expect(page.locator('#digest-missing .digest-gap dd')).toHaveText([
    'Not connected',
    'Not connected',
    'Not connected',
    'Not connected',
  ]);
  await expect(page.locator('#digest-missing .tmt-ui-chart')).toHaveCount(0);
  // The method sits collapsed at the bottom of the coverage card.
  const method = page.locator('#digest-method');
  await expect(method).not.toHaveAttribute('open', '');
  await method.locator('summary').click();
  await expect(method.locator('p').first()).toBeVisible();
  // The legend rows use the same series hues as the chart, beside it inside one card.
  const dots = page.locator('#digest-ranking .tmt-ui-status-tile-dot');
  await expect(dots).toHaveCount(6);
  expect(
    new Set(
      await dots.evaluateAll((nodes) => nodes.map((n) => getComputedStyle(n).backgroundColor)),
    ).size,
  ).toBe(6);
  expect(errors).toEqual([]);
});

test('the Absolute/Share control swaps the stacked chart for a 100 % one and reads shares', async ({
  page,
}) => {
  await open(page);
  const chart = page.locator('#digest-tokens .tmt-ui-chart');
  await expect(chart.locator('.tmt-ui-chart-title')).toHaveText('Tokens by seat');
  await expect(page.getByRole('tab', { name: 'Absolute' })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  await page.getByRole('tab', { name: 'Share' }).click();
  await expect(chart.locator('.tmt-ui-chart-title')).toHaveText('Share of tokens by seat');
  await expect(chart.locator('.tmt-ui-chart-readout')).toContainText('%');
  await expect(page.getByRole('tab', { name: 'Share' })).toHaveAttribute('aria-selected', 'true');
  await expect(page.getByRole('tab', { name: 'Absolute' })).toHaveAttribute(
    'aria-selected',
    'false',
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

test('the matrix marks self with a dash and a known none as zero and counts only talk and reply', async ({
  page,
}) => {
  await open(page);
  const rows = page.locator('#digest-exchange tbody tr');
  expect(await rows.count()).toBeGreaterThan(3);
  await expect(rows.first().locator('td[data-self]')).toHaveText('—');
  const text = await page.locator('#digest-exchange tbody').innerText();
  expect(await rows.locator('td').allTextContents()).toContain('0');
  expect(text).not.toContain('99');
});

test('Lines uses the same seat colours, preserves missing runs and switches ranges without Activity', async ({
  page,
}) => {
  await open(page);
  const colours = await page
    .locator('#digest-ranking .tmt-ui-status-tile-dot')
    .evaluateAll((nodes) => nodes.map((node) => getComputedStyle(node).backgroundColor));
  await page.getByRole('tab', { name: 'Lines', exact: true }).click();
  expect(
    await page
      .locator('#digest-ranking .tmt-ui-status-tile-dot')
      .evaluateAll((nodes) => nodes.map((node) => getComputedStyle(node).backgroundColor)),
  ).toEqual(colours);
  const plot = page.locator('#digest-tokens .tmt-ui-chart-plot');
  await plot.focus();
  await page.keyboard.press('Home');
  await expect(page.locator('#digest-tokens .tmt-ui-chart-readout')).toContainText('—');
  for (const range of ['7d', '30d', '24h']) {
    await page.getByRole('tab', { name: range, exact: true }).click();
    await expect(page.getByRole('tab', { name: 'Lines', exact: true })).toHaveAttribute(
      'aria-selected',
      'true',
    );
    await expect(page.locator('#digest-tokens .digest-total-label')).toContainText(range);
    await expect(page.locator('#digest-tokens .tmt-ui-chart-line')).toHaveCount(6);
  }
  await page.getByRole('tab', { name: 'Share', exact: true }).click();
  await expect(
    page.locator('#digest-tokens .tmt-ui-chart-tick').filter({ hasText: '%' }).first(),
  ).toBeVisible();
  await expect(page.locator('#digest-activity')).toHaveCount(0);
});

test('allowance has a current OpenAI window, distinct folded histories, and an explicit Claude row', async ({
  page,
}) => {
  await open(page);
  const openai = page.locator('#digest-provider-codex');
  const primary = openai.locator(':scope > .digest-provider');
  await expect(primary.locator('.digest-provider-label strong')).toHaveText(['OpenAI', '55% left']);
  await expect(primary.locator('.digest-provider-symbol')).toHaveAttribute('aria-hidden', 'true');
  await expect(primary.locator('.digest-window-observation')).toBeVisible();
  await expect(primary).toContainText('Observed 10m ago');
  await expect(primary).toContainText('Reported by core-1, feat-1');
  const history = primary.locator('details');
  await expect(history).not.toHaveAttribute('open');
  await history.locator('summary').click();
  await expect(primary.locator('.tmt-ui-status-tile-label')).toHaveText([
    'Elapsed',
    'Pace',
    'Resets in',
  ]);
  await expect(primary.locator('.tmt-ui-chart')).toBeVisible();
  const meter = (await primary.locator('.tmt-ui-meter').boundingBox())!;
  const fill = (await primary.locator('.tmt-ui-meter-fill').boundingBox())!;
  expect(Math.abs(fill.width / meter.width - 0.55)).toBeLessThan(0.03);
  const other = openai.locator('.digest-other-windows');
  await expect(other).not.toHaveAttribute('open');
  await expect(other.locator(':scope > summary')).toHaveText(
    'Other OpenAI windows seen this week (2)',
  );
  await other.locator(':scope > summary').focus();
  await page.keyboard.press('Enter');
  await expect(other).toHaveAttribute('open', '');
  const windows = other.locator('.digest-provider');
  await expect(windows).toHaveCount(2);
  await expect(windows.first()).toContainText('Observed 20m ago');
  await expect(windows.first()).toContainText('Reported by remote-1, tmt-lead, ux-1');
  for (const window of await windows.all()) {
    await expect(window).toContainText('Resets');
    await window.locator('summary').click();
    await expect(window.locator('.tmt-ui-chart')).toBeVisible();
  }
  const readings = await openai.locator('.tmt-ui-chart-readout').allTextContents();
  expect(new Set(readings).size).toBe(3);
  const ids = await page.locator('[id]').evaluateAll((nodes) => nodes.map((node) => node.id));
  expect(new Set(ids).size).toBe(ids.length);
  await expect(page.locator('#digest-provider-claude')).toContainText('Claude○ not reported');
  await expect(page.locator('#digest-provider-claude')).toContainText(
    'Claude Code does not report its allowance to TMT.',
  );
  await expect(page.locator('#digest-provider-claude .tmt-ui-meter')).toHaveCount(0);
});

test('the same allowance windows and open disclosures survive every usage range', async ({
  page,
}) => {
  await open(page);
  const allowance = page.locator('#digest-allowance');
  await allowance.locator('.digest-other-windows > summary').click();
  const text = await allowance.innerText();
  const tokens = new Set<string>();
  for (const range of ['7d', '30d', '24h']) {
    await page.getByRole('tab', { name: range, exact: true }).click();
    await expect(allowance.locator('.digest-other-windows')).toHaveAttribute('open', '');
    expect(await allowance.innerText()).toBe(text);
    await expect(page.locator('.digest-total-label')).toContainText(range);
    tokens.add((await page.locator('.digest-total').textContent())!);
  }
  expect(tokens.size).toBe(3);
});

test('a team of 24 lists the top 6, one Other row, a single partial note and Show all', async ({
  page,
}) => {
  const errors = await open(page, '?seats=24', { width: 1440, height: 900 });
  const ranking = page.locator('#digest-ranking');
  const names = ranking.locator('tbody th');
  await expect(names).toHaveCount(7);
  await expect(names.last()).toHaveText('Other (18 seats)');
  await expect(ranking.locator('.usage-note')).toHaveText([
    '* Partial: some of this range has no data for the seat.',
  ]);
  await expect(ranking.getByText('(partial)')).toHaveCount(0);
  // Every name is one line: no row is taller than the shortest.
  const heights = await ranking
    .locator('tbody tr')
    .evaluateAll((rows) => rows.map((row) => row.getBoundingClientRect().height));
  expect(Math.max(...heights) - Math.min(...heights)).toBeLessThan(1);
  const toggle = ranking.getByRole('button', { name: 'Show all 24 seats' });
  await expect(toggle).toHaveAttribute('aria-expanded', 'false');
  await toggle.click();
  await expect(names).toHaveCount(24);
  await expect(ranking.getByText(/^Other \(/)).toHaveCount(0);
  await ranking.getByRole('button', { name: 'Show top 6' }).click();
  await expect(names).toHaveCount(7);
  // Cards keep their own height: the ranking does not stretch to the token chart's.
  const tokens = (await page.locator('#digest-tokens').boundingBox())!;
  const listed = (await ranking.boundingBox())!;
  expect(listed.y).toBeGreaterThan(tokens.y);
  expect(listed.x).toBeGreaterThan(tokens.x);
  expect(Math.abs(tokens.height - listed.height)).toBeGreaterThan(1);
  // The complete Messages axes use the same usage order as the expanded seat table.
  await ranking.getByRole('button', { name: 'Show all 24 seats' }).click();
  const ranked = (await names.allTextContents()).map((name) => name.replace(/\*$/, ''));
  await expect(page.locator('#digest-exchange tbody th')).toHaveText(ranked);
  expect(errors).toEqual([]);
});

test('one provider shares the allowance card with its heading and retains its history', async ({
  page,
}) => {
  const errors = await open(page, '?one-provider', { width: 1440, height: 900 });
  const provider = page.locator('#digest-provider-codex');
  await expect(page.locator('#digest-allowance .digest-provider')).toHaveCount(1);
  await expect(page.locator('#digest-provider-claude')).toContainText('○ not reported');
  await expect(provider.locator('.tmt-ui-chart')).toBeHidden();
  await provider.locator('summary').click();
  await expect(provider.locator('.tmt-ui-chart')).toBeVisible();
  expect(errors).toEqual([]);
});

/** Acceptance captures: the shell at 1440 and 390, light and dark, sample and a 24-seat team. */
for (const [label, query] of [
  ['sample', ''],
  ['24-seats', '?seats=24'],
  ['24-seats-one-provider', '?seats=24&one-provider'],
] as const)
  for (const scheme of ['light', 'dark'] as const)
    for (const width of [1440, 390])
      test(`capture ${label} ${scheme} ${width}`, async ({ page }) => {
        const dir = process.env.DIGEST_CAPTURE_DIR;
        test.skip(!dir, 'captures only when DIGEST_CAPTURE_DIR is set');
        await page.emulateMedia({ colorScheme: scheme });
        const errors = await openShell(page, { width, height: 900 }, query);
        await page.waitForTimeout(300);
        await page.screenshot({
          path: path.join(dir!, `digest-${label}-${scheme}-${width}.png`),
          fullPage: true,
        });
        expect(errors).toEqual([]);
      });

for (const width of [320, 390, 768, 1440]) {
  test(`one scroll at ${width}px: nothing makes the page scroll sideways`, async ({ page }) => {
    await open(page, '', { width, height: 900 });
    await expect(page.locator('#digest-sections .digest-card')).toHaveCount(4);
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
      for (const cell of document.querySelectorAll('#digest-exchange td')) {
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

test('the product source reads the owner reads and, without an owner session, says so and draws nothing', async ({
  page,
}) => {
  const errors = await open(page, '?product');
  await expect(page.locator('#usage-note')).toHaveText(
    'This browser has no owner session for your machine. Pair it from Remote to see your usage.',
  );
  await expect(page.locator('#usage-badge')).toBeHidden();
  await expect(page.locator('#digest-sections .tmt-ui-chart')).toHaveCount(0);
  await expect(page.locator('#digest-sections .digest-card')).toHaveCount(0);
  await expect(page.locator('body')).not.toContainText('Sample data');
  expect(errors).toEqual([]);
});

test('the report refreshes once a minute and repaints only when it changed', async ({ page }) => {
  await page.clock.install();
  const errors = await open(page, '?refresh&seats=24', { width: 390, height: 900 });
  const reads = () => page.evaluate(() => window.digestReads);
  await expect(
    page.locator(
      '#digest-provider-codex > .digest-provider .digest-provider-label > strong:last-child',
    ),
  ).toHaveText('55% left');
  const matrix = page.locator('#digest-exchange').getByRole('region', { name: /^Messages, last / });
  await matrix.evaluate((node) => {
    node.scrollLeft = 100;
  });
  const matrixPosition = await matrix.evaluate((node) => node.scrollLeft);
  expect(matrixPosition).toBeGreaterThan(0);
  expect(await reads()).toBe(1);
  // A reader's place: the Share choice and a marker on the token card must survive a refresh
  // that found nothing new.
  await page.getByRole('tab', { name: 'Share' }).click();
  await page.locator('#digest-tokens').evaluate((node) => node.setAttribute('data-kept', 'yes'));
  await page.clock.fastForward(60_000);
  await expect.poll(reads).toBe(2);
  await expect(page.locator('#digest-tokens')).toHaveAttribute('data-kept', 'yes');
  await expect(
    page.locator('#digest-provider-codex > .digest-provider .digest-window-observation'),
  ).toHaveText('Observed 12m ago');
  expect(await matrix.evaluate((node) => node.scrollLeft)).toBe(matrixPosition);
  // The third read has less allowance left: the cards repaint, and Share is still the choice.
  await page.clock.fastForward(60_000);
  await expect.poll(reads).toBe(3);
  await expect(
    page.locator(
      '#digest-provider-codex > .digest-provider .digest-provider-label > strong:last-child',
    ),
  ).toHaveText('10% left');
  await expect(page.getByRole('tab', { name: 'Share' })).toHaveAttribute('aria-selected', 'true');
  await expect(page.locator('#digest-tokens .tmt-ui-chart-title')).toHaveText(
    'Share of tokens by seat',
  );
  // A read that fails after a good one keeps the last good report and says why in a small note;
  // the panel is not emptied.
  await page.clock.fastForward(60_000);
  await expect.poll(reads).toBe(4);
  await expect(
    page.locator(
      '#digest-provider-codex > .digest-provider .digest-provider-label > strong:last-child',
    ),
  ).toHaveText('10% left');
  await expect(page.locator('#digest-tokens')).toBeVisible();
  await expect(page.locator('#usage-note')).toContainText('Your machine did not answer');
  await expect(page.locator('#usage-note')).toContainText('Showing the last report, read at');
  await page
    .getByRole('tablist', { name: 'Seat order' })
    .getByRole('tab', { name: 'Name', exact: true })
    .click();
  await expect(page.locator('#usage-note')).toContainText('Showing the last report, read at');
  // The next good read clears it: the sample's own preview line is back.
  await page.clock.fastForward(120_000);
  await expect.poll(reads).toBe(5);
  await expect(page.locator('#usage-note')).toContainText('Usage preview');
  await expect(page.locator('#usage-note')).not.toContainText('Showing the last report');
  await expect(
    page.locator(
      '#digest-provider-codex > .digest-provider .digest-provider-label > strong:last-child',
    ),
  ).toHaveText('10% left');
  // Leaving the route stops the reads.
  await page.evaluate(() => window.digestRoute.abort());
  await page.clock.fastForward(300_000);
  expect(await reads()).toBe(5);
  expect(errors).toEqual([]);
});

for (const scheme of ['light', 'dark'] as const)
  for (const width of [1440, 390])
    test(`the 24-seat matrix reaches the last column and keeps sender labels at ${width}px ${scheme}`, async ({
      page,
    }) => {
      await page.emulateMedia({ colorScheme: scheme });
      const errors = await openShell(page, { width, height: 900 }, '?seats=24');
      const card = page.locator('#digest-exchange');
      const region = card.getByRole('region', { name: /^Messages, last / });
      await expect(region).toHaveAttribute('tabindex', '0');
      await expect(card.locator('thead th:not(:first-child)')).toHaveCount(24);
      await expect(card.locator('tbody tr')).toHaveCount(24);
      await card.scrollIntoViewIfNeeded();
      const sender = card.locator('tbody th').first();
      const viewport = (await region.boundingBox())!;
      expect(await region.evaluate((node) => node.scrollWidth > node.clientWidth)).toBe(true);
      await expect(card.locator('.digest-messages-edge')).toBeVisible();
      const dir = process.env.DIGEST_CAPTURE_DIR;
      if (dir)
        await card.screenshot({ path: path.join(dir, `matrix-${scheme}-${width}-initial.png`) });
      await region.evaluate((node) => node.focus({ preventScroll: true }));
      await page.keyboard.press('ArrowRight');
      await expect.poll(() => region.evaluate((node) => node.scrollLeft)).toBeGreaterThan(0);
      expect(await region.evaluate((node) => getComputedStyle(node).outlineStyle)).not.toBe('none');
      await region.evaluate((node) => {
        node.scrollLeft = node.scrollWidth;
      });
      const last = (await card.locator('tbody tr').first().locator('td').last().boundingBox())!;
      const pinned = (await sender.boundingBox())!;
      expect(Math.abs(pinned.x - viewport.x)).toBeLessThanOrEqual(3);
      expect(
        await sender.evaluate((node) => getComputedStyle(node.closest('table')!).borderCollapse),
      ).toBe('separate');
      expect(
        await sender.evaluate((node) => Number.parseFloat(getComputedStyle(node).borderRightWidth)),
      ).toBe(0);
      const firstVisibleCell = await sender.evaluate((node) => {
        const edge = node.getBoundingClientRect().right;
        const cell = Array.from(node.parentElement!.querySelectorAll('td')).find(
          (candidate) => candidate.getBoundingClientRect().left >= edge,
        );
        return cell?.getBoundingClientRect().left ?? null;
      });
      expect(firstVisibleCell).not.toBeNull();
      expect(pinned.x + pinned.width).toBeLessThanOrEqual(firstVisibleCell!);
      expect(last.x + last.width).toBeLessThanOrEqual(viewport.x + viewport.width + 1);
      expect(last.x).toBeGreaterThanOrEqual(viewport.x);
      await expect(card.locator('.digest-messages-edge')).toBeHidden();
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
        ),
      ).toBeLessThanOrEqual(0);
      if (dir)
        await card.screenshot({ path: path.join(dir, `matrix-${scheme}-${width}-last.png`) });
      expect(errors).toEqual([]);
    });

test('a fitting matrix has no edge cue and resize updates the overflow cue', async ({ page }) => {
  await open(page, '', { width: 1440, height: 900 });
  const card = page.locator('#digest-exchange');
  const region = card.getByRole('region', { name: /^Messages, last / });
  const edge = card.locator('.digest-messages-edge');
  expect(await region.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
  await expect(edge).toBeHidden();
  await page.setViewportSize({ width: 390, height: 900 });
  expect(await region.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
  await region.evaluate((node) => {
    node.style.maxWidth = '160px';
  });
  await expect
    .poll(() => region.evaluate((node) => node.scrollWidth > node.clientWidth))
    .toBe(true);
  await expect(edge).toBeVisible();
  await region.evaluate((node) => {
    node.style.maxWidth = '';
  });
  await page.setViewportSize({ width: 1440, height: 900 });
  await expect(edge).toBeHidden();
});

test('refresh and range changes retain provider and method disclosures, legend expansion and summary focus', async ({
  page,
}) => {
  await page.clock.install();
  await open(page, '?refresh&seats=24', { width: 1440, height: 900 });
  const provider = page.locator('#digest-provider-codex > .digest-provider > details');
  const method = page.locator('#digest-method');
  await provider.locator('summary').click();
  await method.locator('summary').click();
  await page.getByRole('button', { name: 'Show all 24 seats' }).click();
  await page.getByRole('tab', { name: 'Share' }).click();
  await provider.locator('summary').focus();
  await page.clock.fastForward(60_000);
  await expect.poll(() => page.evaluate(() => window.digestReads)).toBe(2);
  await page.clock.fastForward(60_000);
  await expect.poll(() => page.evaluate(() => window.digestReads)).toBe(3);
  await expect(
    page.locator('#digest-provider-codex > .digest-provider .digest-provider-label'),
  ).toContainText('10% left');
  await expect(provider).toHaveAttribute('open', '');
  await expect(method).toHaveAttribute('open', '');
  await expect(provider.locator('summary')).toBeFocused();
  await expect(page.locator('#digest-ranking tbody tr')).toHaveCount(24);
  await expect(page.getByRole('tab', { name: 'Share' })).toHaveAttribute('aria-selected', 'true');
  // A new range that cannot read must not show the old range's observations.
  await page.getByRole('tab', { name: '7d' }).click();
  await expect(page.locator('#digest-sections .digest-card')).toHaveCount(0);
  await expect(page.locator('#usage-note')).toContainText('Your machine did not answer');
  await page.getByRole('tab', { name: '30d' }).click();
  await expect(provider).toHaveAttribute('open', '');
  await expect(method).toHaveAttribute('open', '');
  await expect(page.locator('#digest-ranking tbody tr')).toHaveCount(24);
  await expect(page.locator('#digest-tokens .digest-total-label')).toContainText('30d');
  await expect(page.getByRole('tabpanel', { name: 'Share', exact: true })).toHaveAttribute(
    'id',
    'digest-tokens-chart',
  );
});

test('stale allowance is named before its disclosure is opened', async ({ page }) => {
  await open(page, '?stale-allowance');
  for (const provider of await page.locator('.digest-provider').all()) {
    await expect(provider.locator('.digest-provider-label')).toContainText('(stale)');
    await expect(provider.locator('details')).not.toHaveAttribute('open');
  }
});

test('unavailable seats make the reported total a lower bound and missing allowance stays explicit', async ({
  page,
}) => {
  await open(page, '?missing-evidence');
  await expect(page.locator('.digest-total-label')).toContainText('lower bound');
  await expect(page.locator('#digest-tokens')).toContainText('Unavailable seats are not included');
  await expect(page.locator('#digest-allowance')).toContainText(
    'Claude Code does not report its allowance to TMT.',
  );
  await expect(page.locator('.digest-provider')).toHaveCount(0);
  await expect(page.locator('.digest-provider-group')).toHaveCount(2);
  await expect(page.locator('#digest-allowance')).not.toContainText('No weekly window');
});

test('range keyboard focus follows the selection immediately and late reports cannot move it back', async ({
  page,
}) => {
  await open(page, '?delayed-range');
  await expect(page.locator('#digest-tokens')).toBeVisible();
  await page.getByRole('tab', { name: '24h' }).focus();
  await page.keyboard.press('ArrowRight');
  await expect(page.getByRole('tab', { name: '7d' })).toBeFocused();
  await expect.poll(() => page.evaluate(() => window.digestPending)).toContain('7d');
  await page.keyboard.press('ArrowRight');
  await expect(page.getByRole('tab', { name: '30d' })).toBeFocused();
  await expect.poll(() => page.evaluate(() => window.digestPending)).toContain('30d');
  await page.evaluate(() => window.digestRelease('30d'));
  await expect(page.locator('.digest-total-label')).toContainText('30d');
  await page.evaluate(() => window.digestRelease('7d'));
  await expect(page.locator('.digest-total-label')).toContainText('30d');
  await expect(page.getByRole('tab', { name: '30d' })).toBeFocused();
});

for (const theme of ['light', 'dark'] as const) {
  test(`Digest panels fit desktop and preserve phone scrolling (${theme})`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: theme });
    await openShell(page, { width: 1440, height: 992 });
    const scroll = page.locator('#digest-exchange .digest-messages-scroll');
    expect(await scroll.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
    const cell = page.locator('#digest-exchange td').first();
    expect(await cell.evaluate((node) => getComputedStyle(node).borderWidth)).toBe('0px');
    expect(await cell.evaluate((node) => getComputedStyle(node).fontFamily)).not.toMatch(/mono/i);
    const areas = page.locator('#digest-token-plot .tmt-ui-chart-area');
    expect(await areas.first().getAttribute('d')).toContain('C');
    expect(Number(await areas.first().evaluate((node) => getComputedStyle(node).fillOpacity))).toBe(
      0.75,
    );
    await expect(page.locator('#digest-token-plot .tmt-ui-chart-cursor')).toHaveAttribute(
      'visibility',
      'hidden',
    );
    await page.setViewportSize({ width: 390, height: 992 });
    await expect(page.locator('html')).toHaveJSProperty('scrollWidth', 390);
    expect(await scroll.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
  });
}

for (const scheme of ['light', 'dark'] as const)
  for (const width of [1440, 390, 320])
    test(`allowance supports N providers and folded windows at ${width}px ${scheme}`, async ({
      page,
    }) => {
      await page.emulateMedia({ colorScheme: scheme });
      const errors = await openShell(page, { width, height: 900 }, '?providers=4');
      const groups = page.locator('.digest-provider-group');
      await expect(groups).toHaveCount(4);
      const boxes = await groups.evaluateAll((nodes) =>
        nodes.map((node) => ({
          width: node.getBoundingClientRect().width,
          x: node.getBoundingClientRect().x,
          y: node.getBoundingClientRect().y,
        })),
      );
      expect(
        Math.max(...boxes.map((box) => box.width)) - Math.min(...boxes.map((box) => box.width)),
      ).toBeLessThan(1);
      await page.locator('.digest-other-windows > summary').click();
      await expect(page.locator('html')).toHaveJSProperty('scrollWidth', width);
      if (process.env.DIGEST_CAPTURE_DIR)
        await page.screenshot({
          fullPage: true,
          path: path.join(process.env.DIGEST_CAPTURE_DIR, `allowance-four-${scheme}-${width}.png`),
        });
      expect(errors).toEqual([]);
    });

test('an allowance-only failure does not substitute selected-range limits and recovers next refresh', async ({
  page,
}) => {
  await page.clock.install();
  await open(page, '?refresh&allowance-failure');
  await expect(page.locator('#digest-provider-codex > .digest-provider')).toBeVisible();
  await page.clock.fastForward(60_000);
  await expect.poll(() => page.evaluate(() => window.digestReads)).toBe(2);
  await expect(page.locator('#digest-tokens')).toBeVisible();
  await expect(page.locator('#digest-allowance')).toContainText('Allowance could not be read');
  await expect(page.locator('#digest-allowance .tmt-ui-meter')).toHaveCount(0);
  await expect(page.locator('#digest-provider-claude')).toContainText('○ not reported');
  await page.clock.fastForward(60_000);
  await expect.poll(() => page.evaluate(() => window.digestReads)).toBe(3);
  await expect(page.locator('#digest-provider-codex > .digest-provider')).toContainText('10% left');
  await expect(page.locator('#digest-allowance')).not.toContainText('Allowance could not be read');
});

test('a window disappears when the owner clock reaches its reset even if its observation is unchanged', async ({
  page,
}) => {
  await page.clock.install();
  await open(page, '?refresh&expiring-window');
  await expect(page.locator('#digest-provider-codex > .digest-provider')).toContainText('55% left');
  await page.clock.fastForward(60_000);
  await expect.poll(() => page.evaluate(() => window.digestReads)).toBe(2);
  await expect(page.locator('#digest-provider-codex')).toContainText('○ not reported');
  await expect(page.locator('#digest-provider-codex .tmt-ui-meter')).toHaveCount(0);
});

test('4781 merges Activity into selected Stacked/Lines and shares seat order with the Messages heatmap', async ({
  page,
}) => {
  const errors = await open(page);
  await expect(page.locator('#digest-activity')).toHaveCount(0);
  const style = page.getByRole('tablist', { name: 'Chart style', exact: true });
  await expect(style.getByRole('tab', { name: 'Stacked', exact: true })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  await style.getByRole('tab', { name: 'Lines', exact: true }).click();
  await expect(style.getByRole('tab', { name: 'Lines', exact: true })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  await expect(page.locator('#digest-tokens .tmt-ui-chart-line')).toHaveCount(6);
  await expect(page.locator('#digest-tokens .tmt-ui-chart-area')).toHaveCount(0);
  const sort = page.getByRole('tablist', { name: 'Seat order', exact: true });
  await sort.getByRole('tab', { name: 'Name', exact: true }).click();
  const legend = (
    await page.locator('#digest-ranking tbody th .digest-seat').allTextContents()
  ).map((name) => name.replace(/\*$/, ''));
  expect(legend).toEqual([...legend].sort((a, b) => a.localeCompare(b)));
  await expect(page.locator('#digest-exchange tbody th')).toHaveText(legend);
  await expect(page.locator('#digest-exchange thead th').filter({ hasNotText: 'From' })).toHaveText(
    legend,
  );
  const positive = await page.locator('#digest-exchange td[data-level]').evaluateAll((cells) =>
    cells.map((cell) => ({
      count: Number(cell.textContent),
      fill: getComputedStyle(cell).backgroundColor,
    })),
  );
  expect(positive.length).toBeGreaterThan(0);
  expect(new Set(positive.map((cell) => cell.fill)).size).toBeGreaterThan(1);
  await expect(page.locator('#digest-exchange td[data-self]')).toHaveCount(6);
  await page.reload();
  await expect(
    page
      .getByRole('tablist', { name: 'Seat order' })
      .getByRole('tab', { name: 'Name', exact: true }),
  ).toHaveAttribute('aria-selected', 'true');
  expect(errors).toEqual([]);
});

for (const failure of ['invalid', 'read', 'write'] as const)
  test(`seat order falls back to Usage when local preference is ${failure}`, async ({ page }) => {
    await page.addInitScript((mode) => {
      if (mode === 'invalid') localStorage.setItem('tmt-digest-seat-order', 'random');
      else
        Object.defineProperty(Storage.prototype, mode === 'read' ? 'getItem' : 'setItem', {
          value() {
            throw new Error('blocked storage');
          },
        });
    }, failure);
    const errors = await open(page);
    const sort = page.getByRole('tablist', { name: 'Seat order', exact: true });
    await expect(sort.getByRole('tab', { name: 'Usage', exact: true })).toHaveAttribute(
      'aria-selected',
      'true',
    );
    if (failure === 'write') {
      await sort.getByRole('tab', { name: 'Name', exact: true }).click();
      await expect(sort.getByRole('tab', { name: 'Usage', exact: true })).toHaveAttribute(
        'aria-selected',
        'true',
      );
    }
    expect(errors).toEqual([]);
  });

for (const width of [1440, 390])
  test(`provider carousel has equal heights, a next-card cue and working navigation at ${width}`, async ({
    page,
  }) => {
    const errors = await open(page, '?providers=4', { width, height: 900 });
    const region = page.getByRole('region', { name: 'Provider allowances' });
    const cards = region.locator('.digest-provider-group');
    await expect(cards).toHaveCount(4);
    const boxes = await cards.evaluateAll((nodes) =>
      nodes.map((node) => node.getBoundingClientRect().toJSON()),
    );
    expect(
      Math.max(...boxes.map((box) => box.height)) - Math.min(...boxes.map((box) => box.height)),
    ).toBeLessThan(1);
    const frame = (await region.boundingBox())!;
    const peeking = boxes[width === 390 ? 1 : 2]!;
    expect(peeking.x).toBeLessThan(frame.x + frame.width);
    expect(peeking.right).toBeGreaterThan(frame.x + frame.width);
    await expect(page.getByRole('button', { name: 'Previous provider' })).toBeDisabled();
    await page.getByRole('button', { name: 'Next provider' }).click();
    await expect.poll(() => region.evaluate((node) => node.scrollLeft)).toBeGreaterThan(0);
    await expect(page.getByRole('button', { name: 'Previous provider' })).toBeEnabled();
    expect(await region.evaluate((node) => getComputedStyle(node).scrollSnapType)).toContain(
      'mandatory',
    );
    expect(errors).toEqual([]);
  });

test('changing seat order during a pending range never restores the preceding range', async ({
  page,
}) => {
  const errors = await open(page, '?delayed-range');
  await expect(page.locator('.digest-total-label')).toContainText('24h');
  await page.getByRole('tab', { name: '7d', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.digestPending)).toContain('7d');
  await expect(page.locator('#digest-tokens')).toHaveCount(0);
  await page
    .getByRole('tablist', { name: 'Seat order' })
    .getByRole('tab', { name: 'Name', exact: true })
    .click();
  await expect(page.locator('#digest-tokens')).toHaveCount(0);
  await page.evaluate(() => window.digestRelease('7d'));
  await expect(page.locator('.digest-total-label')).toContainText('7d');
  await expect(
    page
      .getByRole('tablist', { name: 'Seat order' })
      .getByRole('tab', { name: 'Name', exact: true }),
  ).toHaveAttribute('aria-selected', 'true');
  const names = (await page.locator('#digest-ranking tbody .digest-seat').allTextContents()).map(
    (name) => name.replace(/\*$/, ''),
  );
  expect(names).toEqual([...names].sort((a, b) => a.localeCompare(b)));
  expect(errors).toEqual([]);
});
