import { expect, test, type Page } from '@playwright/test';
import { controlRadii, focusFrames, textAtSurfaceEdge } from './audits.js';

const request = (page: Page, label: string) =>
  page
    .getByRole('button', { name: label, exact: true })
    .evaluate((node) => (node as HTMLButtonElement).click());
for (const source of ['React', 'Native'] as const)
  for (const theme of ['light', 'dark'] as const)
    for (const width of [1440, 390])
      for (const kind of ['icon', 'text']) {
        test(`${source} ${kind} menu ${width} ${theme}: shared keyboard and original selection`, async ({
          page,
        }) => {
          await page.setViewportSize({ width, height: 992 });
          await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
          await page.goto(`/?frame=menu-host&theme=${theme}&kind=${kind}`);
          const trigger = page.getByRole('button', { name: `${source} menu`, exact: true });
          const menu = page.getByRole('menu', { name: `${source} menu`, exact: true });
          await trigger.click();
          const alpha = menu.getByRole('menuitemradio', { name: 'Alpha', exact: true });
          const gamma = menu.getByRole('menuitemradio', { name: 'Gamma', exact: true });
          const delta = menu.getByRole('menuitemradio', { name: 'Delta', exact: true });
          await expect(alpha).toBeFocused();
          await expect(alpha).toHaveAttribute('aria-checked', 'true');
          const blocked = menu.getByRole('menuitem', { name: 'Blocked', exact: true });
          await expect(blocked).toBeDisabled();
          await expect(blocked).toHaveAccessibleDescription('Read only');
          await page.keyboard.press('ArrowDown');
          await expect(gamma).toBeFocused();
          await page.keyboard.press('End');
          await expect(delta).toBeFocused();
          await page.keyboard.press('Home');
          await expect(alpha).toBeFocused();
          await page.keyboard.press('d');
          await expect(delta).toBeFocused();
          await page.keyboard.press('ArrowDown');
          await expect(alpha).toBeFocused();
          await page.keyboard.press('Shift+Escape');
          await expect(menu).toBeVisible();
          const frames = await focusFrames(page, '.tmt-ui-menu-surface[open]');
          expect(frames.clipped).toEqual([]);
          // The audit walks focusable controls; explicitly focus the last row for its geometry.
          await delta.focus();
          await expect(delta).toBeFocused();
          expect((await focusFrames(page, '.tmt-ui-menu-surface[open]')).clipped).toEqual([]);
          await expect(page.locator('.tmt-ui-menu-surface[open]')).toHaveCSS('padding', '4px');
          await expect(delta).toHaveCSS('border-radius', '6px');
          await expect(delta).toHaveCSS('height', '42px');
          expect(frames.doubled).toEqual([]);
          expect(await textAtSurfaceEdge(page, '.tmt-ui-menu-surface[open]')).toEqual([]);
          expect((await controlRadii(page, '.tmt-ui-menu-surface[open]')).off).toEqual([]);
          await page.keyboard.press('Escape');
          await expect(menu).toBeHidden();
          await expect(trigger).toBeFocused();
          await expect(page.getByTestId('selection')).toHaveText('none');
          await trigger.press('ArrowUp');
          await expect(delta).toBeFocused();
          await page.keyboard.press('Enter');
          await expect(menu).toBeHidden();
          await expect(page.getByTestId('selection')).toHaveText(
            `${source.toLowerCase()} A delta original=true`,
          );
          await trigger.click();
          await expect(delta).toHaveAttribute('aria-checked', 'true');
          await page.mouse.click(width - 4, 988);
          await expect(menu).toBeHidden();
          await expect(trigger).toBeFocused();
          const other = source === 'React' ? 'Native' : 'React';
          await page.getByRole('button', { name: `${other} menu`, exact: true }).press('ArrowDown');
          await expect(
            page
              .getByRole('menu', { name: `${other} menu`, exact: true })
              .getByRole('menuitemradio', { name: 'Alpha', exact: true }),
          ).toBeFocused();
          expect(await focusFrames(page, '.tmt-ui-menu-surface[open]')).toEqual(frames);
          await page.keyboard.press('Escape');
        });
      }

