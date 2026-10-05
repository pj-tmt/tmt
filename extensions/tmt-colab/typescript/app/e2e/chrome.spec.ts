import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';
import { mkdtemp, mkdir, readFile, rm } from 'node:fs/promises';
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
  'mounted-inactive',
  'mounted-failed',
  'reader',
  'reader-opening',
  'reader-invalid',
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
  if (!executable) throw new Error('Build the socket test and set COLAB_CHROME_FIXTURE_EXECUTABLE');
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
    await expect(page.locator('.guidance-mark')).toHaveAttribute('fill', 'currentColor');
    await expect(page.locator('.guidance-detail')).toContainText('You are signed in as Laptop.');
    await expect(page.locator('.guidance-detail')).toContainText('Build the app:');
  } else {
    await expect(page.locator('.guidance-mark path')).toBeVisible();
  }
}

async function metrics(page: Page) {
  await expect(page.locator('.colab-header:visible')).toHaveCount(1);
  return page.locator('.colab-header:visible').evaluate((header) => {
    const style = getComputedStyle(header);
    const font = (selector: string) => {
      const value = getComputedStyle(header.querySelector(selector)!);
      return {
        family: value.fontFamily,
        size: value.fontSize,
        weight: value.fontWeight,
        lineHeight: value.lineHeight,
        color: value.color,
      };
    };
    return {
      height: header.getBoundingClientRect().height,
      padding: [style.paddingLeft, style.paddingRight],
      gap: style.gap,
      background: style.backgroundColor,
      mark: font('.colab-mark'),
      wordmark: font('.colab-wordmark'),
      title: font('.colab-title'),
      actions: font('.colab-actions'),
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

for (const width of [1440, 390])
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
            if (screen === 'archived')
              await page.getByRole('button', { name: 'Show archived', exact: true }).click();
            if (screen === 'page' || screen === 'reader') {
              await expect(page.locator('iframe')).toHaveAttribute('data-scroll-mode', 'window');
              await expect
                .poll(() =>
                  page.locator('iframe').evaluate((frame) => frame.getBoundingClientRect().height),
                )
                .toBeGreaterThan(1500);
            }
          }
          const current = await metrics(page);
          reference ??= current;
          expect(current, screen).toEqual(reference);
          expect(current.height).toBe(
            parseFloat(width < 480 ? tokens.header['compact-height'] : tokens.header.height),
          );
          expect(current.title.size).toBe(tokens.header['title-size']);
          expect(current.wordmark.size).toBe(tokens.header['wordmark-size']);
          expect(current.wordmark.color).not.toBe(current.title.color);
          await expect(page.locator('.colab-header:visible')).toHaveCSS('position', 'fixed');
          await expect(page.locator('.colab-header:visible')).toHaveCSS('flex-wrap', 'nowrap');
          await containment(page);
          const card = page.locator('.notice:visible');
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
              };
            });
            cardReference ??= geometry;
            expect(geometry, `${screen} state card`).toEqual(cardReference);
            const role =
              screen === 'rust-owner'
                ? 'working'
                : screen.includes('opening') || screen === 'rust-private'
                  ? 'waiting'
                  : screen === 'reader-ended' || screen === 'mounted-inactive'
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
            expect(await card.evaluate((node) => getComputedStyle(node).boxShadow)).toContain(
              color,
            );
            await expect(card.locator('h2')).not.toBeEmpty();
            await expect(card.locator('.notice-eyebrow, .guidance-eyebrow')).not.toBeEmpty();
          }
          if (screen === 'pages') {
            const card = page.locator('.page-card:visible').first();
            await expect(card.locator('h2')).toHaveText('Release notes');
            await expect(card.locator('.page-id')).toHaveText('a19c0460');
            await expect(card.getByRole('button', { name: 'Manage page' })).toBeVisible();
            await expect(card.locator('summary')).toHaveText('Details');
            await expect(page.locator('.page-card:visible').nth(1).locator('h2')).toHaveText(
              'Untitled page',
            );
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
          expect((await page.locator('.colab-header:visible').boundingBox())!.y).toBe(0);
          expect(await metrics(page)).toEqual(reference);
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

test('the page list filters archived pages with a square text toggle, not a native checkbox', async ({
  page,
}) => {
  await page.goto('/');
  await page.evaluate(async () => {
    const path = '/test/chrome-browser.tsx';
    await (await import(path)).mount('pages');
  });
  await expect(page.locator('input[type="checkbox"]')).toHaveCount(0);
  const toggle = page.getByRole('button', { name: 'Show archived', exact: true });
  const box = toggle.locator('.toggle-box');
  await expect(toggle).toHaveAttribute('aria-pressed', 'false');
  await expect(page.locator('#chrome-fixture ul.pages li').first()).toContainText('Release notes');
  const before = await toggle.boundingBox();
  const off = await box.evaluate((node) => getComputedStyle(node).backgroundColor);
  // Space and Enter both toggle; the label is fixed, so the toggle never changes width.
  await toggle.focus();
  await page.keyboard.press('Space');
  await expect(toggle).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByText('No archived pages.', { exact: true })).toBeVisible();
  expect((await toggle.boundingBox())!.width).toBe(before!.width);
  expect(await box.evaluate((node) => getComputedStyle(node).backgroundColor)).not.toBe(off);
  await page.keyboard.press('Enter');
  await expect(toggle).toHaveAttribute('aria-pressed', 'false');
  await expect(page.locator('#chrome-fixture ul.pages li').first()).toContainText('Release notes');
  await expect(toggle).toBeFocused();
  // Pressed or not, it stays a light text button: no fill, no border.
  await page.keyboard.press('Space');
  await expect(toggle).toHaveAttribute('aria-pressed', 'true');
  await expect(toggle).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
  await expect(toggle).toHaveCSS('border-top-width', '0px');
  await page.keyboard.press('Space');
  await expect(toggle).toHaveAttribute('aria-pressed', 'false');
  // Light text button on the token colors: no border, no radius, transparent.
  await expect(toggle).toHaveCSS('border-top-width', '0px');
  await expect(toggle).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
  await expect(box).toHaveCSS('border-radius', '0px');
});

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`archived toggle captures: ${width}px ${theme}`, async ({ page }) => {
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
      const toggle = page.getByRole('button', { name: 'Show archived', exact: true });
      await expect(toggle).toBeVisible();
      await page.screenshot({ path: `/tmp/1716-${width}-${theme}-off.png` });
      await toggle.click();
      await expect(toggle).toHaveAttribute('aria-pressed', 'true');
      await page.screenshot({ path: `/tmp/1716-${width}-${theme}-on.png` });
    });

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`page header shows only labeled actions until they overflow, with a text-sized live dot: ${width}px ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 800 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      await page.goto('/');
      await page.evaluate(async () => {
        const path = '/test/chrome-browser.tsx';
        await (await import(path)).mount('page');
      });
      const header = page.locator('#chrome-fixture .colab-header');
      await expect(header).toBeVisible();
      const overflow = header.locator('.page-overflow-toggle');
      const close = header.locator('.page-menu-close');
      await expect(close).toBeHidden();
      if (width === 1440) {
        // Every action fits: no "more" button and no bare × in the header.
        await expect(overflow).toBeHidden();
        await expect(header.getByRole('button', { name: 'More page actions' })).toBeHidden();
        const dot = header.locator('.status svg.status-dot');
        await expect(dot).toBeVisible();
        const box = (await dot.boundingBox())!;
        expect(box.width).toBeLessThanOrEqual(10);
        expect(box.height).toBeLessThanOrEqual(10);
        for (const button of await header.getByRole('button').all())
          if (await button.isVisible()) {
            const name = (await button.getAttribute('aria-label')) ?? (await button.innerText());
            expect(name.trim(), 'every visible header action is labeled').not.toBe('');
          }
        await page.screenshot({ path: `/tmp/1730-${width}-${theme}-header.png` });
      } else {
        await expect(overflow).toBeVisible();
        await page.screenshot({ path: `/tmp/1730-${width}-${theme}-header.png` });
        await overflow.click();
        await expect(close).toBeVisible();
        await page.screenshot({ path: `/tmp/1730-${width}-${theme}-header-menu.png` });
        await close.click();
        await expect(close).toBeHidden();
      }
    });
