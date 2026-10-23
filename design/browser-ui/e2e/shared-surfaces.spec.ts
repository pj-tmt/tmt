import { expect, test } from '@playwright/test';
import { controlBoundaries, controlRadii, focusFrames, textAtSurfaceEdge } from './audits';

for (const theme of ['light', 'dark'] as const) {
  for (const width of [1440, 390])
    for (const size of ['regular', 'wide'] as const) {
      test(`dialog ${size} ${width} ${theme}: reading width and modal boundary`, async ({
        page,
      }) => {
        await page.setViewportSize({ width, height: 992 });
        await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
        await page.goto(`/?frame=dialog&theme=${theme}&size=${size}&open=false`);
        const opener = page.getByRole('button', { name: 'Open dialog', exact: true });
        await opener.click();
        const dialog = page.getByRole('dialog');
        await expect(dialog).toBeVisible();
        await expect(dialog).toHaveCSS('scale', '1');
        const box = (await dialog.boundingBox())!;
        expect(box.width).toBe(Math.min(size === 'wide' ? 800 : 590, width - 32));
        expect(box.x).toBeGreaterThanOrEqual(16);
        expect(await dialog.evaluate((node) => node.matches(':modal'))).toBe(true);
        expect(
          await dialog.evaluate((node) => getComputedStyle(node, '::backdrop').backdropFilter),
        ).toBe('blur(3px)');
        await page.keyboard.press('Escape');
        await expect(dialog).toBeHidden();
        await expect(opener).toBeFocused();
      });
    }
  for (const width of [1440, 390])
    for (const variant of ['regular', 'wide'] as const) {
      test(`drawer ${variant} ${width} ${theme}: viewport fit, scrim and shared header`, async ({
        page,
      }) => {
        await page.setViewportSize({ width, height: 992 });
        await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
        await page.goto(`/?frame=drawer&theme=${theme}&width=${variant}&open=false`);
        const opener = page.getByRole('button', { name: 'Open drawer', exact: true });
        await opener.click();
        const drawer = page.getByRole('dialog', { name: 'Page settings' });
        await expect(drawer).toBeVisible();
        const box = (await drawer.boundingBox())!;
        expect(box.width).toBe(
          variant === 'wide' ? (width === 390 ? 382 : 660) : Math.min(430, width - 12),
        );
        expect(box.x).toBeGreaterThanOrEqual(0);
        expect(box.x + box.width).toBeLessThanOrEqual(width);
        expect(
          await drawer.evaluate((node) => getComputedStyle(node, '::backdrop').backdropFilter),
        ).toBe('blur(3px)');
        await expect(drawer.getByText('Drafts stay here', { exact: true })).toBeVisible();
        await drawer.getByRole('button', { name: 'Close Page settings', exact: true }).click();
        await expect(drawer).toBeHidden();
        await expect(opener).toBeFocused();
      });
    }
  for (const width of [1440, 1024, 390]) {
    test(`non-modal panel ${width} ${theme}: focus, page interaction and dismissal`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 992 });
      await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
      await page.goto(`/?frame=panel&theme=${theme}&open=false`);
      const opener = page.getByRole('button', { name: 'Discussion', exact: true });
      await opener.click();
      const panel = page.getByRole('complementary', { name: 'Discussion' });
      const heading = panel.getByRole('heading', { name: 'Discussion' });
      await expect(heading).toBeFocused();
      await expect(page.locator(':modal, [inert]')).toHaveCount(0);
      const box = (await panel.boundingBox())!;
      if (width === 1024) {
        expect(box.width).toBe(330);
        expect(width - box.x - box.width).toBe(12);
        expect(992 - box.y - box.height).toBe(12);
      } else if (width === 390) {
        expect(box.x).toBe(7);
        expect(box.width).toBe(width - 14);
        expect(992 - box.y - box.height).toBe(7);
        expect(box.height).toBeLessThanOrEqual(992 * 0.75);
      } else {
        await expect(panel).toHaveCSS('position', 'static');
        await expect(panel).toHaveCSS('box-shadow', 'none');
      }
      await heading.press('Shift+Tab');
      const outside = page.getByRole('button', { name: 'Page action', exact: true });
      await expect(outside).toBeFocused();
      await outside.click();
      await expect(page.locator('#page-action-status')).toHaveText('Page actions 1');
      await expect(outside).toBeFocused();
      await outside.press('Escape');
      await expect(panel).toBeVisible();
      await panel.getByRole('textbox', { name: 'Reply', exact: true }).press('Escape');
      await expect(panel).toBeVisible();
      await heading.focus();
      await heading.press('Shift+Escape');
      await expect(panel).toBeVisible();
      const focus = await focusFrames(page, '.tmt-ui-panel');
      expect(focus.checked).toBeGreaterThan(0);
      expect(focus.clipped).toEqual([]);
      expect(focus.doubled).toEqual([]);
      expect(await textAtSurfaceEdge(page, '.tmt-ui-panel')).toEqual([]);
      const boundaries = await controlBoundaries(page, '.tmt-ui-panel');
      expect(boundaries.borders.misses).toEqual([]);
      expect(boundaries.fields.sameFill).toEqual([]);
      expect(boundaries.fields.unlabelled).toEqual([]);
      expect((await controlRadii(page, '.tmt-ui-panel')).off).toEqual([]);
      await heading.press('Escape');
      await expect(panel).toHaveCount(0);
      await expect(opener).toBeFocused();
      await opener.click();
      await panel.getByRole('button', { name: 'Close Discussion', exact: true }).click();
      await expect(panel).toHaveCount(0);
      await expect(opener).toBeFocused();
    });
  }
  for (const width of [1440, 390])
    for (const intent of ['archive', 'delete'] as const) {
      test(`confirmation ${intent} ${width} ${theme}: centred heading/actions and host intent`, async ({
        page,
      }) => {
        await page.setViewportSize({ width, height: 992 });
        await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
        await page.goto(`/?frame=dialog&presentation=confirm&intent=${intent}&theme=${theme}`);
        await page.evaluate(() => document.fonts.ready);
        const verb = intent === 'delete' ? 'Delete' : 'Archive';
        const dialog = page.getByRole('dialog', { name: `${verb} page?` });
        await expect(dialog).toBeVisible();
        await expect(dialog.locator('.tmt-ui-dialog-confirm-mark')).toHaveAttribute(
          'aria-hidden',
          'true',
        );
        const heading = dialog.getByRole('heading', { name: `${verb} page?` });
        await expect(heading).toHaveCount(1);
        const title = (await heading.boundingBox())!;
        const surface = (await dialog.boundingBox())!;
        const center = surface.x + surface.width / 2;
        expect(Math.abs(title.x + title.width / 2 - center)).toBeLessThan(1);
        const cancel = dialog.getByRole('button', { name: 'Cancel', exact: true });
        const action = dialog.getByRole('button', { name: `${verb} page`, exact: true });
        await expect(cancel).toHaveAttribute('data-variant', 'secondary');
        await expect(action).toHaveAttribute(
          'data-variant',
          intent === 'delete' ? 'destructive' : 'primary',
        );
        const first = (await cancel.boundingBox())!;
        const last = (await action.boundingBox())!;
        expect(Math.abs((first.x + last.x + last.width) / 2 - center)).toBeLessThan(1);
        expect(await textAtSurfaceEdge(page, '.tmt-ui-dialog')).toEqual([]);
      });
    }
}

