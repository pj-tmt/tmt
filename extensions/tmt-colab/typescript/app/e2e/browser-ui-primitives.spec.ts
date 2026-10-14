import { test, expect, type Page } from '@playwright/test';
import { readFileSync } from 'node:fs';

const tokens = JSON.parse(
  readFileSync(new URL('../../../../../design/tokens/tokens.json', import.meta.url), 'utf8'),
);
const rgb = (hex: string) =>
  `rgb(${[1, 3, 5].map((offset) => parseInt(hex.slice(offset, offset + 2), 16)).join(', ')})`;
async function gallery(page: Page, theme: 'light' | 'dark' = 'light') {
  await page.emulateMedia({ colorScheme: theme });
  await page.goto('/test/browser-ui-foundation.html');
  await page.getByLabel('Outer theme').selectOption(theme);
  await page.getByRole('button', { name: 'Load isolated UI CSS' }).click();
  await expect(page.locator('[data-css-loaded=true]')).toHaveText('UI CSS loaded');
  await expect(page.getByRole('heading', { name: 'Slice2 primitive qualification' })).toBeVisible();
}
const programmatic = async (page: Page, name: string) =>
  page
    .getByRole('button', { name, exact: true, includeHidden: true })
    .evaluate((node: HTMLButtonElement) => node.click());

test('native controls and blocked states retain their actual DOM identity and controlled values', async ({
  page,
}) => {
  await gallery(page);
  const input = page.getByLabel('Controlled input');
  await input.evaluate((node) => node.setAttribute('data-retained-node', 'yes'));
  await input.fill('Changed input');
  await expect(input).toHaveValue('Changed input');
  await expect(input).toHaveAttribute('data-retained-node', 'yes');
  await page.getByLabel('Controlled textarea').fill('Changed textarea');
  await expect(page.getByLabel('Controlled textarea')).toHaveValue('Changed textarea');
  for (const [name, value] of [
    ['Read-only input', 'Read-only value'],
    ['Read-only textarea', 'Read-only text'],
    ['Working input', 'Retained pending value'],
  ]) {
    const control = page.getByLabel(name);
    await control.focus();
    await control.press('End');
    await control.pressSequentially('x');
    await expect(control).toHaveValue(value);
  }
  await expect(page.getByLabel('Disabled input')).toBeDisabled();
  await expect(page.getByLabel('Disabled textarea')).toBeDisabled();
  await expect(page.getByLabel('Working input')).toHaveAttribute('aria-busy', 'true');
  const action = page.getByRole('button', { name: 'Ready action', exact: true });
  await action.focus();
  await action.press('Enter');
  await action.press('Space');
  await expect(page.getByLabel('Action count')).toHaveText('2');
  await page.getByRole('button', { name: 'Disabled action', exact: true }).click({ force: true });
  await page.getByRole('button', { name: 'Working action', exact: true }).click({ force: true });
  await page.getByRole('link', { name: 'Disabled child action' }).click({ force: true });
  await expect(page.getByLabel('Action count')).toHaveText('2');
  expect(new URL(page.url()).hash).toBe('');
  const check = page.getByRole('checkbox', { name: 'Controlled checkbox' });
  await check.click();
  await expect(check).toHaveAttribute('aria-checked', 'true');
  await check.press('Space');
  await expect(check).toHaveAttribute('aria-checked', 'false');
  const toggle = page.getByRole('switch', { name: 'Controlled switch' });
  await toggle.click();
  await expect(toggle).toHaveAttribute('aria-checked', 'true');
  await toggle.press('Space');
  await expect(toggle).toHaveAttribute('aria-checked', 'false');
  for (const [role, name] of [
    ['checkbox', 'Read-only checkbox'],
    ['switch', 'Read-only switch'],
  ] as const) {
    const control = page.getByRole(role, { name });
    await control.click();
    await control.press('Space');
    await expect(control).toHaveAttribute('aria-checked', 'true');
    await expect(control).toHaveAttribute('aria-readonly', 'true');
  }
  await expect(page.getByRole('checkbox', { name: 'Mixed checkbox' })).toHaveAttribute(
    'aria-checked',
    'mixed',
  );
});

test('Radix tabs skip disabled items and preserve manual versus automatic activation', async ({
  page,
}) => {
  await gallery(page);
  const first = page.getByRole('tab', { name: 'First', exact: true });
  const second = page.getByRole('tab', { name: 'Second', exact: true });
  await first.focus();
  await first.press('ArrowRight');
  await expect(second).toBeFocused();
  await expect(first).toHaveAttribute('aria-selected', 'true');
  await second.press('Space');
  await expect(second).toHaveAttribute('aria-selected', 'true');
  await expect(page.getByRole('tabpanel').filter({ hasText: 'Second panel' })).toBeVisible();
  await second.press('Home');
  await expect(first).toBeFocused();
  await first.press('End');
  await expect(second).toBeFocused();
  const north = page.getByRole('tab', { name: 'North', exact: true });
  const south = page.getByRole('tab', { name: 'South', exact: true });
  await north.focus();
  await north.press('ArrowDown');
  await expect(south).toBeFocused();
  await expect(south).toHaveAttribute('aria-selected', 'true');
});

