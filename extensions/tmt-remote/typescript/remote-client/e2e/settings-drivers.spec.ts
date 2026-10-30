import { expect, test, type Page } from '@playwright/test';
import { join } from 'node:path';
import {
  buildSettingsHost,
  openSettings,
  removeSettingsHost,
  type SettingsHost,
} from './settings-host.js';

// localhost is a secure context, which the clipboard needs; every request is answered by the route.
const origin = 'http://localhost:4578';
let host: SettingsHost;
test.beforeAll(async () => {
  host = await buildSettingsHost('./settings-drivers-harness.ts', 'settings-drivers-');
});
test.afterAll(() => removeSettingsHost(host));
const open = (page: Page, state: string, width = 1440, theme = 'light') =>
  openSettings(page, host, origin, { section: 'drivers', state, width, theme });
const reads = (page: Page) => page.evaluate(() => window.driverSourceReads);
const capture = async (page: Page, name: string) => {
  if (!process.env.TMT_SETTINGS_CAPTURE_DIR) return;
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: join(process.env.TMT_SETTINGS_CAPTURE_DIR, name), fullPage: true });
};

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'])
    test(`injected driver and hook records at ${width} ${theme}`, async ({ page }) => {
      await open(page, 'injected', width, theme);
      await expect(
        page.getByRole('heading', {
          name: 'TMT settings / Drivers & hooks',
          exact: true,
          level: 1,
        }),
      ).toBeVisible();
      const notice = page.locator('.tmt-settings-warning');
      await expect(notice).toHaveAttribute('data-tone', 'neutral');
      await expect(notice).toContainText('not this machine');
      await expect(page.locator('#tmt-settings-note')).toBeHidden();
      await expect(page.getByRole('button', { name: 'Copy tmt driver ls' })).toBeVisible();
      await expect(page.getByRole('button', { name: 'Copy tmt setup --status' })).toBeVisible();
      await expect(page.getByRole('heading', { name: 'Host drivers' })).toBeVisible();
      await expect(page.getByRole('heading', { name: 'Agent hooks' })).toBeVisible();
      await expect(
        page.getByRole('heading', { name: 'Different connections. Clear roles.' }),
      ).toBeVisible();
      const records = page.locator('.tmt-settings-record');
      await expect(records.locator('strong')).toHaveText(['Herdr', 'Zellij', 'Claude', 'Codex']);
      const [herdr, zellij, claude, codex] = await records.all();
      await expect(herdr!).toContainText('Host driver for agent windows.');
      await expect(herdr!).toContainText('Approved');
      await expect(herdr!).toContainText('0.2.0');
      // A driver that no longer runs as approved names its reason, and has no product line.
      await expect(zellij!).toContainText('binary changed');
      await expect(zellij!).not.toContainText('Approved');
      await expect(zellij!.locator('.tmt-settings-help')).toHaveCount(0);
      await expect(zellij!.locator('.tmt-ui-chip')).toHaveAttribute('data-tone', 'waiting');
      await expect(claude!).toContainText('Usage collection records agent usage for Digest.');
      await expect(claude!).toContainText('Collection on');
      await expect(codex!).toContainText('Collection off');
      await expect(codex!).toContainText('Set up');
      // A state is a word and a mark, never colour alone.
      await expect(page.locator('.tmt-settings-record .tmt-ui-state-mark')).toHaveCount(4);
      await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
      await expect(page.locator('html')).toHaveJSProperty('scrollWidth', width);
      const fits = await records.evaluateAll((nodes) =>
        nodes.every((node) =>
          [...node.querySelectorAll<HTMLElement>('strong, .tmt-ui-chip-label')].every(
            (label) => label.scrollWidth <= label.clientWidth,
          ),
        ),
      );
      expect(fits).toBe(true);
      // The two commands sit side by side on a wide page and stack on a narrow one.
      const [first, second] = await page
        .locator('.tmt-settings-commands > .tmt-ui-command')
        .evaluateAll((nodes) => nodes.map((node) => node.getBoundingClientRect()));
      if (width === 1440) expect(second!.top).toBe(first!.top);
      else expect(second!.top).toBeGreaterThan(first!.bottom - 1);
      await capture(page, `drivers-injected-${width}-${theme}.png`);
      expect(await reads(page)).toBe(1);
    });

test('Copy puts the exact command on the clipboard, and says so when it cannot', async ({
  page,
  context,
}) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write'], { origin });
  await open(page, 'injected');
  await page.getByRole('button', { name: 'Copy tmt setup --status' }).click();
  await expect(page.locator('.tmt-ui-command-feedback').nth(1)).toHaveText('Copied.');
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe('tmt setup --status');
  await page.evaluate(() => {
    Object.defineProperty(navigator, 'clipboard', {
      value: { writeText: () => Promise.reject(new Error('denied')) },
      configurable: true,
    });
  });
  await page.getByRole('button', { name: 'Copy tmt driver ls' }).click();
  await expect(page.locator('.tmt-ui-command-feedback').first()).toContainText('Copy failed.');
});

for (const state of ['unavailable', 'empty', 'refused', 'failed', 'observed'])
  test(`driver source state ${state} stays truthful`, async ({ page }) => {
    await open(page, state);
    if (state === 'failed') {
      await expect(page.locator('#tmt-settings-note')).toHaveText('Settings could not be read.');
      await expect(page.locator('#tmt-settings-section')).toBeEmpty();
      await expect(
        page.getByRole('heading', {
          level: 1,
          name: 'TMT settings / Drivers & hooks',
          exact: true,
        }),
      ).toBeVisible();
      await expect(page.locator('.tmt-page-loading')).toHaveCount(0);
    } else if (state === 'observed') {
      await expect(page.locator('.tmt-settings-warning')).toContainText(
        'Read-only driver and hook records',
      );
      await expect(page.locator('.tmt-settings-warning')).toHaveAttribute('data-tone', 'neutral');
      await expect(page.locator('.tmt-settings-record')).toHaveCount(4);
      await expect(page.locator('#tmt-settings-section')).not.toContainText('Sample');
    } else {
      // No record, version or state is inferred when the browser has no read.
      await expect(page.locator('.tmt-settings-record')).toHaveCount(0);
      await expect(page.locator('.tmt-settings-warning')).toHaveAttribute(
        'data-tone',
        state === 'empty' ? 'neutral' : 'waiting',
      );
      await expect(page.locator('#tmt-settings-section')).toContainText(
        state === 'empty'
          ? 'No drivers or agent hooks are set up.'
          : 'Driver and hook records are not available to this browser yet.',
      );
      if (state === 'refused')
        await expect(page.locator('#tmt-settings-section')).toContainText(
          'Permission refused for driver inventory.',
        );
      await expect(page.locator('#tmt-settings-section')).not.toContainText('Approved');
      await expect(page.locator('#tmt-settings-section')).not.toContainText('0.2.0');
      await expect(page.getByRole('button', { name: 'Copy tmt driver ls' })).toBeVisible();
      await expect(page.getByRole('button', { name: 'Copy tmt setup --status' })).toBeVisible();
      await expect(
        page.getByRole('heading', { name: 'Different connections. Clear roles.' }),
      ).toBeVisible();
    }
    expect(await reads(page)).toBe(1);
  });

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'])
    test(`production driver notice at ${width} ${theme}`, async ({ page }) => {
      await open(page, 'unavailable', width, theme);
      await expect(page.locator('.tmt-settings-warning')).toBeVisible();
      await expect(page.locator('html')).toHaveJSProperty('scrollWidth', width);
      await capture(page, `drivers-production-${width}-${theme}.png`);
    });
