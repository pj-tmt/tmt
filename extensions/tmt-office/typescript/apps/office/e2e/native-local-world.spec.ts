import { mapGeometry } from '../src/world-map/map-source.js';
import Database from '../../../../../../typescript/test/support/sqlite-oracle.js';
import { expect, test } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import type { WorldSnapshot } from '../src/world-map/world-port.js';
import { STUDY_DIGEST, MODULAR_LOUNGE_DIGEST } from '../src/props/prop-contract.js';
import { installDrawObserver, observeIdleScene, sceneActivity } from './scene-observation.js';
import { installationWorldPoint } from './native-world-geometry.js';
import { openKeyboardSelection } from './office-navigation.js';

test('one installation world is lazy, saves through the browser and retains an empty layout after restart', async ({
  page,
}, info) => {
  await withSandbox(async (sandbox) => {
    const prefix = await installNativeOffice(sandbox);
    const office = async (args: string[]) => {
      const result = await runCli(sandbox, ['office', '--prefix', prefix, ...args, '--json'], {
        deadlineMs: 30_000,
      });
      expect(result.status, result.stdout).toBe(0);
      return JSON.parse(result.stdout);
    };
    const show = async (): Promise<WorldSnapshot> => office(['layout', 'show']);
    const observe = () => {
      const database = new Database(sandbox.database, { readonly: true });
      try {
        const row = database
          .prepare(
            'SELECT layout_revision AS revision, layout_json AS layout FROM office_local_worlds'
          )
          .get() as { revision: number; layout: string } | undefined;
        return {
          identities: database.prepare('SELECT count(*) FROM identities').pluck().get(),
          blocks: database.prepare('SELECT count(*) FROM office_local_blocks').pluck().get(),
          world: row ? { revision: row.revision, layout: JSON.parse(row.layout) } : null,
        };
      } finally {
        database.close();
      }
    };
    const initial = await show();
    expect(initial).toMatchObject({ worldId: null, revision: 0, changed: false });
    expect(initial.legacyBasis).toMatch(/^[a-f0-9]{64}$/);
    expect(initial.layout.map.version).toBe(8);
    const areas = mapGeometry(initial.layout.map).areas;
    expect(areas).toHaveLength(5);
    expect(areas.find((area) => area.id === initial.layout.map.primaryLobbyId)).toEqual({
      id: initial.layout.map.primaryLobbyId,
      name: 'Lobby',
      binding: { type: 'lobby' },
    });
    expect(
      areas.filter((area) => area.binding.type === 'personal').map((area) => area.binding)
    ).toEqual(Array.from({ length: 4 }, () => ({ type: 'personal', identityId: null })));
    const props = initial.layout.objects.map((object) => object.placement.prop);
    expect(props).toContain(`${MODULAR_LOUNGE_DIGEST}/lounge-sofa`);
    expect(props).toContain(`${STUDY_DIGEST}/oak-bookcase`);
    const unmaterialized = { identities: 0, blocks: 0, world: null };
    const desks = initial.layout.objects.filter((item) =>
      item.placement.prop.endsWith('/workstation-desk')
    );
    expect(desks).toHaveLength(4);
    expect(observe()).toEqual(unmaterialized);
    let started = await office(['start', '--port', String(await unusedLoopbackPort())]);
    try {
      await page.setViewportSize({ width: 1536, height: 1024 });
      await installDrawObserver(page);
      await page.goto(started.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      await expect(page.getByRole('button', { name: 'Office menu' })).toBeVisible();
      expect(observe()).toEqual(unmaterialized);
      await page.screenshot({ path: info.outputPath('native-world-preset.png') });
      await observeIdleScene(page, info, 'starter');
      // Inspect the real Lobby at a readable scale through normal camera controls.
      await page.mouse.move(900, 650);
      await page.mouse.down();
      await page.mouse.move(1032, 618);
      await page.mouse.up();
      for (let step = 0; step < 3; step++)
        await page.getByRole('button', { name: 'Zoom in', exact: true }).click();
      await sceneActivity(page);
      await page.screenshot({ path: info.outputPath('native-world-lobby.png') });
      await page.getByRole('button', { name: 'Fit office', exact: true }).click();
      await page.setViewportSize({ width: 390, height: 844 });
      await page.screenshot({ path: info.outputPath('native-world-preset-narrow.png') });
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        390
      );
      await page.setViewportSize({ width: 1536, height: 1024 });
      await openKeyboardSelection(page);
      await expect(
        page.getByRole('combobox', { name: 'Object', exact: true }).locator('option')
      ).toHaveCount(initial.layout.objects.length + 1);
      expect(observe()).toEqual(unmaterialized);
      await page.screenshot({ path: info.outputPath('native-world-edit-walls.png') });
      const areaPicker = page.getByRole('combobox', { name: 'Area', exact: true });
      await areaPicker.selectOption(areas.find((area) => area.name === 'Office 01')!.id);
      await expect(page.getByLabel('Area name', { exact: true })).toHaveValue('Office 01');
      await expect(page.getByText('Keyboard selection', { exact: true })).toBeHidden();
      await expect(page.locator('.world-build-content optgroup').first()).toHaveAttribute(
        'label',
        /^Office 01 · Current room \(/
      );
      await page.screenshot({ path: info.outputPath('native-world-edit-office-walls.png') });
      expect(observe()).toEqual(unmaterialized);
      await openKeyboardSelection(page);
      await areaPicker.selectOption(initial.layout.map.primaryLobbyId);
      await openKeyboardSelection(page);
      const sofa = initial.layout.objects.find(
        (object) => object.placement.prop.endsWith('/lounge-sofa') && object.placement.x === 8
      )!;
      expect(sofa.placement.y).toBe(8);
      await page.getByRole('combobox', { name: 'Object', exact: true }).selectOption(sofa.id);
      await expect(page.locator('.world-object-preview > svg')).toHaveCSS('width', '80px');
      await expect(page.locator('.world-object-preview > svg svg')).toHaveCSS('width', 'auto');
      await expect(page.locator('.world-object-preview > svg svg')).toHaveAttribute('width', '16');
      const inspector = await page
        .getByRole('complementary', { name: 'Layout tools' })
        .boundingBox();
      const saveBar = await page.locator('.world-save-bar').boundingBox();
      // A collapsed inspector must leave the unused space transparent to the scene.
      expect(inspector).not.toBeNull();
      expect(saveBar).not.toBeNull();
      expect(saveBar!.y + saveBar!.height).toBeLessThanOrEqual(inspector!.y);
      await page.screenshot({ path: info.outputPath('native-world-object-actions.png') });
      const canvasBounds = (await page.locator('.office-canvas canvas').boundingBox())!;
      // Within v8's first lattice cell, the 16x16 sofa at [8,8] paints at [8,5]..[24,21].
      const start = installationWorldPoint(canvasBounds, 16, 13);
      const outside = installationWorldPoint(canvasBounds, -8, 13);
      const end = installationWorldPoint(canvasBounds, 20, 13);
      await page.mouse.move(start.x, start.y);
      await page.mouse.down();
      await page.mouse.move(outside.x, outside.y, { steps: 6 });
      await expect(page.locator('.office-canvas canvas')).toHaveAttribute(
        'data-drop-validity',
        'invalid'
      );
      await page.mouse.up();
      await expect(page.getByRole('button', { name: 'Undo', exact: true })).toBeDisabled();
      expect(await show()).toEqual(initial);
      await page.mouse.move(start.x, start.y);
      await page.mouse.down();
      await page.mouse.move(end.x, end.y, { steps: 6 });
      await expect(page.locator('.office-canvas canvas')).toHaveAttribute(
        'data-drop-validity',
        'valid'
      );
      await page.mouse.up();
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      const moved = await show();
      expect(moved.revision).toBe(1);
      expect(moved.layout.objects.find((object) => object.id === sofa.id)!.placement).toMatchObject(
        { x: 12, y: 8 }
      );
      await page.getByRole('button', { name: 'Undo', exact: true }).click();
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      expect((await show()).layout).toEqual(initial.layout);
      await page.getByRole('button', { name: 'Redo', exact: true }).click();
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      // Explicit movement adopts the shallow base; merely reading the starter does not.
      const expectedLayout = {
        ...initial.layout,
        objects: initial.layout.objects.map((object) =>
          object.id === sofa.id
            ? {
                ...object,
                placement: { ...object.placement, x: 12 },
                surface: { type: 'floor', base: { x: 1, y: 10, width: 14, height: 6 } },
              }
            : object
        ),
      };
      await page.screenshot({ path: info.outputPath('native-world-solid-walls.png') });
      const saved = await show();
      expect(saved).toMatchObject({ revision: 3, legacyBasis: null, layout: expectedLayout });
      expect(observe()).toEqual({
        identities: 0,
        blocks: 0,
        world: { revision: 3, layout: expectedLayout },
      });

      const objects = page.getByRole('combobox', { name: 'Object', exact: true });
      for (const [index, object] of initial.layout.objects.entries()) {
        await objects.selectOption(object.id);
        await page.getByRole('button', { name: 'Delete', exact: true }).click();
        await openKeyboardSelection(page);
        await expect(objects.locator('option')).toHaveCount(initial.layout.objects.length - index);
      }
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      const cleared = await show();
      const empty = { ...initial.layout, objects: [] };
      expect(cleared).toMatchObject({ worldId: saved.worldId, layout: empty });
      expect(cleared.revision).toBeGreaterThan(saved.revision);
      await office(['stop']);
      started = await office(['start', '--port', String(await unusedLoopbackPort())]);
      await page.goto(started.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      await openKeyboardSelection(page);
      await expect(objects.locator('option')).toHaveCount(1);
      expect(await show()).toEqual(cleared);
      expect(observe()).toEqual({
        identities: 0,
        blocks: 0,
        world: { revision: cleared.revision, layout: empty },
      });
      await page.screenshot({ path: info.outputPath('native-world-empty-restarted.png') });
    } finally {
      await office(['stop']);
    }
  });
});
