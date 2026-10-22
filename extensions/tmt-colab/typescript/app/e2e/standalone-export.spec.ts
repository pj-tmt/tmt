import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { expect, test } from '@playwright/test';
import { standaloneDocument } from '../src/standalone.js';

// The standalone copy of an opted-in page (#1693), built from the real runtime and opened from
// disk the way a person opens a downloaded file: no Colab, no server, no network. The page is
// React + Tailwind; its theme must follow the system colour scheme, and the copy must stay inside
// the renderer frame's no-network policy.
const app = fileURLToPath(new URL('..', import.meta.url));
const tokens = JSON.parse(
  readFileSync(new URL('../../../../../design/tokens/tokens.json', import.meta.url), 'utf8'),
);
const rgb = (hex: string) => {
  const value = parseInt(hex.slice(1, 7), 16);
  return `rgb(${value >> 16}, ${(value >> 8) & 255}, ${value & 255})`;
};
const surface = (key: string, mode: 'light' | 'dark') => rgb(tokens.browser.surface[key][mode]);
const color = (key: string, mode: 'light' | 'dark') => rgb(tokens.browser.color[key][mode]);

const source = `<meta name="tmt-colab-runtime" content="react tailwind">
<main id="app"></main>
<script>
  const { createRoot, html, icons } = window.tmt;
  createRoot(document.getElementById('app')).render(
    html\`<p id="message" class="bg-paper text-text p-4">Offline <\${icons.Check} id="icon" aria-label="ok" /></p>\`,
  );
</script>`;

let directory = '';
let copy = '';
test.beforeAll(() => {
  directory = mkdtempSync(join(tmpdir(), 'tmt-standalone-export-'));
  const built = join(directory, 'runtime');
  execFileSync('./node_modules/.bin/vp', ['build', '--mode', 'runtime', '--outDir', built], {
    cwd: app,
    stdio: 'pipe',
  });
  const document = standaloneDocument(source, {
    script: readFileSync(join(built, 'assets/colab-runtime.js'), 'utf8'),
    style: readFileSync(join(built, 'assets/colab-runtime.css'), 'utf8'),
  });
  expect(document).toBeDefined();
  copy = join(directory, 'page.standalone.html');
  writeFileSync(copy, document!);
});
test.afterAll(() => {
  if (directory) rmSync(directory, { recursive: true, force: true });
});

test('the standalone copy renders the page from disk, offline, and follows the system theme', async ({
  browser,
}) => {
  const context = await browser.newContext({ colorScheme: 'light', offline: true });
  const page = await context.newPage();
  const problems: string[] = [];
  const requests: string[] = [];
  page.on('pageerror', (error) => problems.push(String(error)));
  page.on('console', (message) => {
    if (message.type() === 'error') problems.push(message.text());
  });
  page.on('request', (request) => requests.push(request.url()));
  await page.goto(pathToFileURL(copy).href);

  const paint = (id: string, property: 'backgroundColor' | 'color') =>
    page.evaluate(
      ([target, name]) =>
        (getComputedStyle(document.getElementById(target!)!) as unknown as Record<string, string>)[
          name!
        ],
      [id, property],
    );
  // The page's own script ran against window.tmt, and Tailwind compiled its classes.
  await expect(page.locator('#message')).toContainText('Offline');
  await expect(page.locator('#icon')).toHaveAttribute('role', 'img');
  await expect.poll(() => paint('message', 'backgroundColor')).toBe(surface('paper', 'light'));
  expect(await paint('message', 'color')).toBe(color('text', 'light'));
  expect(await page.evaluate(() => document.documentElement.dataset.theme)).toBe('light');

  // The theme follows the system colour scheme, now and when it changes.
  await page.emulateMedia({ colorScheme: 'dark' });
  await expect.poll(() => paint('message', 'backgroundColor')).toBe(surface('paper', 'dark'));
  expect(await paint('message', 'color')).toBe(color('text', 'dark'));
  expect(await page.evaluate(() => document.documentElement.dataset.theme)).toBe('dark');
  await page.emulateMedia({ colorScheme: 'light' });
  await expect.poll(() => paint('message', 'backgroundColor')).toBe(surface('paper', 'light'));

  // Nothing went wrong while it ran. (The probe below provokes errors of its own.)
  expect(problems).toEqual([]);

  // The renderer frame's policy travels with the copy: nothing leaves, and the copy asks for nothing.
  const violations = await page.evaluate(
    () =>
      new Promise<string[]>((resolve) => {
        const seen: string[] = [];
        document.addEventListener('securitypolicyviolation', (event) =>
          seen.push(event.violatedDirective),
        );
        void fetch('https://example.com/leak').catch(() => undefined);
        const frame = document.createElement('iframe');
        frame.src = 'https://example.com/';
        document.body.append(frame);
        setTimeout(() => resolve(seen), 500);
      }),
  );
  expect(violations.some((name) => name.startsWith('connect-src'))).toBe(true);
  expect(violations.some((name) => name.startsWith('frame-src'))).toBe(true);
  expect(requests.filter((url) => !url.startsWith('file:'))).toEqual([]);
  await context.close();
});
