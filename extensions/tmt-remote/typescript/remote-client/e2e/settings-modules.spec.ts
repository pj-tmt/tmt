import { expect, test, type Page } from '@playwright/test';
import { join } from 'node:path';
import {
  buildSettingsHost,
  openSettings,
  removeSettingsHost,
  type SettingsHost,
} from './settings-host.js';

// localhost is a secure context, which the clipboard needs; every request is answered by the route.
const origin = 'http://localhost:4577';
let host: SettingsHost;
test.beforeAll(async () => {
  host = await buildSettingsHost('./settings-modules-harness.ts', 'settings-modules-');
});
test.afterAll(() => removeSettingsHost(host));
const open = (page: Page, state: string, width = 1440, theme = 'light') =>
  openSettings(page, host, origin, { section: 'modules', state, width, theme });
const reads = (page: Page) => page.evaluate(() => window.moduleSourceReads);

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'])
    test(`injected module records and their actions at ${width} ${theme}`, async ({ page }) => {
      await open(page, 'injected', width, theme);
      await expect(
        page.getByRole('heading', { name: 'TMT settings / Modules', exact: true }),
      ).toBeVisible();
      await expect(page.getByRole('heading', { name: 'Sample installed modules' })).toBeVisible();
      const notice = page.locator('.tmt-settings-warning');
      await expect(notice).toHaveAttribute('data-tone', 'neutral');
      await expect(notice).toContainText('not this machine');
      await expect(page.locator('#tmt-settings-note')).toBeHidden();
      const records = page.locator('.tmt-settings-record');
      await expect(records).toHaveCount(5);
      await expect(records.locator('strong')).toHaveText([
        'Colab',
        'Remote',
        'Ops',
        'Digest',
        'Office',
      ]);
      await expect(records.first()).toContainText('Shared pages and agent collaboration.');
      await expect(records.first()).toContainText('Installed');
      // The version names its channel already, so the channel is not repeated beside it.
      await expect(records.first().locator('.tmt-settings-mono')).toHaveText('0.1.0-alpha.231');
      // The not-installed module is a word and a ring, with no action and no description.
      await expect(records.last()).toContainText('Not installed');
      await expect(records.last().locator('a')).toHaveCount(0);
      await expect(records.last().locator('.tmt-settings-help')).toHaveCount(0);
      const actions = page.locator('.tmt-settings-record a');
      await expect(actions).toHaveText(['Settings', 'Settings', 'Settings', 'Settings']);
      expect(
        await actions.evaluateAll((links) => links.map((link) => link.getAttribute('href'))),
      ).toEqual(['/colab/settings/', '/remote/settings/', '/ops/settings/', '/digest/settings/']);
      await expect(page.getByRole('link', { name: 'Open Remote settings →' })).toHaveAttribute(
        'href',
        '/remote/settings/',
      );
      // A state is a word and a mark, never colour alone.
      await expect(page.locator('.tmt-settings-record .tmt-ui-state-mark')).toHaveCount(5);
      await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
      await expect(page.locator('html')).toHaveJSProperty('scrollWidth', width);
      // The action ends the name row at any width; the facts sit beside it wide and below it narrow.
      const [row, name, facts, action] = await records
        .first()
        .evaluate((node) =>
          [
            node,
            node.querySelector('strong')!,
            node.querySelector('.tmt-settings-record-facts')!,
            node.querySelector('a')!,
          ].map((part) => part.getBoundingClientRect().toJSON()),
        );
      expect(action!.right).toBeLessThanOrEqual(row!.right);
      expect(row!.right - action!.right).toBeLessThan(2);
      if (width === 390) {
        expect(action!.top).toBeLessThan(name!.bottom);
        expect(facts!.top).toBeGreaterThan(name!.bottom - 1);
      } else
        expect(
          Math.abs(action!.top + action!.height / 2 - (facts!.top + facts!.height / 2)),
        ).toBeLessThan(4);
      const fits = await page
        .locator('.tmt-settings-record')
        .evaluateAll((nodes) =>
          nodes.every((node) =>
            [...node.querySelectorAll<HTMLElement>('strong, .tmt-ui-chip-label')].every(
              (label) => label.scrollWidth <= label.clientWidth,
            ),
          ),
        );
      expect(fits).toBe(true);
      if (process.env.TMT_SETTINGS_CAPTURE_DIR) {
        await page.evaluate(() => document.fonts.ready);
        await page.screenshot({
          path: join(
            process.env.TMT_SETTINGS_CAPTURE_DIR,
            `modules-injected-${width}-${theme}.png`,
          ),
          fullPage: true,
        });
      }
      expect(await reads(page)).toBe(1);
    });

