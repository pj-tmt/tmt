import { writeFileSync } from 'node:fs';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import { savedWorld } from './native-world-state.js';
import { legacyLobbyObjects } from '../../../../../../typescript/test/support/office-world.js';
import { installDrawObserver, observeIdleScene } from './scene-observation.js';
import { workshopStarter } from '../src/blocks/workshop-starter.js';
import vectors from '../../../../../../contracts/office/modules-central-grid-vectors.json' with { type: 'json' };
import { decodeModuleMap } from '../src/world-map/module-contract.js';
import { moduleBounds } from '../src/world-map/module-geometry.js';
import { addMeetingPreset } from '../src/world-map/meeting-preset.js';
import type { WorldDocument, WorldObject } from '../src/world-map/world-contract.js';
import type { WorldSnapshot } from '../src/world-map/world-port.js';
import { openKeyboardSelection } from './office-navigation.js';

/** A real stored layout with empty grid cells, not a screenshot-only mock world. */
test('central Lobby and roomless corridors retain independent meeting slots through browser Save', async ({
  page,
}, info) => {
  await withSandbox(async (sandbox) => {
    const prefix = await installNativeOffice(sandbox);
    async function cli<T>(args: string[]): Promise<T> {
      const result = await runCli(sandbox, [...args, '--json'], { deadlineMs: 30_000 });
      expect(result.status, result.stdout + result.stderr).toBe(0);
      return JSON.parse(result.stdout) as T;
    }
    const office = <T>(args: string[]) => cli<T>(['office', '--prefix', prefix, ...args]);
    const initial = await office<WorldSnapshot>(['layout', 'show']);
    const map = structuredClone(vectors.map);
    for (const module of map.modules)
      if (module.area.binding.type === 'meeting') {
        const { room } = await cli<{ room: { id: string } }>(['room', 'create', module.area.name]);
        module.area.binding.roomId = room.id;
      }
    const source = decodeModuleMap(map);
    const objects: WorldObject[] = legacyLobbyObjects().map((object) => ({
      ...object,
      placement: { ...object.placement, x: object.placement.x + 32, y: object.placement.y + 20 },
    }));
    for (const module of source.modules.filter((module) => module.slot.type === 'office')) {
      const bounds = moduleBounds(module, source.version);
      for (const placement of workshopStarter('study'))
        objects.push({
          id: crypto.randomUUID(),
          kind: 'decoration',
          surface: { type: 'floor' },
          extension: null,
          placement: { ...placement, x: placement.x + bounds.x + 4, y: placement.y + bounds.y + 4 },
        });
    }
    let layout: WorldDocument = { ...initial.layout, map: source, objects };
    for (const module of source.modules.filter((module) => module.slot.type === 'meeting'))
      layout = addMeetingPreset(layout, module.area.id);
    const file = path.join(sandbox.root, 'central-grid.json');
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
      await page.goto(started.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      expect(await page.evaluate(() => devicePixelRatio)).toBe(1);
      await page.getByRole('button', { name: 'Fit office', exact: true }).click();
      await page.screenshot({ path: info.outputPath('central-grid-overview.png') });
      await openKeyboardSelection(page);
      await page
        .getByRole('combobox', { name: 'Area', exact: true })
        .selectOption(source.primaryLobbyId);
      await page.getByRole('textbox', { name: 'Area name', exact: true }).fill('Central commons');
      const northWest = source.modules.find((module) => module.area.name === 'North west')!;
      await page
        .getByRole('combobox', { name: 'Area', exact: true })
        .selectOption(northWest.area.id);
      for (const name of ['Observatory window', 'Brass wall lamp']) {
        await page.getByRole('button', { name: 'Clear selection', exact: true }).click();
        await page.getByRole('button', { name, exact: true }).click();
      }
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      const saved = await office<WorldSnapshot>(['layout', 'show']);
      expect(saved.layout.map).toEqual({
        ...source,
        modules: source.modules.map((module) =>
          module.area.id === source.primaryLobbyId
            ? { ...module, area: { ...module.area, name: 'Central commons' } }
            : module
        ),
      });
      expect(saved.layout.objects.slice(0, layout.objects.length)).toEqual(layout.objects);
      const mounted = saved.layout.objects.slice(layout.objects.length);
      expect(mounted).toHaveLength(2);
      expect(mounted.map((object) => object.kind)).toEqual(['window', 'wallLight']);
      for (const object of mounted) {
        expect(object.surface).toMatchObject({
          type: 'wall',
          axis: 'horizontal',
          face: 'positive',
        });
        expect(object.placement.y).toBe(-48);
      }
      expect(JSON.parse(savedWorld(sandbox.database).layout)).toEqual(saved.layout);
      await page.getByRole('button', { name: 'Fit office', exact: true }).click();
      await page.screenshot({ path: info.outputPath('central-grid-mounted.png') });
      // The session stays in memory after the bootstrap fragment is removed.
      // Reopen through the issued entry URL, as in the other native fixtures.
      await page.goto('about:blank');
      await page.goto(started.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      await page.getByRole('button', { name: 'Office menu', exact: true }).click();
      await page.getByRole('button', { name: /^Directory · / }).click();
      await page.getByText('Available spaces', { exact: true }).click();
      await page.getByRole('button', { name: 'Office · column 0, row 3', exact: true }).click();
      await page.getByRole('button', { name: 'Fit office', exact: true }).click();
      await page.getByRole('textbox', { name: 'Name', exact: true }).fill('South studio');
      const form = page.getByRole('form', { name: 'New office' });
      await expect(form.getByRole('textbox', { name: 'Name', exact: true })).toBeFocused();
      const card = await form.boundingBox();
      expect(card).not.toBeNull();
      expect(card!.width).toBeLessThanOrEqual(240);
      expect(card!.height).toBeLessThan(260);
      await page.screenshot({ path: info.outputPath('central-grid-expansion.png') });
      await page.setViewportSize({ width: 390, height: 844 });
      await page.getByRole('button', { name: 'Fit office', exact: true }).click();
      const narrow = await form.boundingBox();
      expect(narrow!.x).toBeGreaterThanOrEqual(0);
      expect(narrow!.x + narrow!.width).toBeLessThanOrEqual(390);
      expect(narrow!.y + narrow!.height).toBeLessThanOrEqual(844);
      await page.screenshot({ path: info.outputPath('central-grid-expansion-narrow.png') });
      await form.getByRole('textbox', { name: 'Name', exact: true }).focus();
      await page.keyboard.press('Escape');
      await expect(form).toHaveCount(0);
      expect(JSON.parse(savedWorld(sandbox.database).layout)).toEqual(saved.layout);
      const highDpi = await page
        .context()
        .browser()!
        .newContext({
          viewport: { width: 1536, height: 1024 },
          deviceScaleFactor: 2,
        });
      try {
        const retina = await highDpi.newPage();
        retina.on('pageerror', (error) => errors.push(error.message));
        await installDrawObserver(retina);
        await retina.goto(started.url);
        await expect(retina.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
        expect(await retina.evaluate(() => devicePixelRatio)).toBe(2);
        await retina.getByRole('button', { name: 'Fit office', exact: true }).click();
        await retina.screenshot({ path: info.outputPath('central-grid-dpr2.png'), scale: 'css' });
        await observeIdleScene(retina, info, 'central-grid-dpr2');
      } finally {
        await highDpi.close();
      }
      expect(errors).toEqual([]);
    } finally {
      await office(['stop']);
    }
  });
});
