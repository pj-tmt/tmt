import Database from '../../../../../../typescript/test/support/sqlite-oracle.js';
import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import { installationWorldPoint } from './native-world-geometry.js';
import { openKeyboardSelection } from './office-navigation.js';
import type { WorldSnapshot } from '../src/world-map/world-port.js';

async function startDrag(page: Page, label: string, target: { x: number; y: number }) {
  await page.getByRole('searchbox', { name: 'Search objects' }).fill(label);
  const card = page.getByRole('button', { name: label, exact: true });
  await card.scrollIntoViewIfNeeded();
  const bounds = (await card.boundingBox())!;
  await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2);
  await page.mouse.down();
  await page.mouse.move(target.x, target.y, { steps: 8 });
}

test('catalog drag previews without a write, commits once, and retains exact Undo/Redo across service restart', async ({
  page,
}, info) => {
  await withSandbox(async (sandbox) => {
    const prefix = await installNativeOffice(sandbox);
    const office = async (args: string[]) => {
      const result = await runCli(sandbox, ['office', '--prefix', prefix, ...args, '--json'], {
        deadlineMs: 30_000,
      });
      expect(result.status, result.stdout + result.stderr).toBe(0);
      return JSON.parse(result.stdout);
    };
    const initial: WorldSnapshot = await office(['layout', 'show']);
    const observe = () => {
      const database = new Database(sandbox.database, { readonly: true });
      try {
        const row = database
          .prepare(
            'SELECT layout_revision AS revision, layout_json AS layout FROM office_local_worlds'
          )
          .get() as { revision: number; layout: string } | undefined;
        return row ? { revision: row.revision, layout: JSON.parse(row.layout) } : null;
      } finally {
        database.close();
      }
    };
    let started = await office(['start', '--port', String(await unusedLoopbackPort())]);
    const writes: unknown[] = [];
    page.on('request', (request) => {
      if (request.method() === 'PUT' && new URL(request.url()).pathname === '/api/v1/local/world')
        writes.push(request.postDataJSON());
    });
    try {
      await page.setViewportSize({ width: 1536, height: 1024 });
      await page.goto(started.url);
      const canvas = page.locator('.office-canvas canvas');
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      const view = (await canvas.boundingBox())!;
      await expect(page.getByText('Legacy pixel basics', { exact: true })).toHaveCount(0);
      const search = page.getByRole('searchbox', { name: 'Search objects' });
      await search.fill('Office built-ins');
      await expect(page.getByText('No matching objects.', { exact: true })).toBeVisible();
      // V8: source base bottom y=56 projects to 63; rigid 16x16 art centers at 55.
      const target = installationWorldPoint(view, 38, 55);
      await startDrag(page, 'Workshop writing desk', target);
      await expect(canvas).toHaveAttribute('data-catalog-drop-validity', 'valid');
      await expect(
        page.getByText('✓ Release to place · Esc to cancel', { exact: true })
      ).toBeVisible();
      expect(writes).toHaveLength(0);
      expect(observe()).toBeNull();
      await page.screenshot({ path: info.outputPath('catalog-drag-valid.png') });
      await page.mouse.up();
      await expect.poll(() => observe()?.revision).toBe(1);
      const saved = observe()!;
      expect(writes).toHaveLength(1);
      expect(saved.layout.map).toEqual(initial.layout.map);
      expect(saved.layout.objects.slice(0, -1)).toEqual(initial.layout.objects);
      expect(saved.layout.objects.at(-1)).toMatchObject({
        placement: { x: 30, y: 40 },
        surface: { type: 'floor', base: { x: 1, y: 8, width: 14, height: 8 } },
      });
      await expect(
        page.getByRole('heading', { name: 'Selected object: Workshop writing desk', exact: true })
      ).toBeVisible();
      await expect(canvas).not.toHaveAttribute('data-catalog-drop-validity');
      await page.getByRole('button', { name: 'Undo', exact: true }).click();
      await expect.poll(() => observe()?.revision).toBe(2);
      expect(observe()!.layout).toEqual(initial.layout);
      await page.getByRole('button', { name: 'Redo', exact: true }).click();
      await expect.poll(() => observe()?.revision).toBe(3);
      expect(observe()!.layout).toEqual(saved.layout);
      expect(writes).toHaveLength(3);
      await office(['stop']);
      started = await office(['start', '--port', String(await unusedLoopbackPort())]);
      await page.goto(started.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      expect((await office(['layout', 'show'])).layout).toEqual(saved.layout);
      await page.screenshot({ path: info.outputPath('catalog-drag-restored.png') });
    } finally {
      await office(['stop']);
    }
  });
});

test('invalid, cancelled and panel drops never save; another device drag still works after pan and zoom', async ({
  page,
}, info) => {
  await withSandbox(async (sandbox) => {
    const prefix = await installNativeOffice(sandbox);
    const office = async (args: string[]) => {
      const result = await runCli(sandbox, ['office', '--prefix', prefix, ...args, '--json'], {
        deadlineMs: 30_000,
      });
      expect(result.status, result.stdout + result.stderr).toBe(0);
      return JSON.parse(result.stdout);
    };
    const initial = await office(['layout', 'show']);
    const started = await office(['start', '--port', String(await unusedLoopbackPort())]);
    const writes: unknown[] = [];
    page.on('request', (request) => {
      if (request.method() === 'PUT' && new URL(request.url()).pathname === '/api/v1/local/world')
        writes.push(request.postDataJSON());
    });
    try {
      await page.setViewportSize({ width: 1536, height: 1024 });
      await page.goto(started.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      const canvas = page.locator('.office-canvas canvas');
      const view = (await canvas.boundingBox())!;
      const valid = installationWorldPoint(view, 32, 35.875);
      const invalid = installationWorldPoint(view, -12, 35.875);
      await startDrag(page, 'Workshop writing desk', invalid);
      await expect(canvas).toHaveAttribute('data-catalog-drop-validity', 'invalid');
      await expect(
        page.getByText('× Keep the entire base on the platform', { exact: true })
      ).toBeVisible();
      await page.screenshot({ path: info.outputPath('catalog-drag-invalid.png') });
      await page.mouse.up();
      for (const cancel of ['escape', 'pointercancel', 'panel']) {
        await startDrag(page, 'Workshop writing desk', valid);
        await expect(canvas).toHaveAttribute('data-catalog-drop-validity', 'valid');
        if (cancel === 'escape') await page.keyboard.press('Escape');
        else if (cancel === 'pointercancel')
          await page
            .getByRole('button', { name: 'Workshop writing desk', exact: true })
            .dispatchEvent('pointercancel', { pointerId: 1 });
        else {
          const card = (await page
            .getByRole('button', { name: 'Workshop writing desk', exact: true })
            .boundingBox())!;
          await page.mouse.move(card.x + 15, card.y + 15);
        }
        await page.mouse.up();
        await expect(canvas).not.toHaveAttribute('data-catalog-drop-validity');
      }
      expect(writes).toHaveLength(0);
      expect(await office(['layout', 'show'])).toEqual(initial);
      await expect(page.getByRole('button', { name: 'Undo', exact: true })).toBeDisabled();
      // Camera changes are real input; expected placement is derived from the
      // independent initial projection and the chosen affine camera movement.
      await page.mouse.move(invalid.x, invalid.y);
      await page.mouse.down();
      await page.mouse.move(invalid.x + 24, invalid.y + 16);
      await page.mouse.up();
      await page.getByRole('button', { name: 'Zoom in', exact: true }).click();
      const zoomed = {
        x: view.x + view.width / 2 + (valid.x + 24 - view.x - view.width / 2) * 1.2,
        y: view.y + view.height / 2 + (valid.y + 16 - view.y - view.height / 2) * 1.2,
      };
      await startDrag(page, 'Workshop whiteboard', zoomed);
      await expect(canvas).toHaveAttribute('data-catalog-drop-validity', 'valid');
      const applied = page.waitForResponse(
        (response) =>
          response.request().method() === 'PUT' &&
          new URL(response.url()).pathname === '/api/v1/local/world' &&
          response.status() === 200
      );
      await page.mouse.up();
      await applied;
      await expect(page.getByRole('status')).toHaveText('All changes applied');
      expect(writes).toHaveLength(1);
      expect((await office(['layout', 'show'])).layout.objects).toHaveLength(
        initial.layout.objects.length + 1
      );
      await page.setViewportSize({ width: 390, height: 844 });
      await page.getByRole('button', { name: 'Clear selection', exact: true }).click();
      await expect(page.getByRole('heading', { name: 'Furniture & devices' })).toBeVisible();
      await expect(page.getByText('Legacy pixel basics', { exact: true })).toHaveCount(0);
      await expect(page.getByRole('button', { name: 'Desk', exact: true })).toHaveCount(0);
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        390
      );
      await page.screenshot({ path: info.outputPath('catalog-narrow.png') });
    } finally {
      await office(['stop']);
    }
  });
});

test('corner rotation is transient and durable once; Delete respects text fields and Undo restores the object', async ({
  page,
}, info) => {
  await withSandbox(async (sandbox) => {
    const prefix = await installNativeOffice(sandbox);
    const office = async (args: string[]) => {
      const result = await runCli(sandbox, ['office', '--prefix', prefix, ...args, '--json'], {
        deadlineMs: 30_000,
      });
      expect(result.status, result.stdout + result.stderr).toBe(0);
      return JSON.parse(result.stdout);
    };
    const started = await office(['start', '--port', String(await unusedLoopbackPort())]);
    const writes: unknown[] = [];
    page.on('request', (r) => {
      if (r.method() === 'PUT' && new URL(r.url()).pathname === '/api/v1/local/world')
        writes.push(r.postDataJSON());
    });
    try {
      await page.setViewportSize({ width: 1536, height: 1024 });
      await page.goto(started.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      const canvas = page.locator('.office-canvas canvas');
      const view = (await canvas.boundingBox())!;
      // Keep this rotation fixture within one lattice cell: source (28,24),
      // physical bottom y=32 projects to 28; the rigid 8x8 art centers at (32,24).
      await startDrag(page, 'Workshop rolling chair', installationWorldPoint(view, 32, 24));
      const waitWrite = () =>
        page.waitForResponse(
          (r) =>
            r.request().method() === 'PUT' &&
            new URL(r.url()).pathname === '/api/v1/local/world' &&
            r.status() === 200
        );
      let saved = waitWrite();
      await page.mouse.up();
      await saved;
      const before = await office(['layout', 'show']);
      const chair = before.layout.objects.at(-1);
      expect(chair.surface).toEqual({
        type: 'floor',
        base: { x: 2, y: 4, width: 4, height: 4 },
      });
      const { x, y } = chair.placement;
      expect({ x, y }).toEqual({ x: 28, y: 24 });
      const corner = installationWorldPoint(view, x + 8, (y * 7) / 8 - 1);
      const next = installationWorldPoint(view, x + 8, (y * 7) / 8 + 7);
      await page.mouse.move(corner.x, corner.y);
      await page.mouse.down();
      await page.mouse.move(next.x, next.y, { steps: 8 });
      await expect(canvas).toHaveAttribute('data-drop-validity', 'valid');
      expect(writes).toHaveLength(1);
      expect((await office(['layout', 'show'])).layout).toEqual(before.layout);
      await page.screenshot({ path: info.outputPath('chair-rotation-preview.png') });
      saved = waitWrite();
      await page.mouse.up();
      await saved;
      const rotated = await office(['layout', 'show']);
      expect(rotated.layout.objects.at(-1)).toEqual({
        ...chair,
        placement: {
          ...chair.placement,
          x: chair.placement.x + 2,
          y: chair.placement.y + 2,
          rotation: 1,
        },
      });
      expect(writes).toHaveLength(2);
      await expect(page.getByRole('button', { name: 'Edit room settings' })).toHaveCount(0);
      await page.getByText('Precise placement', { exact: true }).click();
      await page.getByLabel('X', { exact: true }).focus();
      await page.keyboard.press('Delete');
      expect(writes).toHaveLength(2);
      await page.getByRole('heading', { name: 'Selected object: Workshop rolling chair' }).focus();
      saved = waitWrite();
      await page.keyboard.press('Delete');
      await saved;
      expect((await office(['layout', 'show'])).layout.objects).toHaveLength(
        before.layout.objects.length - 1
      );
      saved = waitWrite();
      await page.getByRole('button', { name: 'Undo', exact: true }).click();
      await saved;
      expect((await office(['layout', 'show'])).layout).toEqual(rotated.layout);
      const floor = installationWorldPoint(view, 55, 8);
      await page.mouse.click(floor.x, floor.y);
      await expect(page.getByLabel('Area name', { exact: true })).toHaveValue('Lobby');
      await expect(page.getByText('Keyboard selection', { exact: true })).toBeHidden();
      await expect(
        page
          .getByRole('region', { name: 'Selected area', exact: true })
          .getByRole('heading', { name: 'Lobby', exact: true })
      ).toHaveCount(0);
      await page.screenshot({ path: info.outputPath('area-inspector.png') });
      for (const width of [1536, 390]) {
        await page.setViewportSize({ width, height: 1024 });
        const material = page.getByRole('group', { name: 'Floor & frame' });
        const removal = page.getByRole('button', { name: 'Review module removal' });
        const materialBounds = (await material.boundingBox())!;
        const removalBounds = (await removal.boundingBox())!;
        expect(removalBounds.y - materialBounds.y - materialBounds.height).toBeGreaterThanOrEqual(
          16
        );
        await removal.scrollIntoViewIfNeeded();
        await page.screenshot({ path: info.outputPath(`area-removal-spacing-${width}.png`) });
      }
      await openKeyboardSelection(page);
      await page.getByRole('combobox', { name: 'Area', exact: true }).selectOption({
        label: 'Office 03',
      });
      for (const width of [1536, 390]) {
        await page.setViewportSize({ width, height: 1024 });
        const material = (await page.getByRole('group', { name: 'Floor & frame' }).boundingBox())!;
        const resident = (await page
          .getByRole('combobox', { name: 'Resident', exact: true })
          .locator('..')
          .boundingBox())!;
        const removal = (await page
          .getByRole('button', { name: 'Review module removal' })
          .boundingBox())!;
        expect(resident.y - material.y - material.height).toBeCloseTo(16, 1);
        expect(removal.y - resident.y - resident.height).toBeCloseTo(16, 1);
        await page.screenshot({ path: info.outputPath(`office-field-spacing-${width}.png`) });
      }
    } finally {
      await office(['stop']);
    }
  });
});