test('the keyboard reaches each action and shows the one focus frame of the leaf', async ({
  page,
}) => {
  await open(page, 'injected');
  const links = page.locator('.tmt-settings-record a');
  const edge = (link: ReturnType<typeof links.first>) =>
    link.evaluate((node) => getComputedStyle(node).borderTopColor);
  await links.first().focus();
  await page.keyboard.press('Tab');
  await expect(links.nth(1)).toBeFocused();
  await page.keyboard.press('Shift+Tab');
  await expect(links.first()).toBeFocused();
  // The frame is the action's edge in the focus colour: the focused edge differs from a resting one.
  await expect.poll(() => edge(links.first())).not.toBe(await edge(links.nth(1)));
});

test('Copy puts the exact command on the clipboard, and says so when it cannot', async ({
  page,
  context,
}) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write'], { origin });
  await open(page, 'injected');
  await page.getByRole('button', { name: 'Copy tmt extension ls' }).first().click();
  await expect(page.locator('.tmt-ui-command-feedback').first()).toHaveText('Copied.');
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe('tmt extension ls');
  await page.evaluate(() => {
    Object.defineProperty(navigator, 'clipboard', {
      value: { writeText: () => Promise.reject(new Error('denied')) },
      configurable: true,
    });
  });
  await page.getByRole('button', { name: 'Copy tmt extension ls' }).first().click();
  await expect(page.locator('.tmt-ui-command-feedback').first()).toContainText('Copy failed.');
});

test('the actions follow the shell description when it arrives late', async ({ page }) => {
  await open(page, 'late');
  await expect(page.locator('.tmt-settings-record')).toHaveCount(5);
  await expect(page.locator('.tmt-settings-record a')).toHaveCount(0);
  await expect(page.locator('.tmt-settings-record a')).toHaveCount(4);
  expect(await reads(page)).toBe(1);
});

test('without a description there is no action and no guessed link', async ({ page }) => {
  await open(page, 'undescribed');
  await expect(page.locator('.tmt-settings-record')).toHaveCount(5);
  await expect(page.locator('.tmt-settings-record a')).toHaveCount(0);
  await expect(page.getByRole('link', { name: 'Open Remote settings →' })).toHaveCount(0);
});

for (const state of ['unavailable', 'empty', 'refused', 'failed', 'observed'])
  test(`module source state ${state} stays truthful`, async ({ page }) => {
    await open(page, state);
    if (state === 'failed') {
      await expect(page.locator('#tmt-settings-note')).toHaveText('Settings could not be read.');
      await expect(page.locator('#tmt-settings-section')).toBeEmpty();
      await expect(
        page.getByRole('heading', { level: 1, name: 'TMT settings / Modules', exact: true }),
      ).toBeVisible();
      await expect(page.locator('.tmt-page-loading')).toHaveCount(0);
    } else if (state === 'observed') {
      await expect(
        page.getByRole('heading', { name: 'Installed modules', exact: true }),
      ).toBeVisible();
      await expect(page.locator('.tmt-settings-warning')).toContainText('Read-only module records');
      await expect(page.locator('.tmt-settings-warning')).toHaveAttribute('data-tone', 'neutral');
      await expect(page.locator('.tmt-settings-record')).toHaveCount(5);
      await expect(page.locator('#tmt-settings-section')).not.toContainText('Sample');
    } else {
      // No record, version or channel is inferred when the browser has no read.
      await expect(page.locator('.tmt-settings-record')).toHaveCount(0);
      await expect(page.locator('.tmt-settings-warning')).toHaveAttribute('data-tone', 'waiting');
      await expect(page.locator('#tmt-settings-section')).toContainText(
        state === 'empty'
          ? 'No modules to show.'
          : 'Module inventory is not available to this browser yet.',
      );
      if (state === 'refused')
        await expect(page.locator('#tmt-settings-section')).toContainText(
          'Permission refused for module inventory.',
        );
      await expect(page.locator('#tmt-settings-section')).not.toContainText('0.1.0');
      await expect(page.getByRole('button', { name: 'Copy tmt extension ls' })).toBeVisible();
    }
    expect(await reads(page)).toBe(1);
  });

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'])
    test(`production module notice at ${width} ${theme}`, async ({ page }) => {
      await open(page, 'unavailable', width, theme);
      await expect(page.locator('.tmt-settings-warning')).toBeVisible();
      await expect(page.locator('html')).toHaveJSProperty('scrollWidth', width);
      if (process.env.TMT_SETTINGS_CAPTURE_DIR) {
        await page.evaluate(() => document.fonts.ready);
        await page.screenshot({
          path: join(
            process.env.TMT_SETTINGS_CAPTURE_DIR,
            `modules-production-${width}-${theme}.png`,
          ),
          fullPage: true,
        });
      }
    });
