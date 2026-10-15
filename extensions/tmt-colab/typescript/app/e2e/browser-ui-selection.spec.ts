import { test, expect, type Page } from '@playwright/test';
import { readFileSync } from 'node:fs';
const tokens = JSON.parse(
  readFileSync(new URL('../../../../../design/tokens/tokens.json', import.meta.url), 'utf8'),
);
const rgb = (hex: string) =>
  `rgb(${[1, 3, 5].map((offset) => parseInt(hex.slice(offset, offset + 2), 16)).join(', ')})`;
test.beforeEach(({ page }) => {
  page.on('pageerror', (error) => {
    throw error;
  });
});
async function gallery(page: Page) {
  await page.goto('/test/browser-ui-foundation.html');
  await page.getByRole('button', { name: 'Load isolated UI CSS' }).click();
  await expect(page.locator('[data-css-loaded=true]')).toHaveText('UI CSS loaded');
  await expect(page.getByRole('heading', { name: 'Slice4 selection qualification' })).toBeVisible();
}
test('original Enter and pointer selection each commit once without Send', async ({ page }) => {
  await gallery(page);
  const input = page.getByRole('combobox', { name: 'Selection search' });
  await input.focus();
  await expect(input).toHaveAttribute('aria-expanded', 'true');
  await expect(page.getByRole('option', { name: 'Alpha', exact: true })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  await input.press('Enter');
  await expect(page.getByLabel('Selection value', { exact: true })).toHaveText('alpha');
  await expect(page.getByLabel('Selection callback count')).toHaveText('1');
  await expect(page.getByLabel('Selection original event')).toHaveText('keydown:true:true');
  await expect(page.getByLabel('Selection send count')).toHaveText('0');
  await input.press('ArrowDown');
  await page.getByRole('option', { name: 'Beta', exact: true }).click();
  await expect(page.getByLabel('Selection value', { exact: true })).toHaveText('beta');
  await expect(page.getByLabel('Selection callback count')).toHaveText('2');
  await expect(page.getByLabel('Selection original event')).toHaveText('click:true:true');
  await expect(page.getByLabel('Selection send count')).toHaveText('0');
});
test('Tab and reverse Tab admit once, preserve input identity and continue native focus', async ({
  page,
}) => {
  await gallery(page);
  const input = page.getByRole('combobox', { name: 'Selection search' });
  await input.evaluate((node) => node.setAttribute('data-retained-node', 'yes'));
  await input.focus();
  await expect(page.getByRole('option', { name: 'Alpha', exact: true })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  await input.press('Tab');
  await expect(page.getByRole('button', { name: 'After selection', exact: true })).toBeFocused();
  await expect(input).toHaveAttribute('aria-expanded', 'false');
  await expect(input).toHaveAttribute('data-retained-node', 'yes');
  await expect(page.getByLabel('Selection callback count')).toHaveText('1');
  await expect(page.getByLabel('Selection original event')).toHaveText('keydown:true:true');
  await input.focus();
  await expect(input).toHaveAttribute('aria-expanded', 'true');
  await input.press('Shift+Tab');
  await expect(page.getByRole('button', { name: 'Before selection', exact: true })).toBeFocused();
  await expect(page.getByLabel('Selection callback count')).toHaveText('2');
  await expect(input).toHaveAttribute('data-retained-node', 'yes');
});
test('Tab close veto preserves actual open state and makes no selection callback', async ({
  page,
}) => {
  await gallery(page);
  await page.getByRole('button', { name: 'Close veto off', exact: true }).click();
  const input = page.getByRole('combobox', { name: 'Selection search' });
  await input.focus();
  await input.press('Tab');
  await expect(input).toBeFocused();
  await expect(input).toHaveAttribute('aria-expanded', 'true');
  await expect(page.getByLabel('Selection callback count')).toHaveText('0');
  await input.press('Shift+Tab');
  await expect(input).toBeFocused();
  await expect(page.getByLabel('Selection callback count')).toHaveText('0');
});

test('combobox outside pointer admits the underlying action and keeps its native focus', async ({
  page,
}) => {
  await gallery(page);
  await page.getByRole('combobox', { name: 'Selection search' }).focus();
  const outside = page.getByRole('button', { name: 'Selection outside action', exact: true });
  await outside.click();
  await expect(page.getByLabel('Selection outside count')).toHaveText('1');
  await expect(outside).toBeFocused();
  await expect(page.getByRole('combobox', { name: 'Selection search' })).toHaveAttribute(
    'aria-expanded',
    'false',
  );
});
test('chips first outside pointer dismisses only; a subsequent press activates the action', async ({
  page,
}) => {
  await gallery(page);
  const input = page.getByRole('combobox', { name: 'Chips search' });
  await input.focus();
  await expect(input).toHaveAttribute('aria-expanded', 'true');
  const outside = page.getByRole('button', { name: 'Chips outside action', exact: true });
  const target = await outside.boundingBox();
  expect(target).not.toBeNull();
  expect(target!.y).toBeGreaterThanOrEqual(0);
  expect(target!.y + target!.height / 2).toBeLessThan(page.viewportSize()!.height);
  // A real coordinate press must hit the shield; locator.click waits for it to vanish.
  await page.mouse.click(target!.x + target!.width / 2, target!.y + target!.height / 2);
  await expect(page.getByLabel('Chips outside count')).toHaveText('0');
  await expect(input).toHaveAttribute('aria-expanded', 'false');
  await outside.click();
  await expect(page.getByLabel('Chips outside count')).toHaveText('1');
});

test('shield veto always consumes the real first press, then Escape returns focus without reopening', async ({
  page,
}) => {
  await gallery(page);
  const input = page.getByRole('combobox', { name: 'Chips search' });
  await input.click();
  await page
    .getByRole('button', { name: 'Chips busy off', exact: true })
    .evaluate((button) => (button as HTMLButtonElement).click());
  await expect(input).toHaveAttribute('aria-busy', 'true');
  const outside = page.getByRole('button', { name: 'Chips outside action', exact: true });
  const box = (await outside.boundingBox())!;
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
  await expect(page.getByLabel('Chips outside count')).toHaveText('0');
  await expect(input).toHaveAttribute('aria-expanded', 'true');
  await expect(page.locator('[data-slot=popover-outside-shield]')).toHaveCount(1);
  await input.press('Tab');
  await expect(input).toBeFocused();
  await input.press('Escape');
  await expect(input).toHaveAttribute('aria-expanded', 'true');
  await expect(page.getByLabel('Chips add count')).toHaveText('0');
  await page
    .getByRole('button', { name: 'Chips busy on', exact: true })
    .evaluate((button) => (button as HTMLButtonElement).click());
  await input.press('Escape');
  await expect(input).toHaveAttribute('aria-expanded', 'false');
  await expect(input).toBeFocused();
  await expect(page.locator('[data-slot=popover-outside-shield]')).toHaveCount(0);
  expect(await page.evaluate(() => document.body.style.pointerEvents)).toBe('');
});

test('external input remains hit-testable/editable with stable DOM; caps and removal focus stay host-owned', async ({
  page,
}) => {
  await gallery(page);
  const input = page.getByRole('combobox', { name: 'Chips search' });
  await input.click();
  await input.evaluate((node) => node.setAttribute('data-stable-chip-input', 'yes'));
  await input.click();
  await input.pressSequentially('Be');
  await expect(input).toHaveValue('Be');
  await expect(input).toBeFocused();
  await expect(input).toHaveAttribute('data-stable-chip-input', 'yes');
  await expect(page.getByRole('option', { name: 'Beta', exact: true })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  await input.press('Enter');
  await expect(page.getByLabel('Chips add count')).toHaveText('1');
  await expect(page.getByLabel('Chips event')).toHaveText('keydown:true');
  await expect(
    page.getByRole('status').filter({ hasText: 'Capacity reached: remove a chip to add another' }),
  ).toBeVisible();
  await input.press('ArrowDown');
  await input.press('Enter');
  await expect(page.getByLabel('Chips add count')).toHaveText('1');
  await input.press('Escape');
  await expect(input).toHaveAttribute('aria-expanded', 'false');
  await expect(input).toBeFocused();
  const alpha = page.getByRole('button', { name: 'Remove Alpha', exact: true });
  await alpha.press('Enter');
  await expect(page.getByRole('button', { name: 'Remove Beta', exact: true })).toBeFocused();
  await expect(page.getByLabel('Chips remove count')).toHaveText('1');
  await input.focus();
  await input.press('Backspace');
  await expect(page.getByLabel('Chips remove count')).toHaveText('2');
  await expect(input).toBeFocused();
  await expect(input).toHaveAttribute('data-stable-chip-input', 'yes');
  await expect(page.getByLabel('Chips Send count')).toHaveText('0');
});

test('blocked, modified, prevented, composing and removed-option paths never commit or send', async ({
  page,
}) => {
  await gallery(page);
  const input = page.getByRole('combobox', { name: 'Selection search' });
  await input.focus();
  await input.press('Control+Enter');
  await input.press('Shift+Enter');
  await input.dispatchEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true });
  await expect(page.getByLabel('Selection callback count')).toHaveText('0');
  await page
    .getByRole('button', { name: 'Prevent selection keys off' })
    .evaluate((button) => (button as HTMLButtonElement).click());
  await input.press('Enter');
  await expect(page.getByLabel('Selection callback count')).toHaveText('0');
  await page
    .getByRole('button', { name: 'Prevent selection keys on' })
    .evaluate((button) => (button as HTMLButtonElement).click());
  const session = await page.context().newCDPSession(page);
  await session.send('Input.imeSetComposition', {
    text: 'preedit',
    selectionStart: 7,
    selectionEnd: 7,
  });
  await session.send('Input.dispatchKeyEvent', {
    type: 'keyDown',
    key: 'Enter',
    code: 'Enter',
    windowsVirtualKeyCode: 229,
  });
  await expect(page.getByLabel('Selection key detail')).toHaveText('true:true:229');
  await expect(page.getByLabel('Selection callback count')).toHaveText('0');
  await session.send('Input.imeSetComposition', { text: '', selectionStart: 0, selectionEnd: 0 });
  await session.detach();
  await input.fill('');
  await page
    .getByRole('button', { name: 'Selection read-only off' })
    .evaluate((button) => (button as HTMLButtonElement).click());
  await input.press('Enter');
  await input.pressSequentially('x');
  await expect(input).toHaveValue('');
  await expect(page.getByLabel('Selection callback count')).toHaveText('0');
  await page
    .getByRole('button', { name: 'Selection read-only on' })
    .evaluate((button) => (button as HTMLButtonElement).click());
  await page
    .getByRole('button', { name: 'Remove active option off' })
    .evaluate((button) => (button as HTMLButtonElement).click());
  await expect(page.getByRole('option', { name: 'Alpha', exact: true })).toHaveCount(0);
  await expect(page.getByRole('option', { name: 'Beta', exact: true })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  await input.press('Enter');
  await expect(page.getByLabel('Selection value', { exact: true })).toHaveText('beta');
  await expect(page.getByLabel('Selection callback count')).toHaveText('1');
  await expect(page.getByLabel('Selection send count')).toHaveText('0');
});

test('raw host unmount and StrictMode leave no shield, pointer suppression or focus guards', async ({
  page,
}) => {
  await gallery(page);
  const guards = page.locator('[data-radix-focus-guard]');
  const before = await guards.count();
  await page.getByRole('combobox', { name: 'Chips search' }).click();
  await expect(page.locator('[data-slot=popover-outside-shield]')).toHaveCount(1);
  await page
    .getByRole('button', { name: 'Chips mounted on' })
    .evaluate((button) => (button as HTMLButtonElement).click());
  await expect(page.getByRole('combobox', { name: 'Chips search' })).toHaveCount(0);
  await expect(page.locator('[data-slot=popover-outside-shield]')).toHaveCount(0);
  await expect(guards).toHaveCount(before);
  await expect(page.getByRole('button', { name: 'After chips', exact: true })).toBeFocused();
  expect(await page.evaluate(() => document.body.style.pointerEvents)).toBe('');
  await page.getByRole('button', { name: 'Chips mounted off' }).click();
  await expect(page.getByRole('combobox', { name: 'Chips search' })).toBeVisible();
});

test('genuine descendant Menu and Popover dismiss innermost without closing their parent shield', async ({
  page,
}) => {
  await gallery(page);
  const input = page.getByRole('combobox', { name: 'Descendant input' });
  await page.getByRole('button', { name: 'Open descendant selection layer' }).click();
  await expect(input).toBeFocused();
  await page.getByRole('button', { name: 'Descendant menu trigger' }).click();
  await page.keyboard.press('ArrowDown');
  await expect(page.getByRole('menuitem', { name: 'Descendant menu action' })).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('menu')).toHaveCount(0);
  await expect(input).toHaveAttribute('aria-expanded', 'true');
  await expect(page.getByRole('button', { name: 'Descendant menu trigger' })).toBeFocused();
  await page.getByRole('button', { name: 'Descendant Popover trigger' }).click();
  await expect(page.getByRole('button', { name: 'Descendant Popover action' })).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('heading', { name: 'Descendant child Popover' })).toHaveCount(0);
  await expect(input).toHaveAttribute('aria-expanded', 'true');
  await expect(page.getByRole('button', { name: 'Descendant Popover trigger' })).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(input).toHaveAttribute('aria-expanded', 'false');
  await expect(input).toBeFocused();
  await expect(input).toHaveValue('Retained nested query');
  await expect(page.locator('[data-slot=popover-outside-shield]')).toHaveCount(0);
  expect(await page.evaluate(() => document.body.style.pointerEvents)).toBe('');
});

