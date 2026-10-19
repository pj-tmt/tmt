import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';
import { mkdtemp, mkdir, readFile, rm } from 'node:fs/promises';
import { fittingCharacters } from './header-title.js';
import { expect, test, type Page } from '@playwright/test';
import tokens from '../../../../../design/tokens/tokens.json' with { type: 'json' };
import type { Screen } from '../test/chrome-browser.js';

const screens: Screen[] = [
  'pages',
  'archived',
  'empty',
  'page',
  'error',
  'not-found',
  'mounted-opening',
  'mounted-failed',
  'reader',
  'reader-opening',
  'reader-invalid',
  'reader-older',
  'reader-ended',
  'reader-failed',
];
const captureDirectory =
  process.env.COLAB_CHROME_CAPTURE_DIR ??
  fileURLToPath(new URL('../test-results/chrome-captures/', import.meta.url));
const run = promisify(execFile);
let nativeDirectory: string;

test.beforeAll(async () => {
  const executable = process.env.COLAB_CHROME_FIXTURE_EXECUTABLE;
  // The gating CI job sets this flag: there a missing fixture is a failure, never a skip.
  if (process.env.COLAB_CHROME_REQUIRED === '1')
    expect(executable, 'COLAB_CHROME_REQUIRED needs the native chrome fixture').toBeTruthy();
  test.skip(
    !executable,
    'Needs the native chrome fixture: build the socket test and set COLAB_CHROME_FIXTURE_EXECUTABLE',
  );
  if (!executable) return;
  nativeDirectory = await mkdtemp('/tmp/colab-chrome-');
  try {
    await run(executable, ['--exact', 'chrome_browser_responses', '--ignored'], {
      env: { ...process.env, COLAB_CHROME_FIXTURE_DIR: nativeDirectory },
      timeout: 10000,
      maxBuffer: 65536,
    });
    await mkdir(captureDirectory, { recursive: true });
  } catch (error) {
    await rm(nativeDirectory, { recursive: true, force: true });
    throw error;
  }
});
test.afterAll(async () => {
  if (nativeDirectory) await rm(nativeDirectory, { recursive: true, force: true });
});

async function nativeScreen(page: Page, name: 'private' | 'owner') {
  const response = JSON.parse(await readFile(`${nativeDirectory}/${name}.json`, 'utf8')) as {
    headers: Record<string, string>;
    body: string;
  };
  const css = JSON.parse(await readFile(`${nativeDirectory}/css.json`, 'utf8')) as typeof response;
  expect(response.headers['content-security-policy']).not.toContain('unsafe-inline');
  expect(css.headers['content-security-policy']).toBe(response.headers['content-security-policy']);
  await page.route('**/native-chrome/assets/chrome.css', (route) => route.fulfill(css));
  await page.route('**/native-chrome/', (route) => route.fulfill(response));
  await page.goto('/native-chrome/');
  await expect(page.locator('.guidance-card')).toBeVisible();
  if (name === 'owner') {
    await expect(page.locator('.guidance-mark')).toHaveAttribute('fill', 'none');
    await expect(page.locator('.tmt-ui-notice-mark')).toContainText('Update needed');
    await expect(page.locator('.tmt-ui-notice-heading')).toHaveText(
      'Update Colab to open this page',
    );
    await expect(page.locator('.guidance-detail')).toContainText('You are signed in as Laptop.');
    await expect(page.locator('.guidance-detail')).toContainText('tmt extension upgrade colab');
    await expect(page.locator('.guidance-detail')).not.toContainText('Build the app:');
  } else {
    await expect(page.locator('.guidance-mark path')).toBeVisible();
  }
}

async function metrics(page: Page) {
  await expect(page.locator('.tmt-ui-header:visible')).toHaveCount(1);
  return page.locator('.tmt-ui-header:visible').evaluate((header) => {
    const style = getComputedStyle(header);
    const font = (selector: string) => {
      const value = getComputedStyle(header.querySelector(selector)!);
      return {
        family: value.fontFamily,
        size: value.fontSize,
        weight: value.fontWeight,
        lineHeight: value.lineHeight,
      };
    };
    return {
      height: header.getBoundingClientRect().height,
      padding: [style.paddingLeft, style.paddingRight],
      gap: style.gap,
      background: style.backgroundColor,
      foreground: style.color,
      mark: font('.tmt-ui-mark'),
      wordmark: font('.tmt-ui-wordmark'),
      title: font('.tmt-ui-title'),
      actions: font('.tmt-ui-actions'),
    };
  });
}