for (const source of ['React', 'Native'] as const) {
  test(`${source}: update retains focused key, checked state and no obsolete selection`, async ({
    page,
  }) => {
    await page.goto('/?frame=menu-host');
    await page.getByRole('button', { name: `${source} menu`, exact: true }).click();
    const menu = page.getByRole('menu', { name: `${source} menu`, exact: true });
    const gamma = menu.getByRole('menuitemradio', { name: 'Gamma', exact: true });
    await page.keyboard.press('ArrowDown');
    await expect(gamma).toBeFocused();
    await request(page, 'Reverse items');
    await expect(gamma).toBeFocused();
    await request(page, 'Set Gamma');
    await expect(gamma).toHaveAttribute('aria-checked', 'true');
    await expect(gamma).toBeFocused();
    await menu.getByRole('menuitemradio', { name: 'Alpha', exact: true }).focus();
    await request(page, 'Remove Alpha');
    await expect(menu.getByRole('menuitemradio', { name: 'Alpha', exact: true })).toHaveCount(0);
    await expect(menu.getByRole('menuitemradio', { name: 'Delta', exact: true })).toBeFocused();
    await gamma.evaluate((node) => (node as HTMLButtonElement).click());
    await expect(page.getByTestId('selection')).toHaveText('none');
    await expect(menu).toBeVisible();
  });
  test(`${source}: close keeps the selected callback and original event even when host changes synchronously`, async ({
    page,
  }) => {
    await page.goto('/?frame=menu-host&replace-on-close=true');
    await page.getByRole('button', { name: `${source} menu`, exact: true }).click();
    await page
      .getByRole('menu', { name: `${source} menu`, exact: true })
      .getByRole('menuitemradio', { name: 'Gamma', exact: true })
      .click();
    await expect(page.getByTestId('selection')).toHaveText(
      `${source.toLowerCase()} A gamma original=true`,
    );
  });
  test(`${source}: actual native close and host disposal each report once; no initial or replay false`, async ({
    page,
  }) => {
    await page.goto('/?frame=menu-host');
    const trigger = page.getByRole('button', { name: `${source} menu`, exact: true });
    await expect(page.getByTestId('transitions')).toBeEmpty();
    await trigger.click();
    await page
      .locator('.tmt-ui-menu-surface[open]')
      .evaluate((node) => (node as HTMLDialogElement).close());
    await expect(page.getByTestId('transitions')).toHaveText(
      `${source.toLowerCase()}:true ${source.toLowerCase()}:false`,
    );
    await expect(trigger).toBeFocused();
    await trigger.click();
    await request(page, `Toggle ${source === 'Native' ? 'native' : 'React'} host`);
    await expect(page.locator('.tmt-ui-menu-surface[open]')).toHaveCount(0);
    await expect(page.getByTestId('transitions')).toHaveText(
      `${source.toLowerCase()}:true ${source.toLowerCase()}:false ${source.toLowerCase()}:true ${source.toLowerCase()}:false`,
    );
    await expect(page.getByTestId('selection')).toHaveText('none');
  });
  test(`${source}: exit completion cannot remove a reopened menu`, async ({ page }) => {
    await page.emulateMedia({ reducedMotion: 'no-preference' });
    await page.goto('/?frame=menu-host');
    await page.getByRole('button', { name: `${source} menu`, exact: true }).click();
    await page.keyboard.press('Escape');
    await request(page, `Open ${source === 'React' ? 'React' : 'native'} request`);
    await page.evaluate(async () =>
      Promise.allSettled(
        document
          .getAnimations()
          .filter((animation) => Number.isFinite(animation.effect?.getComputedTiming().endTime))
          .map((animation) => animation.finished),
      ),
    );
    await expect(page.getByRole('menu', { name: `${source} menu`, exact: true })).toBeVisible();
    await expect(page.getByTestId('selection')).toHaveText('none');
  });
}

test('mixed renderers replace one popup without stealing the new focus', async ({ page }) => {
  await page.goto('/?frame=menu-host');
  await page.getByRole('button', { name: 'React menu', exact: true }).click();
  await request(page, 'Open native request');
  await expect(page.getByRole('menu', { name: 'React menu', exact: true })).toBeHidden();
  await expect(
    page
      .getByRole('menu', { name: 'Native menu', exact: true })
      .getByRole('menuitemradio', { name: 'Alpha', exact: true }),
  ).toBeFocused();
  await request(page, 'Open React request');
  await expect(page.getByRole('menu', { name: 'Native menu', exact: true })).toBeHidden();
  await expect(
    page
      .getByRole('menu', { name: 'React menu', exact: true })
      .getByRole('menuitemradio', { name: 'Alpha', exact: true }),
  ).toBeFocused();
  await expect(page.locator('.tmt-ui-menu-surface[open]')).toHaveCount(1);
  await expect(page.getByTestId('transitions')).toHaveText(
    'react:true react:false native:true native:false react:true',
  );
});