test('real Chips compose inside Dialog and return through persistent controls', async ({
  page,
}) => {
  await gallery(page);
  await page.getByRole('button', { name: 'Selection dialog', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Nested selection dialog' });
  const input = dialog.getByRole('combobox', { name: 'Nested chips search' });
  await input.click();
  await input.press('ArrowDown');
  await input.press('Escape');
  await expect(input).toHaveAttribute('aria-expanded', 'false');
  await expect(dialog).toBeVisible();
  await expect(input).toBeFocused();
  await input.press('Escape');
  await expect(dialog).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Selection dialog', exact: true })).toBeFocused();
  expect(await page.evaluate(() => document.body.style.pointerEvents)).toBe('');
});

for (const width of [320, 390, 1440]) {
  for (const theme of ['light', 'dark'] as const) {
    test(`selection viewport and tokens ${width} ${theme}`, async ({ page }, info) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await gallery(page);
      await page.getByLabel('Outer theme').selectOption(theme);
      const input = page.getByRole('combobox', { name: 'Chips search' });
      await input.click();
      await input.pressSequentially('Gamma');
      const popup = page.locator('[data-slot=popover-content]');
      const option = page.getByRole('option', { name: /Gamma with a deliberately long label/ });
      await expect(option).toBeVisible();
      await expect(input).toBeFocused();
      await expect(input).toHaveValue('Gamma');
      expect(await popup.evaluate((node) => getComputedStyle(node).backgroundColor)).toBe(
        rgb(tokens.browser.surface.sheet[theme]),
      );
      expect(await input.evaluate((node) => getComputedStyle(node).color)).toBe(
        rgb(tokens.browser.color.text[theme]),
      );
      expect(
        await popup.evaluate((node) => Number.parseFloat(getComputedStyle(node).borderRadius)),
      ).toBe(Number.parseFloat(tokens.browser.metric.radius));
      expect(await popup.evaluate((node) => getComputedStyle(node).boxShadow)).toBe('none');
      const assertContained = async () => {
        await expect
          .poll(() =>
            popup.evaluate((node) => {
              const r = node.getBoundingClientRect();
              const v = window.visualViewport;
              if (!v) throw new Error('Missing real visualViewport');
              return (
                r.width > 0 &&
                r.height > 0 &&
                r.left >= v.offsetLeft - 1 &&
                r.top >= v.offsetTop - 1 &&
                r.right <= v.offsetLeft + v.width + 1 &&
                r.bottom <= v.offsetTop + v.height + 1
              );
            }),
          )
          .toBe(true);
      };
      await assertContained();
      await page.screenshot({ path: info.outputPath('open-900.png') });
      await page
        .getByRole('button', { name: 'Chips busy off', exact: true })
        .evaluate((button) => (button as HTMLButtonElement).click());
      await expect(input).toHaveAttribute('aria-busy', 'true');
      const beforeResize = await input.boundingBox();
      expect(beforeResize!.y + beforeResize!.height).toBeGreaterThan(420);
      await info.attach('before-resize-input', {
        body: JSON.stringify(beforeResize),
        contentType: 'application/json',
      });
      await input.evaluate((field) => {
        const original = field.scrollIntoView.bind(field);
        let calls = 0;
        field.setAttribute('data-host-reveals', '0');
        field.scrollIntoView = (options?: boolean | ScrollIntoViewOptions) => {
          field.setAttribute('data-host-reveals', String(++calls));
          field.setAttribute('data-host-reveal-options', JSON.stringify(options));
          original(options);
        };
      });
      await page.setViewportSize({ width, height: 420 });
      try {
        await expect(input).toHaveAttribute('data-host-reveals', '1');
        await assertContained();
      } finally {
        const observed = await popup.evaluate((node) => {
          const v = window.visualViewport;
          const field = document.querySelector<HTMLInputElement>(
            'input[aria-label="Chips search"]',
          );
          const style = getComputedStyle(node);
          return {
            popup: node.getBoundingClientRect().toJSON(),
            input: field?.getBoundingClientRect().toJSON(),
            query: field?.value,
            inputFocused: document.activeElement === field,
            expanded: field?.getAttribute('aria-expanded'),
            hostReveals: field?.getAttribute('data-host-reveals'),
            revealOptions: field?.getAttribute('data-host-reveal-options'),
            minimum: {
              minHeight: style.minHeight,
              paddingTop: style.paddingTop,
              paddingBottom: style.paddingBottom,
              borderTop: style.borderTopWidth,
              borderBottom: style.borderBottomWidth,
              clientHeight: node.clientHeight,
              scrollHeight: node.scrollHeight,
            },
            viewport: v && {
              width: v.width,
              height: v.height,
              offsetLeft: v.offsetLeft,
              offsetTop: v.offsetTop,
              pageTop: v.pageTop,
              scale: v.scale,
            },
            layout: { width: innerWidth, height: innerHeight, scrollY },
            maxHeight: style.maxHeight,
            availableHeight: style.getPropertyValue('--radix-popover-content-available-height'),
            side: node.getAttribute('data-side'),
          };
        });
        await info.attach('after-open-420-bounds', {
          body: JSON.stringify(observed, null, 2),
          contentType: 'application/json',
        });
        await page.screenshot({ path: info.outputPath('after-open-420-observed.png') });
      }
      await expect(input).toHaveValue('Gamma');
      await expect(input).toBeFocused();
      await expect(option).toBeInViewport();
      await page.screenshot({ path: info.outputPath('after-open-420.png') });
      await page.evaluate(() => window.scrollBy(0, 120));
      await assertContained();
      await page.screenshot({ path: info.outputPath('after-page-scroll.png') });
      await expect(input).toHaveAttribute('data-host-reveals', '1');
      await input.press('Escape');
      await expect(input).toHaveAttribute('aria-expanded', 'true');
      await expect(input).toHaveValue('Gamma');
      await page
        .getByRole('button', { name: 'Chips busy on', exact: true })
        .evaluate((button) => (button as HTMLButtonElement).click());
      await input.press('Escape');
      await expect(input).toHaveAttribute('aria-expanded', 'false');
      await expect(page.locator('[data-slot=popover-outside-shield]')).toHaveCount(0);
      expect(await page.evaluate(() => document.body.style.pointerEvents)).toBe('');
    });
  }
}

for (const theme of ['light', 'dark'] as const) {
  test(`readonly and unknown chips remain announced and inert ${theme}`, async ({ page }, info) => {
    await page.emulateMedia({ colorScheme: theme });
    await gallery(page);
    await page.getByLabel('Outer theme').selectOption(theme);
    await page.getByRole('button', { name: 'Chips read-only off', exact: true }).click();
    const remove = page.getByRole('button', { name: 'Remove Alpha', exact: true });
    await expect(remove).toHaveAttribute('aria-disabled', 'true');
    expect(await remove.evaluate((node) => (node as HTMLButtonElement).disabled)).toBe(false);
    const before = await page.getByLabel('Chips parent clicks').textContent();
    await remove.click({ force: true });
    await remove.press('Enter');
    await remove.press('Space');
    await expect(remove).toBeFocused();
    await expect(page.getByLabel('Chips remove count')).toHaveText('0');
    await expect(page.getByLabel('Chips Send count')).toHaveText('0');
    await expect(page.getByLabel('Chips parent clicks')).toHaveText(before!);
    await page.screenshot({ path: info.outputPath('readonly-state.png') });
    await page.getByRole('button', { name: 'Chips read-only on', exact: true }).click();
    await page.getByRole('button', { name: 'Chips unknown off', exact: true }).click();
    await expect(remove).toBeDisabled();
    await expect(remove).toHaveAttribute('aria-busy', 'true');
    await expect(
      page.getByRole('status').filter({ hasText: 'Unknown: outcome uncertain; draft retained' }),
    ).toBeVisible();
    await remove.click({ force: true });
    await expect(page.getByLabel('Chips remove count')).toHaveText('0');
    await expect(page.getByRole('combobox', { name: 'Chips search' })).toHaveAttribute(
      'aria-busy',
      'true',
    );
    await page.screenshot({ path: info.outputPath('unknown-state.png') });
  });
}

for (const theme of ['light', 'dark'] as const) {
  test(`native visual viewport offset preserves the focused draft ${theme}`, async ({
    page,
  }, info) => {
    await page.setViewportSize({ width: 390, height: 900 });
    await gallery(page);
    await page.getByLabel('Outer theme').selectOption(theme);
    const input = page.getByRole('combobox', { name: 'Chips search' });
    await input.click();
    await input.pressSequentially('Gamma');
    await input.evaluate((field) => field.setAttribute('data-retained-offset-input', 'yes'));
    await page
      .getByRole('button', { name: 'Chips busy off', exact: true })
      .evaluate((button) => (button as HTMLButtonElement).click());
    const session = await page.context().newCDPSession(page);
    try {
      // Native Chromium page scaling changes the actual visual viewport. This
      // is an emulated pinch/viewport test, not a physical keyboard claim.
      await session.send('Emulation.setPageScaleFactor', { pageScaleFactor: 1.25 });
      const popup = page.locator('[data-slot=popover-content]');
      try {
        await expect
          .poll(() =>
            popup.evaluate((node) => {
              const r = node.getBoundingClientRect();
              const v = window.visualViewport;
              if (!v) throw new Error('Missing real visualViewport');
              return (
                v.scale > 1 &&
                (v.offsetLeft > 0 || v.offsetTop > 0) &&
                r.width > 0 &&
                r.height > 0 &&
                r.left >= v.offsetLeft - 1 &&
                r.top >= v.offsetTop - 1 &&
                r.right <= v.offsetLeft + v.width + 1 &&
                r.bottom <= v.offsetTop + v.height + 1
              );
            }),
          )
          .toBe(true);
      } finally {
        await info.attach('native-offset-bounds', {
          contentType: 'application/json',
          body: JSON.stringify(
            await popup.evaluate((node) => {
              const v = window.visualViewport;
              const field = document.querySelector<HTMLInputElement>(
                'input[aria-label="Chips search"]',
              );
              const style = getComputedStyle(node);
              return {
                popup: node.getBoundingClientRect().toJSON(),
                input: field?.getBoundingClientRect().toJSON(),
                viewport: v && {
                  width: v.width,
                  height: v.height,
                  offsetLeft: v.offsetLeft,
                  offsetTop: v.offsetTop,
                  scale: v.scale,
                },
                minimum: {
                  minHeight: style.minHeight,
                  paddingTop: style.paddingTop,
                  paddingBottom: style.paddingBottom,
                  borderTop: style.borderTopWidth,
                  borderBottom: style.borderBottomWidth,
                  scrollHeight: node.scrollHeight,
                  clientHeight: node.clientHeight,
                },
                query: field?.value,
                focused: document.activeElement === field,
              };
            }),
            null,
            2,
          ),
        });
        await page.screenshot({ path: info.outputPath('native-offset.png') });
      }
      await expect(input).toHaveValue('Gamma');
      await expect(input).toBeFocused();
      await expect(input).toHaveAttribute('data-retained-offset-input', 'yes');
      await input.press('Escape');
      await expect(input).toHaveAttribute('aria-expanded', 'true');
    } finally {
      await session.send('Emulation.setPageScaleFactor', { pageScaleFactor: 1 });
      await session.detach();
    }
  });
}

test('cmdk owns Home/End/arrows and skips disabled options while Space remains editing', async ({
  page,
}) => {
  await gallery(page);
  const input = page.getByRole('combobox', { name: 'Selection search' });
  await input.focus();
  await input.press('End');
  await expect(
    page.getByRole('option', { name: /Gamma with a deliberately long label/ }),
  ).toHaveAttribute('aria-selected', 'true');
  await input.press('Home');
  await expect(page.getByRole('option', { name: 'Alpha', exact: true })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  await input.press('ArrowDown');
  await expect(page.getByRole('option', { name: 'Beta', exact: true })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  await input.press('ArrowDown');
  await expect(
    page.getByRole('option', { name: /Gamma with a deliberately long label/ }),
  ).toHaveAttribute('aria-selected', 'true');
  await expect(page.getByRole('option', { name: 'Unavailable option' })).toHaveAttribute(
    'aria-disabled',
    'true',
  );
  await expect(page.locator('[cmdk-item][aria-selected=true]')).toHaveCount(1);
  await input.press('Space');
  await expect(input).toHaveValue(' ');
  await expect(input).toBeFocused();
  await expect(page.getByLabel('Selection callback count')).toHaveText('0');
  await expect(page.getByLabel('Selection send count')).toHaveText('0');
});

test('Chips Tab in both directions closes without adding and preserves native traversal', async ({
  page,
}) => {
  await gallery(page);
  const input = page.getByRole('combobox', { name: 'Chips search' });
  await input.focus();
  await input.pressSequentially('Be');
  await input.press('Tab');
  await expect(page.getByRole('button', { name: 'After chips', exact: true })).toBeFocused();
  await expect(input).toHaveAttribute('aria-expanded', 'false');
  await expect(page.getByLabel('Chips add count')).toHaveText('0');
  await input.focus();
  await input.press('Shift+Tab');
  await expect(page.getByRole('button', { name: 'Remove Alpha', exact: true })).toBeFocused();
  await expect(input).toHaveAttribute('aria-expanded', 'false');
  await expect(input).toHaveValue('Be');
  await expect(page.getByLabel('Chips add count')).toHaveText('0');
  await expect(page.getByLabel('Chips Send count')).toHaveText('0');
});

test('programmatic close requests honor host veto without a selection or query mutation', async ({
  page,
}) => {
  await gallery(page);
  const input = page.getByRole('combobox', { name: 'Selection search' });
  await page.getByRole('button', { name: 'Close veto off', exact: true }).click();
  await page
    .getByRole('button', { name: 'Open selection programmatically' })
    .evaluate((button) => (button as HTMLButtonElement).click());
  await expect(input).toHaveAttribute('aria-expanded', 'true');
  await input.pressSequentially('Be');
  await page
    .getByRole('button', { name: 'Close selection programmatically' })
    .evaluate((button) => (button as HTMLButtonElement).click());
  await expect(input).toHaveAttribute('aria-expanded', 'true');
  await expect(input).toHaveValue('Be');
  await expect(input).toBeFocused();
  await expect(page.getByLabel('Selection callback count')).toHaveText('0');
  await page
    .getByRole('button', { name: 'Close veto on', exact: true })
    .evaluate((button) => (button as HTMLButtonElement).click());
  await page
    .getByRole('button', { name: 'Close selection programmatically' })
    .evaluate((button) => (button as HTMLButtonElement).click());
  await expect(input).toHaveAttribute('aria-expanded', 'false');
  await expect(input).toHaveValue('Be');
  await expect(input).toBeFocused();
  await expect(page.getByLabel('Selection callback count')).toHaveText('0');
});

test('deferred popup close never steals focus from an admitted chip removal', async ({
  page,
}, info) => {
  await gallery(page);
  const input = page.getByRole('combobox', { name: 'Chips search' });
  await input.focus();
  await input.pressSequentially('Be');
  await input.press('Enter');
  await expect(page.getByLabel('Chips add count')).toHaveText('1');
  await expect(input).toHaveAttribute('aria-expanded', 'false');
  await input.press('ArrowDown');
  await expect(input).toHaveAttribute('aria-expanded', 'true');
  // Freeze only this regression's deferred stock FocusScope cleanup. Native
  // key events/removal still run; releasing the clock exposes late focus theft.
  await page.clock.install({ time: new Date('2026-01-01T00:00:00Z') });
  await page.clock.pauseAt(new Date('2026-01-01T00:00:01Z'));
  await input.press('Escape');
  await expect(input).toHaveAttribute('aria-expanded', 'false');
  const alpha = page.getByRole('button', { name: 'Remove Alpha', exact: true });
  const beta = page.getByRole('button', { name: 'Remove Beta', exact: true });
  await alpha.focus();
  await page.keyboard.press('Enter');
  await expect(page.getByLabel('Chips remove count')).toHaveText('1');
  await expect(beta).toBeFocused();
  await page.clock.runFor(1);
  try {
    await expect(beta).toBeFocused();
  } finally {
    await info.attach('deferred-close-focus', {
      contentType: 'application/json',
      body: JSON.stringify(
        await page.evaluate(() => ({
          active: document.activeElement?.outerHTML,
          input: document.querySelector('input[aria-label="Chips search"]')?.outerHTML,
        })),
      ),
    });
  }
});
