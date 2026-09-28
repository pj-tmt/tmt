import { writeFileSync } from 'node:fs';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import { savedWorld } from './native-world-state.js';
import { legacyLobbyObjects } from '../../../../../../typescript/test/support/office-world.js';
import {
  installDrawObserver,
  observeIdleScene,
  sceneActivity,
  captureWorldScene,
} from './scene-observation.js';
import vectors from '../../../../../../contracts/office/modules-central-grid-vectors.json' with { type: 'json' };
import { decodeModuleMap } from '../src/world-map/module-contract.js';
import { moduleBounds } from '../src/world-map/module-geometry.js';
import { workshopStarter } from '../src/blocks/workshop-starter.js';
import type { WorldObject } from '../src/world-map/world-contract.js';
import type { WorldSnapshot } from '../src/world-map/world-port.js';
import { openKeyboardSelection } from './office-navigation.js';

test('module finishes change real pixels while preserving topology, objects and bounded texture ownership', async ({
  page,
}, info) => {
  test.setTimeout(120_000);
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
    const objects: WorldObject[] = legacyLobbyObjects().map((object) => ({
      ...object,
      placement: { ...object.placement, x: object.placement.x + 32, y: object.placement.y + 20 },
    }));
    for (const module of map.modules.slice(1)) {
      const bounds = moduleBounds(module, map.version);
      for (const placement of workshopStarter('study'))
        objects.push({
          id: crypto.randomUUID(),
          kind: 'decoration',
          surface: { type: 'floor' },
          extension: null,
          placement: { ...placement, x: placement.x + bounds.x + 4, y: placement.y + bounds.y + 4 },
        });
    }
    const layout = { ...initial.layout, map, objects };
    const file = path.join(sandbox.root, 'room-materials.json');
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
      const area = page.getByRole('combobox', { name: 'Area', exact: true });
      await area.selectOption(map.modules[1]!.area.id);
      for (const name of ['Observatory window', 'Brass wall lamp']) {
        await page.getByRole('button', { name: 'Clear selection', exact: true }).click();
        await page.getByRole('button', { name, exact: true }).click();
      }
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      const original = await office<WorldSnapshot>(['layout', 'show']);
      const before = savedWorld(sandbox.database);
      const nextWorldWrite = () =>
        page.waitForResponse(
          (response) =>
            new URL(response.url()).pathname === '/api/v1/local/world' &&
            response.request().method() === 'PUT'
        );
      const persist = async (action: () => Promise<void>) => {
        const write = nextWorldWrite();
        await action();
        expect((await write).status()).toBe(200);
        await expect(
          page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
        ).toHaveText('All changes applied');
      };
      await area.selectOption(map.modules[1]!.area.id);
      const style = (name: string) => page.getByRole('radio', { name, exact: true });
      await expect(style('Workshop')).toBeChecked();
      const conversion = page.getByRole('region', { name: 'Compact layout preview', exact: true });
      await expect(conversion.locator('details')).not.toHaveAttribute('open', '');
      const nameBox = await page
        .getByRole('textbox', { name: 'Area name', exact: true })
        .boundingBox();
      const conversionBox = await conversion.boundingBox();
      expect(nameBox!.y + nameBox!.height).toBeLessThan(conversionBox!.y);
      const previews = page.locator('.room-material-choices img');
      await expect(previews).toHaveCount(3);
      const expectPreviewRow = async () => {
        const cards = await page.locator('.room-material-choices label').evaluateAll((labels) =>
          labels.map((label) => {
            const box = label.getBoundingClientRect();
            const parent = label.parentElement!.getBoundingClientRect();
            return {
              top: box.top,
              contained: box.left >= parent.left && box.right <= parent.right,
            };
          })
        );
        expect(cards).toHaveLength(3);
        expect(new Set(cards.map((card) => card.top)).size).toBe(1);
        expect(cards.every((card) => card.contained)).toBe(true);
      };
      await expectPreviewRow();
      expect(
        new Set(
          await previews.evaluateAll((images) =>
            images.map((image) => (image as HTMLImageElement).src)
          )
        ).size
      ).toBe(3);
      const worldPixels = (name: string) => captureWorldScene(page, info, `style-${name}.png`);
      await page.mouse.move(10, 10);
      const baseline = await worldPixels('workshop');
      await persist(() => style('Moonlight').check());
      const moonlight = await worldPixels('moonlight');
      expect(moonlight.equals(baseline)).toBe(false);
      await persist(() => page.getByRole('button', { name: 'Undo', exact: true }).click());
      await expect(style('Workshop')).toBeChecked();
      const undone = await worldPixels('undo');
      expect(undone.equals(baseline)).toBe(true);
      await persist(() => page.getByRole('button', { name: 'Redo', exact: true }).click());
      expect((await worldPixels('redo')).equals(moonlight)).toBe(true);
      await persist(() => style('Copper').check());
      const copper = await worldPixels('copper');
      expect(copper.equals(moonlight)).toBe(false);
      expect(copper.equals(baseline)).toBe(false);
      const warm = await sceneActivity(page);
      for (const material of ['Workshop', 'Moonlight', 'Copper', 'Moonlight', 'Workshop'])
        await persist(() => style(material).check());
      const repeated = await sceneActivity(page);
      expect(repeated.texturesCreated).toBe(warm.texturesCreated);
      expect(repeated.texturesLive).toBe(warm.texturesLive);
      const restored = savedWorld(sandbox.database);
      expect(restored.worldId).toBe(before.worldId);
      expect(restored.revision).toBeGreaterThan(before.revision);
      expect(restored.updatedAtMs).toBeGreaterThanOrEqual(before.updatedAtMs);
      expect(JSON.parse(restored.layout)).toEqual(JSON.parse(before.layout));

      await area.selectOption(map.modules[1]!.area.id);
      await persist(() => style('Moonlight').check());
      await area.selectOption(map.modules[2]!.area.id);
      await persist(() => style('Copper').check());
      await page.screenshot({ path: info.outputPath('room-materials-editor.png') });
      const saved = await office<WorldSnapshot>(['layout', 'show']);
      expect(saved.layout.objects).toEqual(original.layout.objects);
      expect(saved.layout.map).toEqual({
        ...map,
        modules: map.modules.map((module, index) => ({
          ...module,
          material: index === 1 ? 'moonlight' : index === 2 ? 'copper' : 'workshop',
        })),
      });
      expect(JSON.parse(savedWorld(sandbox.database).layout)).toEqual(saved.layout);
      await page.getByRole('button', { name: 'Fit office', exact: true }).click();
      await page.screenshot({ path: info.outputPath('room-materials-1536.png') });
      await observeIdleScene(page, info, 'room-materials');
      await page.goto('about:blank');
      await page.goto(started.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      await openKeyboardSelection(page);
      await area.selectOption(map.modules[1]!.area.id);
      await expect(style('Moonlight')).toBeChecked();
      await area.selectOption(map.modules[2]!.area.id);
      await expect(style('Copper')).toBeChecked();
      await page.setViewportSize({ width: 390, height: 844 });
      await expectPreviewRow();
      await style('Copper').focus();
      await expect(style('Copper')).toBeFocused();
      await page.screenshot({ path: info.outputPath('room-materials-narrow.png') });
      expect(JSON.parse(savedWorld(sandbox.database).layout)).toEqual(saved.layout);
      expect(errors).toEqual([]);
    } finally {
      await office(['stop']);
    }
  });
});
