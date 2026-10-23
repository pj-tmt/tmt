import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, test, type Page } from '@playwright/test';

// The bundled page runtime (#1693) with its generated theme, built the way the app builds it and
// run under the renderer frame's CSP: the classes a page writes must resolve to the design tokens
// in both app themes, with no default palette, no shadows, square corners and one focus outline.
const app = fileURLToPath(new URL('..', import.meta.url));
const tokens = JSON.parse(
  readFileSync(new URL('../../../../../design/tokens/tokens.json', import.meta.url), 'utf8'),
);
const policy = [
  "default-src 'none'",
  "script-src 'unsafe-inline'",
  "style-src 'unsafe-inline'",
  'img-src data:',
  "connect-src 'none'",
  "font-src 'none'",
].join('; ');

let runtime = '';
let theme = '';
let output = '';
test.beforeAll(() => {
  output = mkdtempSync(join(tmpdir(), 'tmt-runtime-theme-'));
  execFileSync('./node_modules/.bin/vp', ['build', '--mode', 'runtime', '--outDir', output], {
    cwd: app,
    stdio: 'pipe',
  });
  runtime = readFileSync(join(output, 'assets/colab-runtime.js'), 'utf8');
  theme = readFileSync(join(output, 'assets/colab-runtime.css'), 'utf8');
});
test.afterAll(() => {
  if (output) rmSync(output, { recursive: true, force: true });
});

const rgb = (hex: string) => {
  const value = parseInt(hex.slice(1, 7), 16);
  return `rgb(${value >> 16}, ${(value >> 8) & 255}, ${value & 255})`;
};
const role = (group: 'color' | 'surface', key: string, mode: 'light' | 'dark') =>
  rgb(tokens.browser[group][key][mode]);
const none = 'rgba(0, 0, 0, 0)';

const command = "tmt ops sq set gui-4 'task=release checklist'";
async function shortPage(page: Page, mode: 'light' | 'dark', authorBase = '') {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.setContent(
    `<!doctype html><html data-theme="${mode}"><head>` +
      `<meta http-equiv="Content-Security-Policy" content="${policy}">` +
      `<style type="text/tailwindcss">${authorBase}${theme}</style>` +
      `<script>${runtime.replaceAll('</script', '<\\/script')}</script>` +
      `</head><body><div id="app"></div><script>
        const { createRoot, html } = tmt;
        function App() {
          return html\`<section class="bg-paper text-text p-4 font-sans">
            <label>Filter <input class="border border-edge" aria-label="Filter" /></label>
            <ul>\${['Build', 'Review', 'Deploy', 'Release'].map((name) => html\`<li>\${name}</li>\`)}</ul>
            <button class="border border-edge px-2">Open</button>
            <pre class="border border-edge bg-code p-4 text-sm"><code>${command}</code></pre>
          </section>\`;
        }
        createRoot(document.getElementById('app')).render(html\`<\${App} />\`);
      </script></body></html>`,
  );
  // Wait for both the React content and Tailwind's first compiled paint.
  await expect(page.locator('section')).toHaveCSS(
    'background-color',
    role('surface', 'paper', mode),
  );
}