async function containment(page: Page) {
  const result = await page.evaluate(() => {
    const nested = Array.from(document.querySelectorAll<HTMLElement>('body *'))
      .filter((node) => {
        if (!node.getClientRects().length) return false;
        const style = getComputedStyle(node);
        return /^(auto|scroll)$/.test(style.overflowY) && node.scrollHeight > node.clientHeight + 1;
      })
      .map((node) => node.className || node.tagName);
    return {
      nested,
      window: document.scrollingElement === document.documentElement,
      horizontal: document.documentElement.scrollWidth > document.documentElement.clientWidth,
    };
  });
  expect(result).toEqual({ nested: [], window: true, horizontal: false });
}

for (const width of [1440, 390, 320])
  for (const theme of ['light', 'dark'] as const) {
    test(`every screen shares chrome and window scrolling: ${width}px ${theme}`, async ({
      page,
    }) => {
      test.setTimeout(90000);
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      let reference: Awaited<ReturnType<typeof metrics>> | undefined;
      let cardReference: unknown;
      for (const screen of [...screens, 'rust-private', 'rust-owner'] as const) {
        await test.step(screen, async () => {
          if (screen === 'rust-private' || screen === 'rust-owner') {
            await nativeScreen(page, screen === 'rust-private' ? 'private' : 'owner');
          } else {
            await page.goto('/');
            await page.evaluate(async (screen) => {
              const path = '/test/chrome-browser.tsx';
              await (await import(path)).mount(screen);
            }, screen);
            if (screen === 'archived') {
              await page.getByRole('tab', { name: /^Archived/ }).click();
            }
            if (screen === 'page' || screen === 'reader') {
              await expect(page.locator('iframe')).toHaveAttribute('data-scroll-mode', 'window');
              // The frame reports its content height asynchronously; under a loaded hosted runner
              // that took longer than the default 5 s once, so bound the wait explicitly.
              await expect
                .poll(
                  () =>
                    page
                      .locator('iframe')
                      .evaluate((frame) => frame.getBoundingClientRect().height),
                  { timeout: 15_000 },
                )
                .toBeGreaterThan(1500);
            }
          }
          const current = await metrics(page);
          reference ??= current;
          expect(current, screen).toEqual({ ...reference, gap: tokens.header.gap });
          expect(current.height).toBe(
            parseFloat(width < 480 ? tokens.header['compact-height'] : tokens.header.height),
          );
          expect(current.title.size).toBe(tokens.header['title-size']);
          expect(current.wordmark.size).toBe(tokens.header['wordmark-size']);
          await expect(page.locator('.tmt-ui-header:visible')).toHaveCSS('position', 'fixed');
          await expect(page.locator('.tmt-ui-header:visible')).toHaveCSS('flex-wrap', 'nowrap');
          await containment(page);
          const card = page.locator('.tmt-ui-notice:visible');
          if (await card.count()) {
            const geometry = await card.evaluate((node) => {
              const style = getComputedStyle(node);
              const title = getComputedStyle(node.querySelector('h2')!);
              return {
                width: node.getBoundingClientRect().width,
                border: style.borderWidth,
                padding: style.padding,
                font: style.font,
                title: title.font,
                background: style.backgroundColor,
                foreground: style.color,
              };
            });
            cardReference ??= geometry;
            expect(geometry, `${screen} state card`).toEqual(cardReference);
            const role =
              screen === 'rust-owner'
                ? 'waiting'
                : screen.includes('opening') || screen === 'rust-private'
                  ? 'waiting'
                  : screen === 'reader-ended'
                    ? 'muted'
                    : 'blocked';
            const color = await page.evaluate((role) => {
              const probe = document.createElement('span');
              probe.style.color = `var(--c-${role})`;
              document.body.append(probe);
              const value = getComputedStyle(probe).color;
              probe.remove();
              return value;
            }, role);
            await expect(card.locator('svg.lucide')).toHaveCSS('color', color);
            await expect(card).toHaveCSS('box-shadow', 'none');
            await expect(card).toHaveCSS('border-top-width', '1px');
            await expect(card).toHaveCSS('border-radius', '0px');
            const stateWord =
              screen === 'rust-owner'
                ? 'Update needed'
                : screen === 'rust-private'
                  ? 'waiting'
                  : screen.includes('opening')
                    ? 'opening'
                    : screen === 'reader-ended'
                      ? 'ended'
                      : 'failed';
            await expect(card.locator('.tmt-ui-notice-mark')).toContainText(stateWord);
            await expect(card.locator('h2')).not.toBeEmpty();
            await expect(card.locator('.tmt-ui-notice-eyebrow')).not.toBeEmpty();
          }
          if (screen === 'pages') {
            const card = page.locator('.pages > li:visible').first();
            await expect(card.locator('.tmt-ui-list-title')).toHaveText('Release notes');
            await expect(card.getByRole('button', { name: /^Actions for/ })).toBeVisible();
            await card.getByRole('button', { name: /^Actions for/ }).click();
            await expect(page.getByRole('menu').getByText(/^Page ID:/)).toContainText('a19c0460');
            await expect(page.getByRole('menuitem', { name: 'Manage page' })).toBeVisible();
            await page.getByRole('menu').press('Escape');
            await expect(
              page.locator('.pages > li:visible').nth(1).locator('.tmt-ui-list-title'),
            ).toHaveText('Untitled page');
          }
          for (const icon of await page.locator('svg.lucide:visible').all()) {
            await expect(icon).toHaveCSS('stroke-linecap', 'square');
            await expect(icon).toHaveCSS('stroke-linejoin', 'miter');
            await expect(icon).toHaveCSS('stroke-width', `${tokens.header['icon-stroke']}px`);
          }
          await page.screenshot({
            path: `${captureDirectory}/${screen}-${width}-${theme}-top.png`,
          });
          // Short notices cannot naturally scroll. Stress their actual root/header with an
          // inert block below the notice; top captures remain the natural presentation.
          await page.evaluate(() => {
            if (document.documentElement.scrollHeight <= innerHeight + 1) {
              const probe = document.createElement('div');
              probe.setAttribute('aria-hidden', 'true');
              probe.dataset.chromeScrollProbe = 'true';
              probe.style.height = '1800px';
              document.body.append(probe);
            }
            scrollTo(0, (document.documentElement.scrollHeight - innerHeight) / 2);
          });
          await expect.poll(() => page.evaluate(() => scrollY)).toBeGreaterThan(0);
          expect((await page.locator('.tmt-ui-header:visible').boundingBox())!.y).toBe(0);
          expect(await metrics(page)).toEqual(current);
          await containment(page);
          if (screen === 'page' || screen === 'reader') {
            await expect(page.locator('iframe')).toHaveAttribute('scrolling', 'no');
            expect(
              await page
                .frameLocator('iframe')
                .locator('html')
                .evaluate((node) => node.ownerDocument.defaultView!.scrollY),
            ).toBe(0);
          }
          await page.screenshot({
            path: `${captureDirectory}/${screen}-${width}-${theme}-middle.png`,
          });
        });
      }
    });
  }

