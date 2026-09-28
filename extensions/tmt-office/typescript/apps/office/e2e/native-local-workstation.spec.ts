import { writeFileSync } from 'node:fs';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import { savedWorld } from './native-world-state.js';
import { legacyLobbyObjects } from '../../../../../../typescript/test/support/office-world.js';
import { captureWorldScene, installDrawObserver, observeIdleScene } from './scene-observation.js';
import vectors from '../../../../../../contracts/office/modules-central-grid-vectors.json' with { type: 'json' };
import { decodeModuleMap } from '../src/world-map/module-contract.js';
import {
  MODULAR_WORKSTATION_DIGEST,
  MODULAR_MOUNTED_DIGEST,
  DIRECTIONAL_WORKSTATION_DIGEST,
} from '../src/props/prop-contract.js';
import type { WorldSnapshot } from '../src/world-map/world-port.js';
import { openKeyboardSelection } from './office-navigation.js';

test('reviewed workstation and wall art use real catalog placement, four chair views and native persistence', async ({
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
    const initial = await office<WorldSnapshot>(['layout', 'show']);
    const map = decodeModuleMap({ ...vectors.map, modules: vectors.map.modules.slice(0, 5) });
    const layout = {
      ...initial.layout,
      map,
      objects: legacyLobbyObjects().map((object) => ({
        ...object,
        placement: { ...object.placement, x: object.placement.x + 32, y: object.placement.y + 20 },
      })),
    };
    const file = path.join(sandbox.root, 'workstation.json');
    writeFileSync(file, JSON.stringify(layout));
    await office([
      'layout',
      'apply',
      '--file',
      file,
      '--if-revision',
      '0',
      '--legacy-basis',
      initial.legacyBasis!,
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
      await installDrawObserver(page);
      await page.goto(started.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      await openKeyboardSelection(page);
      await page
        .getByRole('combobox', { name: 'Area', exact: true })
        .selectOption(map.modules[1]!.area.id);
      // Placement order is intentional: chair, desk, then equipment on the desk.
      for (const [name, x, y] of [
        ['Workshop rolling chair', 14, -36],
        ['Workshop writing desk', 10, -33],
        ['Workshop terminal', 14, -33],
        ['Workshop bookcase', 30, -44],
      ] as const) {
        await page.getByRole('button', { name: 'Clear selection', exact: true }).click();
        await page.getByRole('button', { name, exact: true }).click();
        const precision = page.locator('.world-placement-details');
        if (!(await precision.evaluate((element) => (element as HTMLDetailsElement).open)))
          await precision.locator('summary').click();
        const coordinates = page.getByRole('form', { name: 'Object coordinates' });
        await coordinates.getByRole('spinbutton', { name: 'X', exact: true }).fill(String(x));
        await coordinates.getByRole('spinbutton', { name: 'Y', exact: true }).fill(String(y));
        await coordinates.getByRole('button', { name: 'Apply coordinates' }).click();
      }
      for (const [name, x, elevation, area] of [
        ['Workshop celestial window', 16, 0, 1],
        ['Workshop brass sconce', 38, 5, 1],
        ['Workshop picture frame', 2, 4, 1],
        ['Workshop planted shelf', 66, 3, 2],
      ] as const) {
        await page
          .getByRole('combobox', { name: 'Area', exact: true })
          .selectOption(map.modules[area]!.area.id);
        await page.getByRole('button', { name: 'Clear selection', exact: true }).click();
        await page.getByRole('button', { name, exact: true }).click();
        const precision = page.locator('.world-placement-details');
        if (!(await precision.evaluate((element) => (element as HTMLDetailsElement).open)))
          await precision.locator('summary').click();
        const coordinates = page.getByRole('form', { name: 'Object coordinates' });
        await coordinates.getByRole('spinbutton', { name: 'X', exact: true }).fill(String(x));
        await coordinates.getByRole('spinbutton', { name: 'Y', exact: true }).fill('-48');
        await coordinates
          .getByRole('spinbutton', { name: 'Elevation', exact: true })
          .fill(String(elevation));
        await coordinates.getByRole('button', { name: 'Apply coordinates' }).click();
      }
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      const saved = await office<WorldSnapshot>(['layout', 'show']);
      expect(saved.layout.map).toEqual(map);
      expect(saved.layout.objects.slice(0, layout.objects.length)).toEqual(layout.objects);
      const additions = saved.layout.objects.slice(layout.objects.length);
      expect(additions.slice(0, 4).map((object) => object.placement.prop)).toEqual(
        [
          'workstation-chair',
          'workstation-desk',
          'workstation-terminal',
          'workstation-bookcase',
        ].map(
          (key) =>
            `${key === 'workstation-chair' ? MODULAR_WORKSTATION_DIGEST : DIRECTIONAL_WORKSTATION_DIGEST}/${key}`
        )
      );
      expect(
        additions.slice(4).map((object) => ({
          kind: object.kind,
          surface: object.surface,
          reference: object.placement.prop,
          x: object.placement.x,
          y: object.placement.y,
          extension: object.extension,
        }))
      ).toEqual(
        [
          ['mounted-window', 'window', 16, 0],
          ['mounted-sconce', 'wallLight', 38, 5],
          ['mounted-frame', 'decoration', 2, 4],
          ['mounted-shelf', 'decoration', 66, 3],
        ].map(([key, kind, x, elevation]) => ({
          kind,
          surface: { type: 'wall', axis: 'horizontal', face: 'positive', elevation },
          reference: `${MODULAR_MOUNTED_DIGEST}/${key}`,
          x,
          y: -48,
          extension: null,
        }))
      );
      expect(JSON.parse(savedWorld(sandbox.database).layout)).toEqual(saved.layout);
      // Fixed native fixture camera: bring the north-west module near center,
      // then enlarge the real world without hiding walls or changing geometry.
      await page.mouse.move(530, 240);
      await page.mouse.down();
      await page.mouse.move(760, 490);
      await page.mouse.up();
      await page.getByRole('button', { name: 'Zoom in', exact: true }).click();
      await page.getByRole('button', { name: 'Zoom in', exact: true }).click();
      await page.screenshot({ path: info.outputPath('workstation-module-1536.png') });
      await openKeyboardSelection(page);
      await page
        .getByRole('combobox', { name: 'Object', exact: true })
        .selectOption(additions[0]!.id);
      const frames = [await captureWorldScene(page, info, 'chair-south.png')];
      if ((await page.locator('.world-placement-details').getAttribute('open')) === null)
        await page.getByText('Precise placement', { exact: true }).click();
      for (const direction of ['west', 'north', 'east']) {
        await page.getByRole('button', { name: 'Rotate 90° clockwise', exact: true }).click();
        frames.push(await captureWorldScene(page, info, `chair-${direction}.png`));
      }
      expect(new Set(frames.map((frame) => frame.toString('base64'))).size).toBe(4);
      for (let rotation = 0; rotation < 3; rotation++)
        await page.getByRole('button', { name: 'Undo', exact: true }).click();
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      expect(JSON.parse(savedWorld(sandbox.database).layout)).toEqual(saved.layout);
      await observeIdleScene(page, info, 'workstation');
      await page.goto('about:blank');
      await page.goto(started.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      expect(JSON.parse(savedWorld(sandbox.database).layout)).toEqual(saved.layout);
      expect(errors).toEqual([]);
    } finally {
      await office(['stop']);
    }
  });
});
