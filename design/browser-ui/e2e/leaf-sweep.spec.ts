import { expect, test } from '@playwright/test';

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`header actions ${width} ${theme}: compact shared context and retained busy admission`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 992 });
      await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
      await page.goto(`/?frame=source&theme=${theme}`);
      const save = page.getByRole('button', { name: 'Save', exact: true });
      await expect(save).toHaveCSS('font-size', '11px');
      await expect(save).toHaveCSS('font-weight', '620');
      await expect(save).toHaveCSS('padding', '7px 10px');
      await expect(save).toHaveCSS('border-radius', '8px');
      const idle = (await save.boundingBox())!;
      expect(idle.height).toBeCloseTo(30.84375, 2);
      await save.focus();
      await save.press('Enter');
      await expect(save).toHaveAttribute('aria-busy', 'true');
      await expect(save).toHaveJSProperty('disabled', false);
      await expect(save).toBeFocused();
      await save.press('Enter');
      await expect(page.locator('[data-source-activations]')).toHaveAttribute(
        'data-source-activations',
        '1',
      );
      expect((await save.boundingBox())!.height).toBe(idle.height);
      expect((await save.boundingBox())!.width).toBe(idle.width);
      await page.getByRole('button', { name: 'Close Source', exact: true }).click();
      await expect(page.getByRole('dialog')).toBeHidden();
      await page.goto(`/?frame=source&legacy&theme=${theme}`);
      await expect(page.getByRole('button', { name: 'Save', exact: true })).toHaveCSS(
        'font-size',
        '14px',
      );
      await expect(page.getByRole('button', { name: 'Save', exact: true })).toHaveCSS(
        'height',
        '37px',
      );
      await page.goto(`/?frame=panel&header-action&theme=${theme}`);
      await expect(page.getByRole('button', { name: 'Save', exact: true })).toHaveCSS(
        'font-size',
        '11px',
      );
      expect(
        (await page.getByRole('button', { name: 'Save', exact: true }).boundingBox())!.height,
      ).toBeCloseTo(30.84375, 2);
      await expect(page.getByRole('button', { name: 'Reply', exact: true })).toHaveCSS(
        'font-size',
        '14px',
      );
    });
    test(`notice typography ${width} ${theme}: body eyebrow and preserved state words`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 992 });
      await page.goto(`/?frame=notice&theme=${theme}`);
      const notices = page.locator(
        `.gallery-column[data-theme="${theme}"] .tmt-ui-notice:has(.tmt-ui-notice-eyebrow)`,
      );
      expect(await notices.count()).toBeGreaterThan(5);
      for (const notice of await notices.all()) {
        const eyebrow = notice.locator('.tmt-ui-notice-eyebrow');
        await expect(eyebrow).toHaveCSS('font-size', '10px');
        await expect(eyebrow).toHaveCSS('font-weight', '400');
        await expect(eyebrow).toHaveCSS('letter-spacing', '1.8px');
        await expect(eyebrow).toHaveCSS('text-transform', 'uppercase');
        const body = await notice.evaluate((node) => getComputedStyle(node).fontFamily);
        await expect(eyebrow).toHaveCSS('font-family', body);
        const state = notice.locator('.tmt-ui-notice-mark > :not([aria-hidden])');
        await expect(state).toHaveCSS('font-family', body);
        await expect(state).toHaveCSS('font-size', '14px');
        await expect(state).toHaveCSS('font-weight', '400');
      }
    });
  }

for (const theme of ['light', 'dark'] as const)
  test(`framework-free glyphs ${theme}: decorative, current colour and one em`, async ({
    page,
  }) => {
    await page.goto(`/?frame=glyphs&theme=${theme}`);
    for (const size of [14, 24])
      for (const name of ['link', 'info']) {
        const host = page.locator(`[data-size="${size}"] [data-glyph="${name}"]`);
        const icon = host.locator('svg');
        await expect(icon).toHaveAttribute('aria-hidden', 'true');
        await expect(icon).toHaveAttribute('viewBox', '0 0 24 24');
        await expect(icon).toHaveCSS('width', `${size}px`);
        await expect(icon).toHaveCSS('height', `${size}px`);
        await expect(icon).toHaveCSS(
          'stroke',
          await host.evaluate((node) => getComputedStyle(node).color),
        );
        await expect(icon).toHaveCSS('fill', 'none');
        await expect(icon).toHaveCSS('stroke-width', '2px');
      }
    await expect(page.getByRole('img')).toHaveCount(0);
  });