test('stock Select retains modal-choice keyboard behavior and actual close/focus paths', async ({
  page,
}) => {
  await gallery(page);
  const trigger = page.getByRole('combobox', {
    name: 'Controlled choice',
    exact: true,
    includeHidden: true,
  });
  await trigger.click();
  const list = page.getByRole('listbox');
  await expect(list).toBeVisible();
  await expect(page.getByRole('option', { name: 'Alpha', exact: true })).toBeFocused();
  await expect(list).not.toHaveAttribute('aria-modal', 'true');
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await page.keyboard.press('Tab');
  expect(await list.evaluate((node) => node.contains(document.activeElement))).toBe(true);
  await page.keyboard.press('Shift+Tab');
  expect(await list.evaluate((node) => node.contains(document.activeElement))).toBe(true);
  await page.keyboard.press('ArrowDown');
  await expect(page.getByRole('option', { name: 'Beta', exact: true })).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(list).toBeHidden();
  await expect(trigger).toBeFocused();
  await expect(page.getByLabel('Choice and change count')).toHaveText('beta:1');
  await trigger.click();
  await page.keyboard.press('a');
  await expect(page.getByRole('option', { name: 'Alpha', exact: true })).toBeFocused();
  // Stock typeahead treats Space as a search character until its search clears.
  await page.keyboard.press('Enter');
  await expect(list).toBeHidden();
  await expect(trigger).toBeFocused();
  await expect(page.getByLabel('Choice and change count')).toHaveText('alpha:2');
  await trigger.click();
  await page.keyboard.press('Space');
  await expect(list).toBeHidden();
  await expect(trigger).toBeFocused();
  await trigger.click();
  await page.keyboard.press('End');
  await expect(
    page.getByRole('option', { name: 'Long choice 12: retained full accessible name' }),
  ).toBeFocused();
  await page.keyboard.press('Home');
  await expect(page.getByRole('option', { name: 'Alpha', exact: true })).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(list).toBeHidden();
  await expect(trigger).toBeFocused();
  await trigger.click();
  await page.mouse.click(1, 1);
  await expect(list).toBeHidden();
  await expect(trigger).toBeFocused();
  await programmatic(page, 'Open select programmatically');
  await expect(list).toBeVisible();
  await programmatic(page, 'Close select programmatically');
  await expect(list).toBeHidden();
  await expect(trigger).toBeFocused();
  await programmatic(page, 'Open select programmatically');
  await programmatic(page, 'Remove select opener');
  await expect(trigger).toHaveCount(0);
  await programmatic(page, 'Close select programmatically');
  await expect(list).toBeHidden();
  await expect(
    page.getByRole('button', { name: 'Open select programmatically', exact: true }),
  ).toBeFocused();
  await expect(page.locator('#legacy')).not.toHaveAttribute('aria-hidden', 'true');
  await expect(page.locator('body')).not.toHaveAttribute('data-scroll-locked', '1');
});