test('the Archived tab filters archived pages with a tab, not a native checkbox', async ({
  page,
}) => {
  await page.goto('/');
  await page.evaluate(async () => {
    const path = '/test/chrome-browser.tsx';
    await (await import(path)).mount('pages');
  });
  // Only a row's own selection box is a checkbox; the Archived filter is a tab.
  await expect(
    page.locator('#chrome-fixture input[type="checkbox"]:not(.page-select)'),
  ).toHaveCount(0);
  const rows = page.locator('#chrome-fixture ul.pages li');
  await expect(rows.first()).toContainText('Release notes');
  const all = page.locator('#chrome-fixture').getByRole('tab', { name: 'All', exact: true });
  const archived = page.locator('#chrome-fixture').getByRole('tab', { name: /^Archived/ });
  await expect(all).toHaveAttribute('aria-selected', 'true');
  await archived.click();
  await expect(page.getByText('No archived pages.', { exact: true })).toBeVisible();
  await expect(archived).toHaveAttribute('aria-selected', 'true');
  await archived.press('ArrowLeft');
  await expect(all).toHaveAttribute('aria-selected', 'true');
  await expect(rows.first()).toContainText('Release notes');
  await expect(all).toBeFocused();
});

for (const width of [1440, 390, 320])
  for (const theme of ['light', 'dark'] as const)
    test(`archived tab captures: ${width}px ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 700 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      await page.goto('/');
      await page.evaluate(async () => {
        const path = '/test/chrome-browser.tsx';
        await (await import(path)).mount('archived');
      });
      const archived = page.locator('#chrome-fixture').getByRole('tab', { name: /^Archived/ });
      await expect(archived).toBeVisible();
      await page.screenshot({ path: `/tmp/1716-${width}-${theme}-off.png` });
      await archived.click();
      await expect(page.getByText('No archived pages.', { exact: true })).toHaveCount(0);
      await page.screenshot({ path: `/tmp/1716-${width}-${theme}-on.png` });
    });

for (const width of [1440, 390, 320])
  for (const theme of ['light', 'dark'] as const)
    test(`page header shows the mark, names the connection, and keeps status and fitting icon actions: ${width}px ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 800 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((value) => {
        document.documentElement.dataset.theme = value;
      }, theme);
      await page.goto('/');
      await page.evaluate(async () => {
        const path = '/test/chrome-browser.tsx';
        await (await import(path)).mount('page');
      });
      const header = page.locator('#chrome-fixture .tmt-ui-header');
      // The connection names the brand; nothing but the mark and the wordmark shows beside it.
      await expect(
        header.getByRole('link', { name: 'Colab · local · Studio Mac' }),
      ).toHaveAttribute('title', 'Colab · local · Studio Mac');
      await expect(header.locator('.tmt-ui-mark')).toBeVisible();
      await expect(header.locator('.colab-remote-attribution, .colab-remote-label')).toHaveCount(0);
      const heading = header.getByRole('heading', { name: 'Release notes', exact: true });
      expect(await header.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
      if (width < 480) {
        // A phone gives the title the chip's room; the access words lead the More menu instead.
        await expect(header.locator('.page-sharing')).toBeHidden();
        await expect(heading).toBeVisible();
        expect(await fittingCharacters(heading)).toBeGreaterThanOrEqual(width === 390 ? 12 : 1);
      } else {
        await expect(header.locator('.page-sharing')).toHaveText('Private · Live');
        expect(
          await header
            .locator('.page-sharing')
            .evaluate((node) => node.scrollWidth <= node.clientWidth),
        ).toBe(true);
      }
      const actions = header.getByRole('navigation', { name: 'Page actions' });
      await expect(actions.getByRole('button')).toHaveCount(width <= 640 ? 2 : 5);
      for (const button of await actions.getByRole('button').all()) {
        await expect(button).toBeInViewport();
        expect(await button.getAttribute('aria-label')).toBeTruthy();
        await expect(button).toHaveClass(/tmt-ui-icon-action-control/);
      }
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
      await page.screenshot({ path: `/tmp/2314-${width}-${theme}-header.png` });
      const more = actions.getByRole('button', { name: 'More', exact: true });
      await more.click();
      const menu = actions.getByRole('menu');
      await expect(menu).toBeVisible();
      if (width <= 640) {
        for (const name of ['Agents', /^Files \(\d+\)$/, 'Source', 'Export page']) {
          await expect(menu.getByRole('menuitem', { name, exact: true })).toBeVisible();
        }
      }
      if (width < 480) {
        // The access words are the menu's first line: described, not a command.
        await expect(menu).toHaveAccessibleDescription('Private · Live');
        await expect(menu.getByRole('menuitem', { name: /Private/ })).toHaveCount(0);
        expect(await menu.evaluate((node) => node.firstElementChild?.textContent)).toBe(
          'Private · Live',
        );
      } else await expect(menu).not.toContainText('Private · Live');
      const surface = actions.getByRole('dialog', { name: 'More', exact: true });
      expect(await surface.evaluate((node) => node.matches(':modal'))).toBe(true);
      await expect(surface).toHaveCSS('box-shadow', 'none');
      await expect(surface).toHaveCSS('border-width', '1px');
      await expect(surface).toHaveCSS('border-radius', '0px');
      await page.screenshot({ path: `/tmp/2314-${width}-${theme}-header-menu.png` });
      await page.keyboard.press('Escape');
      await expect(menu).toHaveCount(0);
      await expect(more).toBeFocused();
    });

