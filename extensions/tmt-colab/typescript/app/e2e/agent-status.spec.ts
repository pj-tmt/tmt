import { pageAction } from '../test/page-actions.js';
import { expect, test, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';
const fixture = '/test/agent-status-browser.html';
async function command(
  page: Page,
  name:
    | 'setMode'
    | 'replaceClient'
    | 'resolvePrevious'
    | 'failPage'
    | 'disableClient'
    | 'mountAdmissionProbe'
    | 'setAdmission'
    | 'resolveOldest'
    | 'refuseNewest',
  value?: string | boolean,
) {
  await page.evaluate(
    async ({ name, value }) => {
      const path = '/test/agent-status-browser.tsx';
      const fixture = await import(path);
      fixture[name](value);
    },
    { name, value },
  );
}
async function openStatus(page: Page) {
  await (await pageAction(page, 'Agents')).click();
  return page.locator('.page-drawer[data-panel=agents]');
}
async function proof(page: Page) {
  return page.evaluate(async () => {
    const path = '/test/agent-status-browser.tsx';
    return (await import(path)).proof();
  });
}
for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`actual agent drawer shows admitted presence and read health: ${width}px ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 844 });
      await page.emulateMedia({ colorScheme: theme });
      await page.goto(fixture);
      await page.evaluate((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      await expect(
        page.frameLocator('iframe').getByRole('heading', { name: 'Storage discussion' }),
      ).toBeVisible();
      // The page observes driver metadata once; the closed drawer adds no reads or effects.
      expect(await proof(page)).toEqual({
        checks: 1,
        prepares: 0,
        writes: 0,
        ledgerActions: 0,
        destinationReads: 0,
      });
      const before = await page.locator('iframe').boundingBox();
      const panel = await openStatus(page);
      await expect(panel).toBeVisible();
      await expect(panel.locator('.agent-status-list li')).toHaveCount(3);
      for (const presence of ['Active', 'Offline', 'Unknown'])
        await expect(panel.getByText(presence, { exact: true })).toBeVisible();
      await expect(panel.getByText('Read succeeded', { exact: true })).toHaveCount(2);
      await expect(panel.getByText('Same name', { exact: true })).toHaveCount(2);
      await expect(panel.getByText('Studio Mac', { exact: true })).toHaveCount(3);
      expect((await page.locator('iframe').boundingBox())?.width).toBe(before?.width);
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
      const captures = process.env.COLAB_STATUS_CAPTURE_DIR;
      if (captures) {
        mkdirSync(captures, { recursive: true });
        await page.screenshot({ path: `${captures}/${width}-${theme}-ready.png` });
      }
      await command(page, 'setMode', 'directory-failed');
      await panel.getByRole('button', { name: 'Recheck status' }).click();
      await expect(
        panel.getByText('Previous snapshot · current presence is unavailable'),
      ).toBeVisible();
      await expect(panel.locator('.agent-status-list li')).toHaveCount(3);
      await expect(panel.getByText('Read succeeded', { exact: true })).toHaveCount(1);
      if (captures) await page.screenshot({ path: `${captures}/${width}-${theme}-stale.png` });
      expect(await proof(page)).toEqual({
        checks: 3,
        prepares: 0,
        writes: 0,
        ledgerActions: 0,
        destinationReads: 0,
      });
      await panel.getByRole('button', { name: 'Close Agents', exact: true }).click();
      await expect(panel).not.toBeVisible();
    });
  }
for (const [mode, session, directory] of [
  ['session-ended', 'Session ended', 'Session ended'],
  ['session-evicted', 'Session limit reached', 'Read unavailable'],
  ['channel-unavailable', 'Session channel unavailable', 'Session channel unavailable'],
  ['scope-refused', 'Read succeeded', 'Read refused'],
] as const) {
  test(`${mode} is separate from presence and clears current rows`, async ({ page }) => {
    await page.goto(fixture);
    const panel = await openStatus(page);
    await expect(panel.locator('.agent-status-list li')).toHaveCount(3);
    await command(page, 'setMode', mode);
    await panel.getByRole('button', { name: 'Recheck status' }).click();
    const health = panel.locator('.agent-status-health dd');
    await expect(health.nth(1)).toHaveText(session);
    await expect(health.nth(2)).toHaveText(directory);
    await expect(panel.locator('.agent-status-list li')).toHaveCount(0);
    await expect(panel.getByText('No successful directory read yet.')).toBeVisible();
    const state = await proof(page);
    expect(state.prepares + state.writes + state.ledgerActions + state.destinationReads).toBe(0);
  });
}
test('empty successful directory and unavailable page are distinct', async ({ page }) => {
  await page.goto(fixture);
  await command(page, 'setMode', 'empty');
  const panel = await openStatus(page);
  await expect(panel.getByText('No admitted agents in this directory.')).toBeVisible();
  await command(page, 'failPage');
  await expect(panel.locator('.agent-status-list li')).toHaveCount(0);
  await expect(panel.getByRole('button', { name: 'Recheck status' })).toBeDisabled();
  expect((await proof(page)).checks).toBe(2);
});

test('same-client admission restoration cannot resurrect cached rows or late results', async ({
  page,
}) => {
  await page.goto(fixture);
  await command(page, 'mountAdmissionProbe');
  const panel = page.getByRole('region', { name: 'Agents', exact: true });
  await expect(panel.locator('.agent-status-list li')).toHaveCount(3);
  const input = page.getByRole('combobox', { name: 'Message', exact: true });
  await input.fill('Keep the draft through admission loss');
  await command(page, 'setMode', 'pending');
  await panel.getByRole('button', { name: 'Recheck status' }).click();
  await expect(panel.getByRole('button', { name: 'Recheck status' })).toBeDisabled();
  await command(page, 'setAdmission', false);
  await expect(panel.locator('.agent-status-list li')).toHaveCount(0);
  await command(page, 'setAdmission', true);
  await expect(panel.getByRole('button', { name: 'Recheck status' })).toBeDisabled();
  await expect(panel.locator('.agent-status-list li')).toHaveCount(0);
  await expect(panel.getByText('Last successful check:', { exact: false })).toHaveCount(0);
  await command(page, 'resolveOldest');
  await expect(panel.locator('.agent-status-list li')).toHaveCount(0);
  await expect(panel.getByRole('button', { name: 'Recheck status' })).toBeDisabled();
  await command(page, 'refuseNewest');
  await expect(panel.getByText('Read refused', { exact: true })).toBeVisible();
  await expect(panel.locator('.agent-status-list li')).toHaveCount(0);
  await expect(panel.getByText('No successful directory read yet.')).toBeVisible();
  await expect(input).toHaveText('Keep the draft through admission loss', { useInnerText: true });
  const effects = await proof(page);
  // One initial page presentation read, then the probe’s three explicit status reads.
  expect(effects.checks).toBe(4);
  expect(effects.prepares + effects.writes + effects.ledgerActions).toBe(0);
});
test('replaced binding drops the old pending directory and retains a Chat draft', async ({
  page,
}) => {
  await page.goto(fixture);
  await (await pageAction(page, 'Chat')).click();
  const chat = page.locator('.page-drawer[data-panel=chat]');
  await chat.getByRole('combobox', { name: 'Message', exact: true }).fill('Retain this draft');
  await chat.getByRole('button', { name: 'Close Chat', exact: true }).click();
  const panel = await openStatus(page);
  await expect(panel.locator('.agent-status-list li')).toHaveCount(3);
  await command(page, 'setMode', 'pending');
  await panel.getByRole('button', { name: 'Recheck status' }).click();
  await expect(panel.getByRole('button', { name: 'Recheck status' })).toBeDisabled();
  await command(page, 'replaceClient');
  await expect(panel.getByText('New Unobserved agent', { exact: true })).toBeVisible();
  await command(page, 'resolvePrevious');
  await expect(panel.getByText('Unobserved agent', { exact: true })).toHaveCount(0);
  await panel.getByRole('button', { name: 'Close Agents', exact: true }).click();
  await (await pageAction(page, 'Chat')).click();
  await expect(chat.getByRole('combobox', { name: 'Message', exact: true })).toHaveText(
    'Retain this draft',
    { useInnerText: true },
  );
  const state = await proof(page);
  expect(state.prepares + state.writes + state.ledgerActions).toBe(0);
});

test('disabling the Ask client leaves the page usable and performs no further status reads', async ({
  page,
}) => {
  await page.goto(fixture);
  await expect(
    page.frameLocator('iframe').getByRole('heading', { name: 'Storage discussion' }),
  ).toBeVisible();
  const before = await proof(page);
  expect(before.checks).toBe(1);
  await command(page, 'disableClient');
  const panel = await openStatus(page);
  await expect(panel.getByRole('button', { name: 'Recheck status' })).toBeDisabled();
  await expect(panel.getByText('No successful directory read yet.')).toBeVisible();
  await expect(
    page.frameLocator('iframe').getByRole('heading', { name: 'Storage discussion' }),
  ).toBeVisible();
  expect(await proof(page)).toEqual({
    checks: before.checks,
    prepares: 0,
    writes: 0,
    ledgerActions: 0,
    destinationReads: 0,
  });
});
