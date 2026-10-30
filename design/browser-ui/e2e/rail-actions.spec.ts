import { expect, test } from '@playwright/test';

for (const renderer of ['native', 'react'])
  for (const theme of ['light', 'dark'])
    for (const width of [1440, 390])
      test(`rail row actions ${renderer} ${theme} ${width}`, async ({ page }, testInfo) => {
        await page.setViewportSize({ width, height: 1000 });
        await page.goto(`/?frame=rail&interactive=true&renderer=${renderer}&theme=${theme}`);
        const rail = page.locator('nav.tmt-ui-rail');
        const open = async () => {
          if (width === 390 && !(await rail.isVisible()))
            await page.getByRole('button', { name: 'Open navigation', exact: true }).click();
        };
        await open();
        const row = (id: string) => rail.locator(`li[data-menu-item="${id}"]`);
        const link = (id: string) => row(id).locator('a');
        const host = page.locator('[data-host-order]');
        const command = async (name: string) => {
          await page
            .getByRole('button', { name, exact: true })
            .evaluate((node) => (node as HTMLButtonElement).click());
        };
        const capture = async (state: string) => {
          await testInfo.attach(`rail-actions-${renderer}-${theme}-${width}-${state}`, {
            body: await page.screenshot(),
            contentType: 'image/png',
          });
        };
        await page.mouse.move(width - 5, 995);
        await expect(
          row('empty').getByRole('button', { name: 'Edit Remote', exact: true }),
        ).toBeHidden();
        await expect(link('empty').locator('.tmt-ui-shell-count')).toBeVisible();
        const geometry = await link('empty').boundingBox();
        await capture('idle');
        await link('empty').hover();
        const edit = row('empty').getByRole('button', { name: 'Edit Remote', exact: true });
        await expect(edit).toBeVisible();
        expect((await edit.boundingBox())!.width).toBeGreaterThanOrEqual(28);
        expect((await edit.boundingBox())!.height).toBeGreaterThanOrEqual(28);
        const editIcon = edit.locator('svg');
        await expect(editIcon).toHaveCSS('width', '16px');
        await expect(editIcon).toHaveCSS('height', '16px');
        await expect(editIcon).toHaveCSS('stroke-width', '1.75px');
        const longLabel = link('project').locator('.tmt-ui-module-menu-label');
        const originalLabel = await longLabel.textContent();
        await longLabel.evaluate((node) => {
          node.textContent = 'A long shortcut name '.repeat(6);
        });
        const labelBox = (await longLabel.boundingBox())!;
        const actionsLeft = await row('project')
          .locator('.tmt-ui-rail-row-actions')
          .evaluate((node) => node.getBoundingClientRect().left);
        expect(labelBox.x + labelBox.width).toBeLessThanOrEqual(actionsLeft);
        await longLabel.evaluate((node, value) => {
          node.textContent = value;
        }, originalLabel);
        await expect(link('empty').locator('.tmt-ui-shell-count')).toBeHidden();
        expect(await link('empty').boundingBox()).toEqual(geometry);
        await capture('hover');
        await page.mouse.move(width - 5, 995);
        await link('empty').focus();
        await expect(edit).toBeVisible();
        await capture('focus');
        await command('Refresh command bindings');
        await expect(link('empty')).toBeFocused();
        await edit.focus();
        await page.keyboard.press('Enter');
        await expect(page.locator('[data-host-status]')).toHaveText('Edit empty 1');
        await expect(host).toHaveAttribute('data-action-calls', '1');
        await open();
        await expect(rail.locator('[aria-current="page"]')).toHaveAttribute('data-id', 'project');
        await expect(rail.locator('a button')).toHaveCount(0);
        await link('empty').focus();
        await page.keyboard.press('Alt+ArrowUp');
        await expect(host).toHaveAttribute('data-host-order', 'empty,project,notes');
        await expect(link('empty')).toBeFocused();
        await expect(page.locator('.tmt-ui-shell > [role="status"]')).toHaveText(
          'Move requested: Remote, position 1 of 3.',
        );
        await page.keyboard.press('Alt+ArrowUp');
        await expect(host).toHaveAttribute('data-reorder-requests', '1');
        await expect(page.locator('.tmt-ui-shell > [role="status"]')).toHaveText(
          'Remote is already first.',
        );
        await page.keyboard.press('Alt+ArrowDown');
        await expect(host).toHaveAttribute('data-host-order', 'project,empty,notes');
        await command('Toggle held reorder requests');
        await page.keyboard.press('Alt+ArrowUp');
        await expect(host).toHaveAttribute('data-reorder-requests', '3');
        await expect(host).toHaveAttribute('data-host-order', 'project,empty,notes');
        expect(
          await rail
            .locator('li[data-menu-item]')
            .evaluateAll((nodes) => nodes.map((node) => (node as HTMLElement).dataset.menuItem)),
        ).toEqual(['project', 'empty', 'notes']);
        await command('Toggle held reorder requests');
        await command('Toggle reorder availability');
        await expect(row('empty').locator('[data-rail-part="drag"]')).toHaveCount(0);
        await link('empty').focus();
        await page.keyboard.press('Alt+ArrowUp');
        await expect(host).toHaveAttribute('data-reorder-requests', '3');
        await command('Toggle reorder availability');
        await page.mouse.move(width - 5, 995);
        await row('notes').hover();
        const handle = row('notes').locator('[data-rail-part="drag"]');
        await expect(handle.locator('svg circle')).toHaveCount(6);
        const start = (await handle.boundingBox())!;
        const target = (await link('project').boundingBox())!;
        await page.mouse.move(start.x + start.width / 2, start.y + start.height / 2);
        await page.mouse.down();
        await page.mouse.move(target.x + target.width / 2, target.y + target.height / 2, {
          steps: 12,
        });
        await page.mouse.move(target.x + target.width / 2 + 1, target.y + target.height / 2);
        await expect(row('notes')).toHaveAttribute('data-dragging', '');
        await expect(row('project')).toHaveAttribute('data-drop-edge', 'before');
        await capture('drag');
        await page.mouse.up();
        await expect(host).toHaveAttribute('data-host-order', 'notes,project,empty');
        await expect(host).toHaveAttribute('data-reorder-requests', '4');
        await expect(rail.locator('[aria-current="page"]')).toHaveCount(1);
        await expect(rail.locator('[aria-current="page"]')).toHaveAttribute('data-id', 'project');
        // An external drop cannot manufacture a move without this live group's own handle.
        await row('empty').dispatchEvent('drop', { bubbles: true });
        await expect(host).toHaveAttribute('data-reorder-requests', '4');
        await link('empty').focus();
        const retained = await edit.elementHandle();
        await command('Unmount rail');
        await expect(rail).toHaveCount(0);
        await retained!.evaluate((node) => (node as HTMLButtonElement).click());
        await expect(page.locator('[data-action-calls]')).toHaveAttribute('data-action-calls', '1');
        await expect(page.locator('.tmt-ui-shell > [role="status"]')).toHaveCount(0);
      });