test('native menu dismisses before its drawer and its backdrop never clicks through', async ({
  page,
}) => {
  await page.goto('/?frame=menu-host');
  await page.getByRole('button', { name: 'Open drawer', exact: true }).click();
  const drawer = page.getByRole('dialog', { name: 'Menu fixture', exact: true });
  const trigger = drawer.getByRole('button', { name: 'Drawer native menu', exact: true });
  await trigger.click();
  await page.keyboard.press('Escape');
  await expect(drawer).toBeVisible();
  await expect(trigger).toBeFocused();
  await trigger.click();
  await page.mouse.click(4, 4);
  await expect(page.getByRole('menu', { name: 'Drawer native menu', exact: true })).toBeHidden();
  await expect(drawer).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(drawer).toBeHidden();
});

test('icon help has one mixed-renderer owner and stays suppressed while its menu is expanded', async ({
  page,
}) => {
  await page.goto('/?frame=menu-host');
  const react = page.getByRole('button', { name: 'React menu', exact: true });
  const native = page.getByRole('button', { name: 'Native menu', exact: true });
  const shown = page.locator('.tmt-ui-icon-action-tooltip:popover-open');
  await react.hover();
  await expect(shown).toHaveText('React menu');
  await native.hover();
  await expect(shown).toHaveText('Native menu');
  await native.click();
  await expect(shown).toHaveCount(0);
  await page.keyboard.press('Escape');
  await expect(native).toBeFocused();
  await expect(shown).toHaveCount(0);
  await page.keyboard.press('Tab');
  await page.keyboard.press('Shift+Tab');
  await expect(native).toBeFocused();
  await expect(shown).toHaveText('Native menu');
});

for (const theme of ['light', 'dark'] as const)
  test(`both renderers use identical existing surface metrics in ${theme}`, async ({ page }) => {
    await page.goto(`/?frame=menu-host&theme=${theme}`);
    const styles: unknown[] = [];
    for (const source of ['React', 'Native']) {
      await page.getByRole('button', { name: `${source} menu`, exact: true }).click();
      styles.push(
        await page.locator('.tmt-ui-menu-surface[open]').evaluate(async (node) => {
          await Promise.allSettled(
            node
              .getAnimations()
              .filter((animation) => Number.isFinite(animation.effect?.getComputedTiming().endTime))
              .map((animation) => animation.finished),
          );
          const style = getComputedStyle(node);
          const row = getComputedStyle(node.querySelector('button')!);
          return {
            width: node.getBoundingClientRect().width,
            font: style.font,
            fill: style.backgroundColor,
            edge: style.border,
            radius: style.borderRadius,
            shadow: style.boxShadow,
            rowPadding: row.padding,
            rowFont: row.font,
          };
        }),
      );
      await page.keyboard.press('Escape');
    }
    expect(styles[1]).toEqual(styles[0]);
  });

for (const source of ['React', 'Native'] as const)
  test(`${source}: Tab closes and continues outside without selecting`, async ({ page }) => {
    await page.goto('/?frame=menu-host');
    await page.getByRole('button', { name: `${source} menu`, exact: true }).click();
    await page.keyboard.press('Tab');
    await expect(page.getByRole('menu', { name: `${source} menu`, exact: true })).toBeHidden();
    const next = source === 'React' ? 'Native menu' : 'Reverse items';
    await expect(page.getByRole('button', { name: next, exact: true })).toBeFocused();
    await expect(page.getByTestId('selection')).toHaveText('none');
  });