for (const mode of ['light', 'dark'] as const) {
  test(`a short runtime page paints the ${mode} paper canvas below its content at 390px`, async ({
    page,
  }) => {
    await shortPage(page, mode);
    const below = await page.evaluate(() => ({
      contentBottom: document.querySelector('section')!.getBoundingClientRect().bottom,
      viewport: innerHeight,
      tag: document.elementFromPoint(8, innerHeight - 8)?.tagName,
      background: getComputedStyle(document.documentElement).backgroundColor,
    }));
    expect(below.contentBottom).toBeLessThan(below.viewport);
    expect(below.tag).toBe('HTML');
    expect(below.background).toBe(role('surface', 'paper', mode));
  });

  test(`a long runtime code line scrolls inside its ${mode} pre at 390px`, async ({ page }) => {
    await shortPage(page, mode);
    const pre = page.locator('pre');
    await expect(pre).toHaveCSS('overflow-x', 'auto');
    await expect(pre).toHaveText(command);
    const bounds = await pre.evaluate((element) => {
      element.scrollLeft = element.scrollWidth;
      return {
        client: element.clientWidth,
        width: element.scrollWidth,
        left: element.scrollLeft,
        documentWidth: document.documentElement.scrollWidth,
        viewportWidth: innerWidth,
      };
    });
    expect(bounds.width).toBeGreaterThan(bounds.client);
    expect(bounds.left).toBeGreaterThan(0);
    expect(bounds.left + bounds.client).toBe(bounds.width);
    expect(bounds.documentWidth).toBe(bounds.viewportWidth);
  });

  test(`an author's earlier pre base style wins over the ${mode} runtime default`, async ({
    page,
  }) => {
    // Same base layer, earlier source order: the runtime's :where(pre) must have zero specificity.
    await shortPage(page, mode, '@layer base { pre { max-width: 17rem; overflow-x: scroll; } }');
    await expect(page.locator('pre')).toHaveCSS('max-width', '272px');
    await expect(page.locator('pre')).toHaveCSS('overflow-x', 'scroll');
  });

  test(`the runtime theme resolves page classes to the ${mode} design tokens`, async ({ page }) => {
    const problems: string[] = [];
    page.on('pageerror', (error) => problems.push(String(error)));
    page.on('console', (message) => {
      if (message.type() === 'error') problems.push(message.text());
    });
    await page.addInitScript(() =>
      document.addEventListener('securitypolicyviolation', (event) =>
        console.error(`CSP violation: ${event.violatedDirective} ${event.blockedURI}`),
      ),
    );
    const classes = [
      'bg-paper',
      'text-text',
      'border-rule',
      'dark:bg-sheet',
      'rounded',
      'rounded-lg',
      'shadow-md',
      'drop-shadow-md',
      'ring',
      'outline',
      'bg-red-500',
      'font-sans',
      'font-display',
      'font-mono',
    ];
    const cells = classes.map((name, index) => `<div id="c${index}" class="${name}">x</div>`);
    await page.setContent(
      `<!doctype html><html data-theme="${mode}"><head>` +
        `<meta http-equiv="Content-Security-Policy" content="${policy}">` +
        `<style type="text/tailwindcss">${theme}</style>` +
        `<script>${runtime.replaceAll('</script', '<\\/script')}</script>` +
        `</head><body>${cells.join('')}<button id="focus">Focus</button></body></html>`,
    );

    const style = (index: number, property: string) =>
      page.evaluate(
        ([id, name]) =>
          (
            getComputedStyle(document.getElementById(id as string)!) as unknown as Record<
              string,
              string
            >
          )[name as string],
        [`c${index}`, property],
      );
    // Tailwind compiles from the page's classes after load: wait for its first paint.
    await expect.poll(() => style(0, 'backgroundColor')).toBe(role('surface', 'paper', mode));

    expect(await style(1, 'color')).toBe(role('color', 'text', mode));
    expect(await style(2, 'borderTopColor')).toBe(role('surface', 'rule', mode));
    expect(await style(3, 'backgroundColor')).toBe(
      mode === 'dark' ? role('surface', 'sheet', mode) : none,
    );
    const radius = tokens.browser.metric.radius;
    expect(await style(4, 'borderTopLeftRadius')).toBe(radius);
    expect(await style(5, 'borderTopLeftRadius')).toBe(radius);
    expect(await style(6, 'boxShadow')).toBe('none');
    expect(await style(7, 'filter')).toBe('none');
    // The default palette is gone: an unknown colour class compiles to nothing.
    expect(await style(10, 'backgroundColor')).toBe(none);
    // The ring and outline utilities default to the focus colour.
    expect(await style(8, 'boxShadow')).toContain(role('color', 'focus', mode));
    expect(await style(9, 'outlineColor')).toBe(role('color', 'focus', mode));
    // System fonts only: the token's leading web family is not in any stack.
    for (const [index, name] of [
      [11, 'body'],
      [12, 'display'],
      [13, 'mono'],
    ] as const) {
      const web = /^"([^"]+)"/.exec(tokens.font[name].stack)![1];
      const family = await style(index, 'fontFamily');
      expect(family).toMatch(/system-ui|ui-monospace/);
      expect(family).not.toContain(web);
    }

    // One focus frame: a single outline of the token width in the focus colour, with no gap.
    await page.keyboard.press('Tab');
    expect(await page.evaluate(() => document.activeElement?.id)).toBe('focus');
    const focus = await page.evaluate(() => {
      const style = getComputedStyle(document.getElementById('focus')!);
      return [style.outlineStyle, style.outlineWidth, style.outlineColor, style.outlineOffset];
    });
    expect(focus).toEqual([
      'solid',
      tokens.browser.metric['focus-width'],
      role('color', 'focus', mode),
      '0px',
    ]);

    expect(problems).toEqual([]);
  });
}