test('panel dismissal admission retains focus and a removed opener uses the fallback', async ({
  page,
}) => {
  await page.goto('/?frame=panel&open=false&refuse=true');
  await page.getByRole('button', { name: 'Discussion', exact: true }).click();
  const panel = page.getByRole('complementary', { name: 'Discussion' });
  const close = panel.getByRole('button', { name: 'Close Discussion', exact: true });
  await close.click();
  await expect(panel).toBeVisible();
  await expect(close).toBeFocused();
  await expect(page.getByRole('status')).toHaveText('Dismissal refused 1');
  await page.goto('/?frame=panel&open=false');
  await page.getByRole('button', { name: 'Discussion', exact: true }).click();
  await panel.getByRole('button', { name: 'Remove opener', exact: true }).click();
  await panel.getByRole('button', { name: 'Close Discussion', exact: true }).click();
  await expect(panel).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Page action', exact: true })).toBeFocused();
});

test('panel mode changes retain the host draft and current control focus', async ({ page }) => {
  await page.setViewportSize({ width: 1024, height: 992 });
  await page.goto('/?frame=panel&open=false');
  await page.getByRole('button', { name: 'Discussion', exact: true }).click();
  const panel = page.getByRole('complementary', { name: 'Discussion' });
  const reply = panel.getByRole('textbox', { name: 'Reply', exact: true });
  await reply.fill('Keep this draft across responsive modes');
  for (const [width, mode] of [
    [390, 'sheet'],
    [1440, 'column'],
    [1024, 'card'],
  ] as const) {
    await page.setViewportSize({ width, height: 992 });
    await expect(panel).toHaveAttribute('data-mode', mode);
    await expect(reply).toHaveValue('Keep this draft across responsive modes');
    await expect(reply).toBeFocused();
  }
});

for (const theme of ['light', 'dark'] as const)
  for (const width of [1440, 1024, 390])
    for (const close of ['close', 'unmount']) {
      test(`panel host ${close} ${width} ${theme} preserves page control focus`, async ({
        page,
      }) => {
        await page.setViewportSize({ width, height: 992 });
        await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
        await page.goto(`/?frame=panel&open=false&host-close=${close}&theme=${theme}`);
        const opener = page.getByRole('button', { name: 'Discussion', exact: true });
        await opener.click();
        const panel = page.getByRole('complementary', { name: 'Discussion' });
        await expect(panel.getByRole('heading')).toBeFocused();
        const outside = page.getByRole('button', { name: 'Page action', exact: true });
        await outside.focus();
        await outside.click();
        await expect(panel).toHaveCount(0);
        await expect(outside).toBeFocused();
        await expect(opener).not.toBeFocused();
      });
    }

for (const close of ['close', 'unmount']) {
  test(`panel host ${close} returns focus when it was inside the panel`, async ({ page }) => {
    await page.goto(`/?frame=panel&open=false&host-close=${close}`);
    const opener = page.getByRole('button', { name: 'Discussion', exact: true });
    await opener.click();
    const panel = page.getByRole('complementary', { name: 'Discussion' });
    await expect(panel.getByRole('heading')).toBeFocused();
    // A host-driven state change does not move focus to the control invoking it.
    await page
      .getByRole('button', { name: 'Page action', exact: true })
      .evaluate((node) => node.click());
    await expect(panel).toHaveCount(0);
    await expect(opener).toBeFocused();
  });
}
