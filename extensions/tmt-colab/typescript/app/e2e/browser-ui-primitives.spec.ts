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
    ['Working textarea', 'Retained pending text'],
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
  await expect(page.getByLabel('Working textarea')).toHaveAttribute('aria-busy', 'true');
  await expect(page.getByLabel('Invalid input')).toHaveAttribute('aria-invalid', 'true');
  await expect(page.getByText('Invalid: supply a valid value.', { exact: true })).toBeVisible();
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
    await expect(control).toHaveAttribute('aria-checked', 'true');
    await control.press('Space');
    await expect(control).toHaveAttribute('aria-checked', 'true');
    await expect(control).toHaveAttribute('aria-readonly', 'true');
  }
  for (const [role, name] of [
    ['checkbox', 'Disabled checkbox'],
    ['checkbox', 'Working checkbox'],
    ['switch', 'Disabled switch'],
    ['switch', 'Working switch'],
  ] as const) {
    const control = page.getByRole(role, { name });
    await expect(control).toBeDisabled();
    await control.click({ force: true });
    await expect(control).toHaveAttribute('aria-checked', 'false');
    if (name.startsWith('Working')) await expect(control).toHaveAttribute('aria-busy', 'true');
  }
  await expect(page.getByRole('checkbox', { name: 'Mixed checkbox' })).toHaveAttribute(
    'aria-checked',
    'mixed',
  );
});

test('slotted disabled capture handlers are inert before child effects and preserve enabled events', async ({
  page,
}) => {
  await gallery(page);
  const enabled = page.getByRole('link', { name: 'Enabled capture child' });
  const childEffects = page.getByLabel('Enabled child capture effects');
  const hostEffects = page.getByLabel('Enabled host capture effects');
  await enabled.click();
  await expect(childEffects).toHaveText('1');
  await expect(hostEffects).toHaveText('1');
  await expect(page.getByLabel('Original event and refs forwarded')).toHaveText('true');
  await enabled.press('Enter');
  await expect(childEffects).toHaveText('3');
  await expect(hostEffects).toHaveText('3');
  await enabled.press('Space');
  await expect(childEffects).toHaveText('4');
  await expect(hostEffects).toHaveText('4');
  await expect(page.getByLabel('Original event and refs forwarded')).toHaveText('true');
  for (const name of [
    'Disabled capture child',
    'Disabled Slottable children',
    'Disabled Slottable render',
  ]) {
    const blocked = page.getByRole('link', { name });
    await expect(blocked).toHaveAttribute('data-child-ref', 'yes');
    await expect(blocked).toHaveAttribute('data-host-ref', 'yes');
    await blocked.evaluate((node) => node.setAttribute('data-retained-node', 'yes'));
    await blocked.scrollIntoViewIfNeeded();
    // Mouse input remains trusted; bypass only locator's aria-disabled actionability wait.
    const box = await blocked.boundingBox();
    expect(box).not.toBeNull();
    await page.mouse.click(box!.x + box!.width / 2, box!.y + box!.height / 2);
    await expect.soft(page.getByLabel('Blocked capture effects')).toHaveText('0', { timeout: 500 });
    await blocked.press('Enter');
    await expect.soft(page.getByLabel('Blocked capture effects')).toHaveText('0', { timeout: 500 });
    await blocked.press('Space');
    await expect.soft(page.getByLabel('Blocked capture effects')).toHaveText('0', { timeout: 500 });
    await expect(blocked).toHaveAttribute('data-retained-node', 'yes');
  }
  expect(new URL(page.url()).hash).toBe('');
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

for (const width of [320, 390, 1440])
  for (const theme of ['light', 'dark'] as const) {
    test(`primitives ${width} ${theme}: tokens, width containment and state captures`, async ({
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
        await expect(field).toHaveCSS('border-color', rgb(tokens.browser.surface.rule[theme]));
        await expect(field).toHaveCSS('border-width', tokens.browser.metric['rule-width']);
      }
      const separator = scope.getByRole('separator');
      await expect(separator).toHaveCSS('height', tokens.browser.metric['rule-width']);
      await expect(separator).toHaveCSS(
        'background-color',
        rgb(tokens.browser.surface.rule[theme]),
      );
      const alert = scope.getByRole('alert').filter({ hasText: 'Ready information' });
      await expect(alert).toHaveCSS('background-color', rgb(tokens.browser.surface.sheet[theme]));
      await expect(alert).toHaveCSS('border-radius', '0px');
      const tab = page.getByRole('tab', { name: 'First', exact: true });
      await tab.focus();
      await expect(tab).toHaveCSS('outline-width', tokens.browser.metric['focus-width']);
      await expect(tab).toHaveCSS('outline-color', rgb(tokens.browser.color.focus[theme]));
      await expect(tab).toHaveCSS('border-radius', '0px');
      await expect(page.getByRole('tab', { name: 'Second', exact: true })).toHaveCSS(
        'background-color',
        'rgba(0, 0, 0, 0)',
      );
      await scope.screenshot({ path: info.outputPath(`primitives-${width}-${theme}-states.png`) });
      expect(
        await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth),
      ).toBe(true);
      expect(failures).toEqual([]);
    });
  }

/** Toggles the three form-reset controls on, checks FormData, then resets the named form. */
async function toggleAndReset(page: Page) {
  await gallery(page);
  const form = page.getByRole('form', { name: 'External reset form' });
  const controls = {
    inForm: page.getByRole('checkbox', { name: 'In-form checkbox' }),
    externalSwitch: page.getByRole('switch', { name: 'External-form switch' }),
    externalCheck: page.getByRole('checkbox', { name: 'External-form checkbox' }),
  };
  const submitted = () =>
    form.evaluate((node) => [...new FormData(node as HTMLFormElement).keys()].sort());
  for (const control of Object.values(controls)) {
    await control.click();
    await expect(control).toHaveAttribute('aria-checked', 'true');
  }
  expect(await submitted()).toEqual(['external-check', 'external-switch', 'in-form-check']);
  await form.getByRole('button', { name: 'Reset external form' }).click();
  return { controls, submitted };
}

test('a form reset clears every named control and the in-form and Switch visuals', async ({
  page,
}) => {
  const { controls, submitted } = await toggleAndReset(page);
  for (const name of ['in-form-check', 'external-switch', 'external-check'])
    await expect(page.locator(`input[name="${name}"]`)).not.toBeChecked();
  expect(await submitted()).toEqual([]);
  await expect(controls.inForm).toHaveAttribute('aria-checked', 'false');
  await expect(controls.externalSwitch).toHaveAttribute('aria-checked', 'false');
});

test('an external-form Checkbox shows unchecked after a reset of the form it names', async ({
  page,
}) => {
  const { controls } = await toggleAndReset(page);
  await expect(controls.externalCheck).toHaveAttribute('aria-checked', 'false');
});

test('a Checkbox naming another form keeps its state when this form resets', async ({ page }) => {
  await gallery(page);
  const other = page.getByRole('checkbox', { name: 'Other-form checkbox' });
  await other.click();
  await expect(other).toHaveAttribute('aria-checked', 'true');
  await page
    .getByRole('form', { name: 'External reset form' })
    .getByRole('button', { name: 'Reset external form' })
    .click();
  await expect(other).toHaveAttribute('aria-checked', 'true');
  await expect(page.locator('input[name="other-check"]')).toBeChecked();
});
