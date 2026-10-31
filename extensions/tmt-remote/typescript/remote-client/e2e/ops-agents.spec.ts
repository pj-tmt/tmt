import { expect, test, type Page } from '@playwright/test';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { build } from 'vite';
import type { OpsAgent, OpsBoard } from '../src/ops-source.js';

const origin = 'http://ops.test';
let harness = '';
let root = '';

test.beforeAll(async () => {
  root = await mkdtemp(path.join(tmpdir(), 'ops-agents-'));
  await build({
    root: new URL('..', import.meta.url).pathname,
    logLevel: 'silent',
    build: {
      outDir: root,
      emptyOutDir: true,
      minify: false,
      lib: {
        entry: new URL('./ops-agents-harness.ts', import.meta.url).pathname,
        formats: ['es'],
        fileName: () => 'harness.js',
      },
      rollupOptions: { output: { inlineDynamicImports: true } },
    },
  });
  harness = await readFile(path.join(root, 'harness.js'), 'utf8');
});
test.afterAll(async () => {
  if (root) await rm(root, { recursive: true, force: true });
});

const agent = (name: string, squad: string, extra: Partial<OpsAgent> = {}): OpsAgent => ({
  id: `00000000-0000-4000-8000-${[...name]
    .map((letter) => letter.charCodeAt(0).toString(16))
    .join('')
    .slice(0, 12)
    .padEnd(12, '0')}`,
  name,
  squad,
  role: 'member',
  state: 'working',
  task: `${name} task`,
  model: 'model-a',
  waitingOnYou: 0,
  ...extra,
});
const board = (agents: OpsAgent[]): OpsBoard => ({
  agents,
  squads: [{ name: 'alpha' }, { name: 'beta' }],
  schedules: [],
  truncated: false,
});

async function open(page: Page): Promise<void> {
  await page.route(`${origin}/**`, (route) =>
    new URL(route.request().url()).pathname === '/harness.js'
      ? route.fulfill({ contentType: 'text/javascript', body: harness })
      : route.fulfill({
          contentType: 'text/html',
          body: '<!doctype html><meta charset="utf-8"><body><script type="module" src="/harness.js"></script></body>',
        }),
  );
  await page.goto(`${origin}/`);
  await page.waitForFunction(() => typeof window.opsUpdate === 'function');
  await expect(page.locator('.tmt-ui-page-subtitle')).toHaveText(
    'A clear view of every squad and agent.',
  );
  await expect(page.locator('h1')).toHaveCount(0);
}
const update = (page: Page, next: OpsBoard) =>
  page.evaluate((value) => window.opsUpdate(value), next);
/** Tags every row and cell node so a later read can tell a kept node from a rebuilt one. */
const tag = (page: Page) =>
  page.evaluate(() => {
    let index = 0;
    for (const node of document.querySelectorAll<HTMLElement>('tbody tr, tbody td'))
      (node as unknown as { opsTag: number }).opsTag = ++index;
  });
/** Per row (by the agent's name): whether the row and each of its cells are the tagged nodes. */
const kept = (page: Page) =>
  page.evaluate(() =>
    Object.fromEntries(
      [...document.querySelectorAll<HTMLTableRowElement>('tbody tr:not([class])')].map((row) => [
        row.querySelector('button')!.textContent,
        [row, ...row.cells].map(
          (node) => (node as unknown as { opsTag?: number }).opsTag !== undefined,
        ),
      ]),
    ),
  );

test('a poll keeps unchanged rows and cells, and rebuilds only the changed cell', async ({
  page,
}) => {
  await open(page);
  const first = [agent('ada', 'alpha'), agent('bo', 'alpha'), agent('cy', 'beta')];
  await update(page, board(first));
  await tag(page);
  await page.getByRole('button', { name: 'bo' }).focus();
  // Only cy's task changes; a new agent joins alpha.
  await update(
    page,
    board([first[0]!, first[1]!, agent('cy', 'beta', { task: 'new work' }), agent('di', 'alpha')]),
  );
  const after = await kept(page);
  expect(after.ada).toEqual([true, true, true, true, true, true]);
  expect(after.bo).toEqual([true, true, true, true, true, true]);
  // The work cell is rebuilt; the row and its other cells stay.
  expect(after.cy).toEqual([true, true, true, false, true, true]);
  expect(after.di).toEqual([false, false, false, false, false, false]);
  await expect(page.getByRole('cell', { name: /new work/ })).toBeVisible();
  // Focus stays on the button the reader was on.
  expect(await page.evaluate(() => document.activeElement?.textContent)).toBe('bo');
  // Rows follow the board's order, keyed by squad and name.
  expect(Object.keys(after)).toEqual(['ada', 'bo', 'di', 'cy']);
});

test('removing and reordering rows moves the kept nodes instead of rebuilding them', async ({
  page,
}) => {
  await open(page);
  await update(page, board([agent('ada', 'alpha'), agent('bo', 'alpha'), agent('cy', 'beta')]));
  await tag(page);
  await update(page, board([agent('cy', 'beta'), agent('bo', 'alpha')]));
  const after = await kept(page);
  expect(Object.keys(after)).toEqual(['bo', 'cy']);
  expect(after.bo!.every(Boolean) && after.cy!.every(Boolean)).toBe(true);
});

test('selecting a row changes it in place and keeps focus on the pressed button', async ({
  page,
}) => {
  await open(page);
  await update(page, board([agent('ada', 'alpha'), agent('bo', 'alpha')]));
  await tag(page);
  await page.getByRole('button', { name: 'bo' }).click();
  expect(await page.evaluate(() => document.activeElement?.textContent)).toBe('bo');
  await expect(page.getByRole('button', { name: 'bo' })).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByRole('button', { name: 'ada' })).toHaveAttribute('aria-pressed', 'false');
  const after = await kept(page);
  expect(after.ada!.every(Boolean) && after.bo!.every(Boolean)).toBe(true);
});

test('search filters after a pause, at once on Enter or when cleared, and matches the undebounced result', async ({
  page,
}) => {
  await open(page);
  await update(page, board([agent('ada', 'alpha'), agent('bo', 'alpha'), agent('cy', 'beta')]));
  const search = page.getByRole('searchbox', { name: 'Filter agents' });
  const names = () =>
    page.evaluate(() =>
      [...document.querySelectorAll('tbody tr:not([class]) button')].map(
        (node) => node.textContent,
      ),
    );
  await search.fill('cy');
  // Still unfiltered right after typing.
  expect(await names()).toEqual(['ada', 'bo', 'cy']);
  await expect.poll(names).toEqual(['cy']);
  await search.fill('bo');
  await search.press('Enter');
  expect(await names()).toEqual(['bo']);
  await search.fill('');
  expect(await names()).toEqual(['ada', 'bo', 'cy']);
  await search.fill('nobody');
  await search.press('Enter');
  await expect(page.locator('.ops-empty').first()).toHaveText('No agents match.');
});
