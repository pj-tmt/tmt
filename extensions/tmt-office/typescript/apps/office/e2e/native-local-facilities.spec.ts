import { expect, test } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import { openOfficeObjects } from './office-navigation.js';
import { installationWorldPoint } from './native-world-geometry.js';
import type { WorldSnapshot } from '../src/world-map/world-port.js';

test('new Lobby facility artwork retains the three real resource entry points without rewriting the world', async ({
  page,
}, info) => {
  await withSandbox(async (sandbox) => {
    const prefix = await installNativeOffice(sandbox);
    async function office<T>(args: string[]): Promise<T> {
      const result = await runCli(sandbox, ['office', '--prefix', prefix, ...args, '--json'], {
        deadlineMs: 30_000,
      });
      expect(result.status, result.stdout + result.stderr).toBe(0);
      return JSON.parse(result.stdout) as T;
    }
    const before = await office<WorldSnapshot>(['layout', 'show']);
    const facilities = before.layout.objects.filter((object) => object.extension);
    expect(
      facilities.map((object) => [
        object.extension!.definition,
        object.placement.prop.split('/')[1],
        object.placement.footprint,
      ])
    ).toEqual([
      ['tmt-discussion-board', 'lobby-discussion-board', { width: 12, height: 12 }],
      ['tmt-whiteboard', 'lobby-whiteboard', { width: 16, height: 16 }],
      ['tmt-broadcaster', 'lobby-radio', { width: 8, height: 8 }],
    ]);
    const started = await office<{ url: string }>([
      'start',
      '--port',
      String(await unusedLoopbackPort()),
    ]);
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    try {
      await page.setViewportSize({ width: 1536, height: 1024 });
      const enter = async () => {
        await page.goto('about:blank');
        await page.goto(started.url);
        await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      };
      await enter();
      await page.getByRole('button', { name: 'Fit office', exact: true }).click();
      await page.screenshot({ path: info.outputPath('new-lobby-facilities.png') });
      // Independent v6 whiteboard body and expanded action label, not the production picker.
      const bounds = (await page.locator('.office-canvas canvas').boundingBox())!;
      const body = installationWorldPoint(bounds, 106, 21.75);
      await page.mouse.move(body.x, body.y);
      await expect(page.locator('.office-canvas canvas')).toHaveCSS('cursor', 'grab');
      const action = installationWorldPoint(bounds, 106, 12.75);
      await page.mouse.move(action.x, action.y);
      await expect(page.locator('.office-canvas canvas')).toHaveCSS('cursor', 'pointer');
      await page.screenshot({ path: info.outputPath('whiteboard-hover-control.png') });
      await page.mouse.click(action.x, action.y);
      await expect(page.getByRole('dialog', { name: 'Whiteboard', exact: true })).toBeVisible();
      for (const [action, panel] of [
        ['Open discussion board', 'Discussion board'],
        ['Open whiteboard', 'Whiteboard'],
        ['Compose announcement', 'Broadcast station'],
      ]) {
        await enter();
        await openOfficeObjects(page);
        await page.getByRole('button', { name: action, exact: true }).click();
        await expect(page.getByRole('dialog', { name: panel, exact: true })).toBeVisible();
        expect((await office<WorldSnapshot>(['layout', 'show'])).layout).toEqual(before.layout);
      }
      expect(errors).toEqual([]);
    } finally {
      await office(['stop']);
    }
  });
});
