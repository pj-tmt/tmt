import { expect, test, type Page } from '@playwright/test';

declare global {
  interface Window {
    __chipsErrors?: string[];
  }
}

const path = '/test/browser-chips.html';
const add = (page: Page) => page.getByRole('button', { name: 'Add recipient', exact: true });
const field = (page: Page) => page.getByRole('combobox', { name: 'Search agents' });
const remove = (page: Page, name: string) =>
  page.getByRole('button', { name: `Remove ${name}`, exact: true });

for (const [width, theme] of [
  [1440, 'light'],
  [390, 'dark'],
] as const) {
  test(`chips keyboard, values, focus and wrapping at ${width} ${theme}`, async ({
    page,
  }, info) => {
    await page.setViewportSize({ width, height: 800 });
    await page.goto(path);
    await page.evaluate((theme) => {
      document.documentElement.dataset.theme = theme;
    }, theme);
    await expect(remove(page, '@tmt-ux')).toBeVisible();
    await expect(page.locator('#visibility')).toHaveText('[]');
    await page.screenshot({ path: info.outputPath(`chips-${width}-${theme}.png`), fullPage: true });
    await add(page).click();
    await expect(field(page)).toBeFocused();
    await expect(page.getByRole('option', { name: '@tmt-ux UX', exact: true })).toHaveCount(0);
    await expect(page.getByRole('option', { name: '@unavailable' })).toBeDisabled();
    await field(page).fill('col');
    await expect(page.getByRole('option')).toHaveCount(2);
    await field(page).press('ArrowDown');
    const active = await field(page).getAttribute('aria-activedescendant');
    await expect(page.locator(`[id="${active}"]`)).toHaveText('@colab-1Colab');
    await page.screenshot({
      path: info.outputPath(`chips-completion-${width}-${theme}.png`),
      fullPage: true,
    });
    await field(page).press('Enter');
    await expect(remove(page, '@colab-1')).toBeVisible();
    await expect(add(page)).toBeFocused();
    await expect(page.locator('#events')).toHaveText('add:one:true');
    await expect(page.locator('#visibility')).toHaveText('[true,false]');
    await add(page).click();
    await field(page).fill('text');
    await field(page).press('Backspace');
    await expect(field(page)).toHaveValue('tex');
    await expect(remove(page, '@colab-1')).toBeVisible();
    await field(page).fill('');
    await field(page).press('Backspace');
    await expect(remove(page, '@colab-1')).toHaveCount(0);
    await expect(field(page)).toBeFocused();
    await field(page).press('Escape');
    await expect(add(page)).toBeFocused();
    await remove(page, '@tmt-ux').click();
    await expect(page.getByText('Stays on the page, no one is asked')).toBeVisible();
    await expect(add(page)).toBeFocused();
    await add(page).click();
    await field(page).press('Tab');
    await expect(page.getByRole('textbox', { name: 'After recipients' })).toBeFocused();
    await expect(field(page)).toHaveCount(0);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
      true,
    );
  });
  for (const mode of ['empty', 'cap', 'long', 'loading', 'failed', 'disabled']) {
    test(`chips ${mode} at ${width} ${theme}`, async ({ page }, info) => {
      await page.setViewportSize({ width, height: 800 });
      await page.goto(`${path}?case=${mode}`);
      await page.evaluate((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      if (mode === 'cap') {
        await expect(add(page)).toBeDisabled();
        await expect(page.getByText('At most 8')).toBeVisible();
      }
      if (mode === 'empty')
        await expect(page.getByText('Stays on the page, no one is asked')).toBeVisible();
      if (mode === 'loading' || mode === 'failed' || mode === 'disabled')
        await expect(add(page)).toBeDisabled();
      if (mode === 'long') {
        await expect(remove(page, `@${'long-name-'.repeat(14)}`)).toBeVisible();
        const label = page.locator('.tmt-ui-chip-label').first();
        if (width === 390)
          expect(await label.evaluate((node) => node.scrollWidth > node.clientWidth)).toBe(true);
      }
      await page.screenshot({
        path: info.outputPath(`chips-${mode}-${width}-${theme}.png`),
        fullPage: true,
      });
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
      if (mode !== 'empty') {
        const geometry = await page
          .locator('.tmt-ui-chip')
          .first()
          .evaluate((node) => {
            const style = getComputedStyle(node);
            return [style.borderTopWidth, style.borderRadius, style.boxShadow];
          });
        expect(geometry).toEqual(['1px', '0px', 'none']);
      }
    });
  }
}

test('chips native close, outside press, removal and StrictMode report once', async ({ page }) => {
  await page.goto(path);
  await add(page).click();
  await expect(page.locator('#visibility')).toHaveText('[true]');
  await page.getByRole('dialog').evaluate((node: HTMLDialogElement) => node.close());
  await expect(page.locator('#visibility')).toHaveText('[true,false]');
  await expect(add(page)).toBeFocused();
  await add(page).click();
  await page.mouse.click(5, 5);
  await expect(page.locator('#visibility')).toHaveText('[true,false,true,false]');
  await expect(page.locator('#events')).toBeEmpty();
  await add(page).click();
  await page.locator('#unmount').evaluate((node: HTMLButtonElement) => node.click());
  await expect(page.locator('#visibility')).toHaveText('[true,false,true,false,true,false]');
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(page.getByRole('textbox', { name: 'After recipients' })).toBeFocused();
});

test('chips ignore composing, modified, prevented and untrusted effect keys', async ({ page }) => {
  await page.goto(path);
  await add(page).click();
  await field(page).fill('');
  await field(page).evaluate((node) => {
    node.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'Backspace', bubbles: true, isComposing: true }),
    );
    node.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
  });
  await field(page).press('Control+Backspace');
  await expect(remove(page, '@tmt-ux')).toBeVisible();
  await field(page).press('Shift+Escape');
  await field(page).press('Shift+Enter');
  await field(page).press('Shift+Backspace');
  await expect(remove(page, '@tmt-ux')).toBeVisible();
  await expect(field(page)).toBeFocused();
  await field(page).evaluate((node) =>
    node.addEventListener('keydown', (event) => event.preventDefault(), { once: true }),
  );
  await field(page).press('Enter');
  await expect(page.locator('#events')).toBeEmpty();
  await field(page).press('Escape');
  await expect(add(page)).toBeFocused();
});

