import { writeFileSync } from 'node:fs';
import path from 'node:path';
import Database from '../../../../../../typescript/test/support/sqlite-oracle.js';
import { expect, test } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { officeWorldFixture } from '../../../../../../typescript/test/support/office-world.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import type { WorldSnapshot } from '../src/world-map/world-port.js';
import type { PropInstallInput } from '../src/props/prop-catalog-contract.js';
import { openKeyboardSelection } from './office-navigation.js';

test.use({ actionTimeout: 10_000, hasTouch: true });

test('draw, retry a lost native receipt, cancel placement, discover after restart and mount saved art without duplicating storage', async ({
  page,
}, info) => {
  await withSandbox(async (sandbox) => {
    const prefix = await installNativeOffice(sandbox);
    const office = async <T>(args: string[]): Promise<T> => {
      const result = await runCli(sandbox, ['office', '--prefix', prefix, ...args, '--json'], {
        deadlineMs: 30_000,
      });
      expect(result.status, result.stdout + result.stderr).toBe(0);
      return JSON.parse(result.stdout);
    };
    const show = () => office<WorldSnapshot>(['layout', 'show']);
    const initial = await show();
    const file = path.join(sandbox.root, 'pixel-world.json');
    writeFileSync(file, JSON.stringify(officeWorldFixture().layout));
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
    const baseline = await show();
    const artState = () => {
      const db = new Database(sandbox.database, { readonly: true });
      try {
        return {
          revision: db
            .prepare('SELECT revision FROM office_prop_catalog WHERE singleton = 1')
            .pluck()
            .get(),
          packs: db
            .prepare(
              'SELECT digest, bytes, installed_revision FROM office_prop_packs ORDER BY digest'
            )
            .all() as { digest: string; bytes: Buffer; installed_revision: number }[],
        };
      } finally {
        db.close();
      }
    };
    let started = await office<{ url: string }>([
      'start',
      '--port',
      String(await unusedLoopbackPort()),
    ]);
    const open = async () => {
      await page
        .getByRole('button', { name: 'Pixel workshop and art library', exact: true })
        .click();
    };
    const close = () =>
      page.getByRole('button', { name: 'Close pixel workshop', exact: true }).click();
    const saveArt = page.getByRole('button', { name: 'Save artwork to library', exact: true });
    let releaseCatalog: (() => void) | undefined;
    try {
      await page.setViewportSize({ width: 1440, height: 1100 });
      await page.goto(started.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      const catalogReady = new Promise<void>((resolve) => {
        releaseCatalog = resolve;
      });
      await page.route('**/api/v1/local/props/list', async (route) => {
        await catalogReady;
        await route.continue();
      });
      await open();
      await expect(page.getByText('Loading library…', { exact: true })).toBeVisible();
      await expect(saveArt).toBeDisabled();
      const canvas = page.getByRole('group', { name: 'Pixel drawing canvas', exact: true });
      await canvas.scrollIntoViewIfNeeded();
      const bounds = (await canvas.boundingBox())!;
      const point = (x: number, y: number) => ({
        x: bounds.x + ((x + 0.5) / 16) * bounds.width,
        y: bounds.y + ((y + 0.5) / 16) * bounds.height,
      });
      // Real pointer capture, including skipped samples, makes one undoable stroke.
      const start = point(3, 4),
        end = point(12, 4);
      await page.mouse.move(start.x, start.y);
      await page.mouse.down();
      // A real catalog response can arrive mid-stroke. It must not move the
      // canvas or reinterpret the remaining screen-space pointer coordinates.
      releaseCatalog!();
      await expect(page.getByText('Loading library…', { exact: true })).toHaveCount(0);
      expect(await canvas.boundingBox()).toEqual(bounds);
      await page.mouse.move(end.x, end.y, { steps: 2 });
      await page.mouse.up();
      await expect(saveArt).toBeEnabled();
      await page.getByRole('button', { name: 'Undo pixels', exact: true }).click();
      await expect(saveArt).toBeDisabled();
      await page.getByRole('button', { name: 'Redo pixels', exact: true }).click();
      await canvas.focus();
      await canvas.press('ArrowDown');
      await canvas.press('Space');
      await canvas.press('Delete');
      await page.getByRole('button', { name: 'Undo pixels', exact: true }).click();
      await page.getByRole('textbox', { name: 'Artwork name', exact: true }).fill('Orbit signal');
      expect(await show()).toEqual(baseline);
      expect(artState().packs).toEqual([]);

      const submissions: PropInstallInput[] = [];
      await page.route('**/api/v1/local/props/install', async (route) => {
        submissions.push(route.request().postDataJSON());
        if (submissions.length === 1) {
          const response = await route.fetch();
          expect(response.status()).toBe(200);
          await route.abort('connectionfailed');
        } else await route.continue();
      });
      await saveArt.click();
      await expect(page.getByText(/Save was not confirmed/)).toBeVisible();
      await expect(page.getByRole('textbox', { name: 'Artwork name', exact: true })).toBeDisabled();
      const stored = artState();
      expect(stored.revision).toBe(1);
      expect(stored.packs).toHaveLength(1);
      expect(stored.packs[0]!.bytes.toString()).toBe(submissions[0]!.document);
      const pack = JSON.parse(submissions[0]!.document);
      expect(pack.props[0].frames[0][4].slice(6, 26)).toBe('03'.repeat(10));
      expect(pack.props[0].frames[0][5].slice(24, 26)).toBe('03');
      await page.getByRole('button', { name: 'Retry exact save', exact: true }).click();
      await expect(page.getByText(/Artwork saved/)).toBeVisible();
      expect(submissions).toHaveLength(2);
      expect(submissions[1]).toEqual(submissions[0]);
      expect(artState()).toEqual(stored);
      await page
        .getByRole('heading', { name: 'Pixel workshop', exact: true })
        .scrollIntoViewIfNeeded();
      await page.screenshot({ path: info.outputPath('pixel-workshop-desktop.png') });
      await page
        .getByRole('button', { name: 'Add saved artwork to layout draft', exact: true })
        .click();
      await close();
      await openKeyboardSelection(page);
      await expect(
        page.getByRole('combobox', { name: 'Object', exact: true }).locator('option')
      ).toHaveCount(3);
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      expect((await show()).layout.objects).toHaveLength(baseline.layout.objects.length + 1);
      await page.getByRole('button', { name: 'Undo', exact: true }).click();
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      const restored = await show();
      expect(restored.layout).toEqual(baseline.layout);
      expect(restored.revision).toBeGreaterThan(baseline.revision);
      expect(artState()).toEqual(stored);

      await page.goto('about:blank');
      await office(['stop']);
      started = await office(['start', '--port', String(await unusedLoopbackPort())]);
      await page.setViewportSize({ width: 390, height: 844 });
      await page.goto(started.url);
      await open();
      // Narrow-screen touch uses the same pointer capture and eraser, without
      // writing a second pack or replacing the artwork saved before restart.
      const raster = canvas.locator(':scope > svg > path');
      await expect(raster).toHaveCount(0);
      const touch = async () => {
        await canvas.scrollIntoViewIfNeeded();
        const bounds = (await canvas.boundingBox())!;
        // Tap inside one cell, not the exact boundary between four cells:
        // device-coordinate rounding must not choose a different erase target.
        await page.touchscreen.tap(
          bounds.x + (8.5 / 16) * bounds.width,
          bounds.y + (8.5 / 16) * bounds.height
        );
      };
      await touch();
      await expect(raster).toHaveAttribute('d', 'M8 8h1v1h-1z');
      await expect(saveArt).toBeEnabled();
      await page.getByRole('button', { name: 'Eraser', exact: true }).click();
      await expect(page.getByRole('button', { name: 'Eraser', exact: true })).toHaveAttribute(
        'aria-pressed',
        'true'
      );
      await touch();
      await expect(raster).toHaveCount(0);
      await expect(saveArt).toBeDisabled();
      expect(artState()).toEqual(stored);
      await page.getByRole('button', { name: 'Saved library', exact: true }).click();
      await page.getByRole('button', { name: 'Orbit signal', exact: true }).click();
      const add = page.getByRole('button', {
        name: 'Add Orbit signal to layout draft',
        exact: true,
      });
      await expect(add).toBeVisible();
      expect(
        await page
          .getByRole('dialog', { name: 'Pixel workshop', exact: true })
          .evaluate((node) => node.scrollWidth <= node.clientWidth)
      ).toBe(true);
      await page.screenshot({ path: info.outputPath('pixel-library-narrow.png') });
      await add.click();
      await close();
      await page.getByText('Precise placement', { exact: true }).click();
      await page
        .getByRole('button', { name: 'Place on a suitable wall in this area', exact: true })
        .click();
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      const furnished = await show();
      expect(furnished.revision).toBeGreaterThan(baseline.revision);
      const artwork = furnished.layout.objects.find(
        (object) => object.placement.prop === `${stored.packs[0]!.digest}/artwork`
      )!;
      expect(artwork.surface.type).toBe('wall');
      expect(artState()).toEqual(stored);
      await page.setViewportSize({ width: 1440, height: 1000 });
      await page.screenshot({ path: info.outputPath('pixel-artwork-mounted.png') });
      await openKeyboardSelection(page);
      await page.getByRole('combobox', { name: 'Object', exact: true }).selectOption(artwork.id);
      await page.getByRole('button', { name: 'Delete', exact: true }).click();
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      expect((await show()).layout).toEqual(baseline.layout);
      expect(artState()).toEqual(stored);
    } finally {
      releaseCatalog?.();
      await office(['stop']);
    }
  });
});