for (const width of [1440, 390, 320])
  for (const theme of ['light', 'dark'] as const)
    test(`fixed header paints above controls and update row in its band: ${width}px ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 800 });
      await page.emulateMedia({ colorScheme: theme });
      await page.goto('/');
      await page.evaluate(async () => {
        const path = '/test/chrome-browser.tsx';
        await (await import(path)).mount('page');
      });
      const header = page.locator('#chrome-fixture .tmt-ui-header');
      await expect(header).toBeVisible();
      for (const className of ['selection-control', 'annotation-popover', 'update-notice']) {
        const painted = await page.evaluate((className) => {
          const header = document.querySelector('#chrome-fixture .tmt-ui-header')!;
          const box = header.getBoundingClientRect();
          const overlay = document.createElement('div');
          overlay.className = className;
          overlay.textContent = 'Overlapping control';
          // Stress the actual host classes in the header band without changing their z-index.
          Object.assign(overlay.style, {
            left: `${box.left + 8}px`,
            top: `${box.top + 8}px`,
            width: '80px',
            height: '32px',
          });
          document.body.append(overlay);
          const hit = document.elementFromPoint(box.left + 16, box.top + 16);
          const result = {
            headerOnTop: !!hit?.closest('.tmt-ui-header'),
            position: getComputedStyle(overlay).position,
            zIndex: Number(getComputedStyle(overlay).zIndex),
          };
          overlay.remove();
          return result;
        }, className);
        expect(painted.position).toBe('fixed');
        expect(painted.zIndex).toBeGreaterThan(0);
        expect(painted.headerOnTop, `${className} must not cover the fixed header`).toBe(true);
      }
    });