for (const source of ['React', 'Native'] as const) {
  test(`${source}: grouped menu exposes names, descriptions and keyboard groups`, async ({
    page,
  }) => {
    await page.goto('/?frame=menu-host&grouped=true');
    await page.getByRole('button', { name: `${source} menu`, exact: true }).click();
    const menu = page.getByRole('menu', { name: `${source} menu`, exact: true });
    const first = menu.getByRole('group', { name: 'First group', exact: true });
    const second = menu.getByRole('group', { name: 'Second group', exact: true });
    await expect(first).toBeVisible();
    await expect(second).toBeVisible();
    await expect(second.locator('.tmt-ui-menu-heading')).toHaveAttribute('data-separated', 'true');
    await expect(second.locator('.tmt-ui-menu-divider')).toHaveCount(0);
    const alpha = first.getByRole('menuitemradio', { name: 'Alpha 4', exact: true });
    await expect(alpha).toHaveAccessibleDescription('Detail');
    await expect(alpha).toBeFocused();
    await page.keyboard.press('End');
    await expect(second.getByRole('menuitemradio', { name: 'Delta', exact: true })).toBeFocused();
    await page.keyboard.press('a');
    await expect(alpha).toBeFocused();
    await expect(menu.locator('.tmt-ui-menu-leading-icon').first()).toHaveAttribute(
      'aria-hidden',
      'true',
    );
    await expect(menu.locator('.tmt-ui-menu-surface')).toHaveCount(0);
  });
}

// Captured from current main before the grouped API: normalization only removes the generated menu ID.
const defaultMenuMarkup: Record<string, string> = {
  React:
    '<div class="tmt-ui-menu-description" id="MENU_ID-description">Choose a fixture value</div><div><button type="button" class="tmt-ui-menu-item" role="menuitemradio" aria-checked="true" tabindex="-1"><span>Alpha</span><span aria-hidden="true">✓</span></button></div><div><button type="button" class="tmt-ui-menu-item" role="menuitem" aria-describedby="MENU_ID-reason-1" disabled="" tabindex="-1"><span>Blocked</span></button><span class="tmt-ui-menu-reason" id="MENU_ID-reason-1">Read only</span></div><div><button type="button" class="tmt-ui-menu-item" role="menuitemradio" aria-checked="false" tabindex="-1"><span>Gamma</span></button></div><div><button type="button" class="tmt-ui-menu-item" data-separated="true" role="menuitemradio" aria-checked="false" tabindex="-1"><span>Delta</span></button></div>',
  Native:
    '<div id="MENU_ID-description" class="tmt-ui-menu-description">Choose a fixture value</div><div><button type="button" class="tmt-ui-menu-item" tabindex="-1" role="menuitemradio" aria-checked="true"><span>Alpha</span><span aria-hidden="true">✓</span></button></div><div><button type="button" class="tmt-ui-menu-item" tabindex="-1" disabled="" role="menuitem" aria-describedby="MENU_ID-reason-1"><span>Blocked</span></button><span class="tmt-ui-menu-reason" id="MENU_ID-reason-1">Read only</span></div><div><button type="button" class="tmt-ui-menu-item" tabindex="-1" role="menuitemradio" aria-checked="false"><span>Gamma</span></button></div><div><button type="button" class="tmt-ui-menu-item" tabindex="-1" role="menuitemradio" aria-checked="false" data-separated="true"><span>Delta</span></button></div>',
};
for (const source of ['React', 'Native'] as const)
  test(`${source}: default menu DOM is byte-identical`, async ({ page }) => {
    await page.goto('/?frame=menu-host');
    await page.getByRole('button', { name: `${source} menu`, exact: true }).click();
    const menu = page.getByRole('menu', { name: `${source} menu`, exact: true });
    const markup = await menu.evaluate((node) => node.innerHTML.replaceAll(node.id, 'MENU_ID'));
    expect(markup).toBe(defaultMenuMarkup[source]);
  });

for (const source of ['React', 'Native'] as const)
  for (const width of [320, 390, 1440])
    for (const theme of ['light', 'dark'] as const)
      test(`${source}: grouped menu fits ${width} ${theme}`, async ({ page }, info) => {
        await page.setViewportSize({ width, height: 900 });
        await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
        await page.goto(`/?frame=menu-host&grouped=true&theme=${theme}`);
        await page.getByRole('button', { name: `${source} menu`, exact: true }).click();
        const surface = page.locator('.tmt-ui-menu-surface[open]');
        const box = await surface.boundingBox();
        expect(box).not.toBeNull();
        expect(box!.width).toBeGreaterThanOrEqual(260);
        expect(box!.x).toBeGreaterThanOrEqual(0);
        expect(box!.x + box!.width).toBeLessThanOrEqual(width);
        const menu = page.getByRole('menu', { name: `${source} menu`, exact: true });
        const delta = menu.getByRole('menuitemradio', { name: 'Delta', exact: true });
        await expect(delta).toHaveCSS('min-height', '32px');
        await page.screenshot({ path: info.outputPath('grouped-menu.png') });
      });
