import { expect, test } from '@playwright/test';

for (const renderer of ['native', 'react'])
  for (const theme of ['light', 'dark'])
    for (const width of [1440, 390])
      test(`shared rail ${renderer} ${theme} ${width}: selection, counts, commands and lifetime`, async ({
        page,
      }, testInfo) => {
        await page.setViewportSize({ width, height: 1000 });
        await page.goto(`/?frame=rail&renderer=${renderer}&theme=${theme}`);
        const rail = page.locator('nav.tmt-ui-rail');
        const open = page.getByRole('button', { name: 'Open navigation', exact: true });
        if (width === 390) await open.click();
        const selected = rail.locator('[aria-current="page"]');
        await expect(selected).toHaveCount(1);
        await expect(selected).toHaveAttribute('data-id', 'all');
        const module = rail.locator('[data-id="colab"]');
        await expect(module).not.toHaveAttribute('aria-current');
        await expect(module).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
        expect(await module.evaluate((node) => getComputedStyle(node, '::before').content)).toBe(
          'none',
        );
        await expect(module).toHaveCSS('font-weight', '600');
        await expect(rail.locator('[data-id="home"] .tmt-ui-shell-count')).toHaveCount(0);
        await expect(rail.locator('[data-id="remote"] .tmt-ui-shell-count')).toHaveCount(0);
        await expect(rail.locator('[data-id="empty"] .tmt-ui-shell-count')).toHaveCount(0);
        const counts = rail.locator('[data-id="all"] .tmt-ui-shell-counts');
        await expect(counts).toHaveCSS('gap', '4px');
        const paints = () =>
          counts.locator('.tmt-ui-shell-count').evaluateAll((nodes) =>
            nodes.map((node) => {
              const css = getComputedStyle(node);
              return [css.color, css.backgroundColor, css.borderRadius];
            }),
          );
        const original = await paints();
        expect(original[0]![1]).not.toBe(original[1]![1]);
        const pillLook = await counts.evaluate((node) => {
          const total = node.children[0] as HTMLElement;
          const attention = node.children[1] as HTMLElement;
          const rail = node.closest('.tmt-ui-rail')!;
          const row = node.closest('a')!;
          const canvas = document.createElement('canvas');
          canvas.width = canvas.height = 1;
          const context = canvas.getContext('2d')!;
          const rgb = (color: string) => {
            context.fillStyle = color;
            context.fillRect(0, 0, 1, 1);
            return [...context.getImageData(0, 0, 1, 1).data].slice(0, 3);
          };
          const light = (channels: number[]) =>
            channels
              .map((n) => n / 255)
              .map((n) => (n <= 0.04045 ? n / 12.92 : ((n + 0.055) / 1.055) ** 2.4))
              .reduce((sum, n, i) => sum + n * [0.2126, 0.7152, 0.0722][i]!, 0);
          const contrast = (a: number[], b: number[]) =>
            (Math.max(light(a), light(b)) + 0.05) / (Math.min(light(a), light(b)) + 0.05);
          const css = getComputedStyle(total);
          const fill = rgb(css.backgroundColor);
          const metrics = (element: HTMLElement) => {
            const style = getComputedStyle(element);
            return {
              height: element.getBoundingClientRect().height,
              padding: style.padding,
              font: style.fontFamily,
              size: style.fontSize,
              minimum: style.minWidth,
            };
          };
          return {
            total: metrics(total),
            attention: metrics(attention),
            body: getComputedStyle(rail).fontFamily,
            railContrast: contrast(fill, rgb(getComputedStyle(rail).backgroundColor)),
            selectionContrast: contrast(fill, rgb(getComputedStyle(row).backgroundColor)),
            textContrast: contrast(fill, rgb(css.color)),
            neutralSpread: Math.max(...fill) - Math.min(...fill),
          };
        });
        expect(pillLook.total).toEqual(pillLook.attention);
        expect(pillLook.total.font).toBe(pillLook.body);
        expect(pillLook.railContrast).toBeGreaterThanOrEqual(1.2);
        expect(pillLook.selectionContrast).toBeGreaterThanOrEqual(1.2);
        expect(pillLook.textContrast).toBeGreaterThanOrEqual(4.5);
        expect(pillLook.neutralSpread).toBeLessThanOrEqual(32);

        await expect(counts.locator('[data-kind="needs-you"]')).toHaveText('3');
        const allLabel = rail.locator('[data-id="all"] .tmt-ui-module-menu-label');
        const projectLabel = rail.locator('[data-id="project"] .tmt-ui-module-menu-label');
        const heading = rail.locator('.tmt-ui-rail-sub-group > span');
        const x = (node: typeof allLabel) =>
          node.evaluate((element) => element.getBoundingClientRect().x);
        expect(await x(projectLabel)).toBe(await x(allLabel));
        expect(await x(heading)).toBe(await x(allLabel));
        expect((await x(allLabel)) - (await selected.boundingBox())!.x).toBe(36);
        await expect(projectLabel).toHaveAttribute('title', 'Release planning and coordination');
        await expect(projectLabel).toHaveCSS('text-overflow', 'ellipsis');
        await expect(selected).toHaveCSS('border-left-width', '0px');
        await expect(selected).toHaveCSS(
          'border-radius',
          await module.evaluate((node) => getComputedStyle(node).borderRadius),
        );
        const settings = rail.locator('.tmt-ui-rail-sub > li:last-child');
        await expect(settings).toHaveClass('tmt-ui-rail-settings');
        expect(
          await settings.evaluate((node) => {
            const css = getComputedStyle(node, '::before');
            return { left: css.left, width: css.borderTopWidth };
          }),
        ).toEqual({ left: '36px', width: '1px' });
        await testInfo.attach(`rail-${renderer}-${theme}-${width}`, {
          body: await page.screenshot(),
          contentType: 'image/png',
        });
        await rail.locator('[data-id="project"]').click();
        if (width === 390) await open.click();
        await expect(selected).toHaveCount(1);
        await expect(selected).toHaveAttribute('data-id', 'project');
        expect(await paints()).toEqual(original);
        const manage = rail.getByRole('button', { name: 'Manage projects', exact: true });
        await manage.focus();
        await page
          .getByRole('button', { name: 'Refresh command bindings' })
          .evaluate((node) => (node as HTMLButtonElement).click());
        await expect(manage).toBeFocused();
        await page.keyboard.press('Enter');
        await expect(page.getByRole('status')).toHaveText('Manage 1');
        if (width === 390) await open.click();
        const add = rail.getByRole('button', { name: 'New project', exact: true });
        await expect(add).not.toHaveAttribute('aria-current');
        const plus = add.locator('svg');
        await expect(plus).toHaveAttribute('aria-hidden', 'true');
        await expect(plus).toHaveCSS('width', '14px');
        await expect(plus).toHaveCSS('height', '14px');
        await add.focus();
        await page.keyboard.press('Enter');
        await expect(page.getByRole('status')).toHaveText('New 1');
        await page.getByRole('button', { name: 'Select module', exact: true }).click();
        if (width === 390) await open.click();
        await expect(selected).toHaveCount(1);
        await expect(selected).toHaveAttribute('data-id', 'colab');
        if (width === 390) {
          await page.keyboard.press('Escape');
          await expect(open).toBeFocused();
          await expect(rail).toBeHidden();
        }
        await page.getByRole('button', { name: 'Unmount rail', exact: true }).click();
        await expect(rail).toHaveCount(0);
        await expect(page.getByRole('tooltip')).toHaveCount(0);
      });