test('host-owned busy close veto and read-only value admission remain independent of Radix navigation', async ({
  page,
}) => {
  await gallery(page);
  const trigger = page.getByRole('combobox', {
    name: 'Controlled choice',
    exact: true,
    includeHidden: true,
  });
  const list = page.getByRole('listbox');
  await trigger.click();
  await programmatic(page, 'Start select work');
  await expect(trigger).toHaveAttribute('aria-busy', 'true');
  await expect(list).toHaveAttribute('aria-busy', 'true');
  await expect(page.getByText('Working choices', { exact: true })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(list).toBeVisible();
  await page.mouse.click(1, 1);
  await expect(list).toBeVisible();
  await programmatic(page, 'Close select programmatically');
  await expect(list).toBeVisible();
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press('Enter');
  await expect(list).toBeVisible();
  await expect(page.getByLabel('Choice and change count')).toHaveText('alpha:0');
  await programmatic(page, 'Finish select work');
  await programmatic(page, 'Close select programmatically');
  await expect(list).toBeHidden();
  await expect(trigger).toBeFocused();
  await programmatic(page, 'Make choice read-only');
  await trigger.click();
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press('Enter');
  await expect(list).toBeHidden();
  await expect(page.getByLabel('Choice and change count')).toHaveText('alpha:0');
  await expect(trigger).toHaveAttribute('aria-readonly', 'true');
});

for (const width of [320, 390, 1440])
  for (const theme of ['light', 'dark'] as const) {
    test(`primitives ${width} ${theme}: tokens, portal geometry and state captures`, async ({
      page,
    }, info) => {
      await page.setViewportSize({ width, height: 900 });
      await gallery(page, theme);
      const failures: string[] = [];
      page.on('pageerror', (error) => failures.push(error.message));
      const scope = page.getByRole('region', { name: 'Real shadcn primitives' });
      const button = page.getByRole('button', { name: 'Ready action', exact: true });
      await expect(button).toHaveCSS(
        'background-color',
        rgb(tokens.browser.surface.selection[theme]),
      );
      await expect(button).toHaveCSS('color', rgb(tokens.browser.color['selected-text'][theme]));
      await expect(button).toHaveCSS('font-size', tokens.browser.metric['command-size']);
      await expect(button).toHaveCSS('border-radius', '0px');
      await expect(button).toHaveCSS('box-shadow', 'none');
      await page.keyboard.press('Tab');
      await button.focus();
      expect(await button.evaluate((node) => node.matches(':focus-visible'))).toBe(true);
      await expect(button).toHaveCSS('outline-width', tokens.browser.metric['focus-width']);
      await expect(button).toHaveCSS('outline-color', rgb(tokens.browser.color.focus[theme]));
      for (const name of ['Controlled input', 'Controlled textarea']) {
        const field = page.getByLabel(name);
        await expect(field).toHaveCSS('border-color', rgb(tokens.browser.color.edge[theme]));
        await expect(field).toHaveCSS('border-width', tokens.browser.metric['rule-width']);
      }
      await scope.screenshot({ path: info.outputPath(`primitives-${width}-${theme}-states.png`) });
      const trigger = page.getByRole('combobox', {
        name: 'Controlled choice',
        exact: true,
        includeHidden: true,
      });
      await trigger.scrollIntoViewIfNeeded();
      await trigger.click();
      const list = page.getByRole('listbox');
      await expect(list).toBeVisible();
      expect(
        await list.evaluate(
          (node) => !!node.closest('[data-tmt-ui-portal]') && !!node.closest('[data-tmt-ui]'),
        ),
      ).toBe(true);
      await expect(list).toHaveCSS('background-color', rgb(tokens.browser.surface.sheet[theme]));
      await expect(list).toHaveCSS('color', rgb(tokens.browser.color.text[theme]));
      await expect(list).toHaveCSS('z-index', tokens.browser.ui['z-layer']);
      const rect = await list.boundingBox();
      expect(rect).not.toBeNull();
      expect(rect!.x).toBeGreaterThanOrEqual(0);
      expect(rect!.x + rect!.width).toBeLessThanOrEqual(width + 1);
      expect(rect!.y).toBeGreaterThanOrEqual(0);
      expect(rect!.y + rect!.height).toBeLessThanOrEqual(901);
      await page.screenshot({ path: info.outputPath(`primitives-${width}-${theme}-select.png`) });
      await programmatic(page, 'Start select work');
      await page.setViewportSize({ width, height: 420 });
      await expect(list).toBeVisible();
      await expect
        .poll(async () => {
          const r = await list.boundingBox();
          return r ? r.y + r.height : Infinity;
        })
        .toBeLessThanOrEqual(421);
      await page.screenshot({
        path: info.outputPath(`primitives-${width}-${theme}-reduced-viewport.png`),
      });
      // Real Chromium visual-viewport resize, not a replacement window.visualViewport object.
      const cdp = await page.context().newCDPSession(page);
      try {
        await cdp.send('Emulation.setPageScaleFactor', { pageScaleFactor: 1.25 });
        await expect
          .poll(() => page.evaluate(() => window.visualViewport?.scale ?? 1))
          .toBeGreaterThan(1);
        await expect
          .poll(async () =>
            list.evaluate((node) => {
              const viewport = window.visualViewport;
              if (!viewport) return false;
              const rect = node.getBoundingClientRect();
              return (
                rect.left >= viewport.offsetLeft - 1 &&
                rect.right <= viewport.offsetLeft + viewport.width + 1 &&
                rect.top >= viewport.offsetTop - 1 &&
                rect.bottom <= viewport.offsetTop + viewport.height + 1
              );
            }),
          )
          .toBe(true);
        await page.screenshot({
          path: info.outputPath(`primitives-${width}-${theme}-visual-viewport.png`),
        });
      } finally {
        await cdp.send('Emulation.setPageScaleFactor', { pageScaleFactor: 1 });
        await cdp.detach();
      }
      await programmatic(page, 'Finish select work');
      await programmatic(page, 'Close select programmatically');
      expect(failures).toEqual([]);
    });
  }
