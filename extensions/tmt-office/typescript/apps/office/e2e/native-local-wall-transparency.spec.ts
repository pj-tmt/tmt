import { writeFileSync } from 'node:fs';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { officeWorldFixture } from '../../../../../../typescript/test/support/office-world.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import { fitWorldCoordinates } from './world-editor-gesture.js';
import { openKeyboardSelection } from './office-navigation.js';

test('selects and drags furniture behind a full front wall and Undo restores it', async ({
  page,
}, info) => {
  await withSandbox(async (sandbox) => {
    const prefix = await installNativeOffice(sandbox);
    const office = async (args: string[]) => {
      const result = await runCli(sandbox, ['office', '--prefix', prefix, ...args, '--json']);
      expect(result.status, result.stdout + result.stderr).toBe(0);
      return JSON.parse(result.stdout);
    };
    const initial = await office(['layout', 'show']);
    const layout = officeWorldFixture().layout;
    const desk = layout.objects[0]!;
    desk.placement.x = 10;
    desk.placement.y = 32;
    const file = path.join(sandbox.root, 'occluded-desk.json');
    writeFileSync(file, JSON.stringify(layout));
    await office([
      'layout',
      'apply',
      '--file',
      file,
      '--if-revision',
      '0',
      '--legacy-basis',
      initial.legacyBasis,
    ]);
    const before = await office(['layout', 'show']);
    const started = await office(['start', '--port', String(await unusedLoopbackPort())]);
    try {
      await page.setViewportSize({ width: 1536, height: 1024 });
      await page.goto(started.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      // Independent fixture projection: 36x36 floor at 5/8 depth; 16-high walls.
      // Desk's center (12,20.25) lies behind the front face spanning y=6.5..22.5.
      const point = await fitWorldCoordinates(page, { x: -4, y: -20, width: 44, height: 50.5 });
      const start = point(12, 20.25);
      await page.mouse.click(start.x, start.y);
      await openKeyboardSelection(page);
      await expect(page.getByRole('combobox', { name: 'Object', exact: true })).toHaveValue(
        desk.id
      );
      const end = point(18, 20.25);
      await page.mouse.move(start.x, start.y);
      await page.mouse.down();
      await page.mouse.move(end.x, end.y, { steps: 4 });
      await page.mouse.up();
      await page.getByText('Precise placement', { exact: true }).click();
      await expect(page.getByRole('spinbutton', { name: 'X', exact: true })).toHaveValue('16');
      await expect(page.getByRole('spinbutton', { name: 'Y', exact: true })).toHaveValue('32');
      await page.screenshot({ path: info.outputPath('drag-behind-wall.png') });
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      expect(await office(['layout', 'show'])).not.toEqual(before);
      await page.getByRole('button', { name: 'Undo', exact: true }).click();
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      const restored = await office(['layout', 'show']);
      expect(restored).toMatchObject({ worldId: before.worldId, layout: before.layout });
      expect(restored.revision).toBeGreaterThan(before.revision);
      await page.screenshot({ path: info.outputPath('undo-restores-solid-wall.png') });
    } finally {
      await page.goto('about:blank');
      await office(['stop']);
    }
  });
});