test('chips remove follows surviving focus and capacity updates after adding', async ({ page }) => {
  await page.goto(path);
  await add(page).click();
  await page.getByRole('option', { name: '@colab-lead Colab', exact: true }).click();
  await remove(page, '@tmt-ux').click();
  await expect(remove(page, '@colab-lead')).toBeFocused();
  await page.goto(`${path}?case=cap`);
  await remove(page, '@agent-7').click();
  await expect(remove(page, '@agent-6')).toBeFocused();
  await expect(add(page)).toBeEnabled();
  await add(page).click();
  await field(page).fill('@agent-7');
  await field(page).press('Enter');
  await expect(add(page)).toBeDisabled();
  await expect(remove(page, '@agent-7')).toBeFocused();
});

test('chips closes once when host disables it or removes its anchor', async ({ page }) => {
  await page.addInitScript(() => {
    const errors: string[] = [];
    Object.defineProperty(window, '__chipsErrors', { value: errors });
    window.addEventListener('error', (event) => errors.push(event.message));
  });
  await page.goto(path);
  await add(page).click();
  await page.locator('#disable').evaluate((node: HTMLButtonElement) => node.click());
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(page.locator('#visibility')).toHaveText('[true,false]');
  await page.goto(path);
  await add(page).click();
  await add(page).evaluate((node) => {
    node.style.display = 'none';
  });
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(page.locator('#visibility')).toHaveText('[true,false]');
  await page.evaluate(
    () =>
      new Promise<void>((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
      ),
  );
  expect(await page.evaluate(() => window.__chipsErrors)).toEqual([]);
});

test('chips keyboard trigger opens at either end without moving focus out of the input', async ({
  page,
}) => {
  await page.goto(path);
  await add(page).focus();
  await add(page).press('ArrowUp');
  await expect(field(page)).toBeFocused();
  const last = await field(page).getAttribute('aria-activedescendant');
  await expect(page.locator(`[id="${last}"]`)).toHaveText('@agent-7');
  await field(page).press('Escape');
  await add(page).press('ArrowDown');
  const first = await field(page).getAttribute('aria-activedescendant');
  await expect(page.locator(`[id="${first}"]`)).toHaveText('@colab-leadColab');
  await expect(field(page)).toBeFocused();
});
