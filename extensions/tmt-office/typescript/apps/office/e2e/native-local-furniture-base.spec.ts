import { randomUUID } from 'node:crypto';
import { writeFileSync } from 'node:fs';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import { installationWorldPoint } from './native-world-geometry.js';
import {
  DIRECTIONAL_WORKSTATION_DIGEST,
  DIRECTIONAL_LOUNGE_DIGEST,
} from '../src/props/prop-contract.js';
import type { WorldObject } from '../src/world-map/world-contract.js';
import type { WorldSnapshot } from '../src/world-map/world-port.js';

test('bookcase upper artwork stays clickable above a shallow supported base through drag, history and restart', async ({
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
    const bookcase: WorldObject = {
      id: randomUUID(),
      kind: 'decoration',
      surface: { type: 'floor' },
      extension: null,
      placement: {
        prop: `${DIRECTIONAL_WORKSTATION_DIGEST}/workstation-bookcase`,
        footprint: { width: 11, height: 11 },
        x: 60,
        y: 20,
        rotation: 0,
      },
    };
    const behind: WorldObject = {
      ...bookcase,
      id: randomUUID(),
      placement: {
        prop: `${DIRECTIONAL_LOUNGE_DIGEST}/lounge-table`,
        footprint: { width: 10, height: 10 },
        x: 60,
        y: 20,
        rotation: 0,
      },
    };
    const layout = { ...initial.layout, objects: [behind, bookcase] };
    const file = path.join(sandbox.root, 'bookcase-base.json');
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
    let started = await office(['start', '--port', String(await unusedLoopbackPort())]);
    const writes: unknown[] = [];
    page.on('request', (request) => {
      if (request.method() === 'PUT' && new URL(request.url()).pathname === '/api/v1/local/world')
        writes.push(request.postDataJSON());
    });
    const acknowledged = () =>
      page.waitForResponse(
        (response) =>
          response.request().method() === 'PUT' &&
          new URL(response.url()).pathname === '/api/v1/local/world' &&
          response.status() === 200
      );
    try {
      await page.setViewportSize({ width: 1536, height: 1024 });
      await page.goto(started.url);
      const canvas = page.locator('.office-canvas canvas');
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      const view = (await canvas.boundingBox())!;
      const point = (x: number, y: number) => installationWorldPoint(view, x, y);
      // V8's second column adds 16 display units to x. The base's bottom
      // remains inside the first Y cell, with an unchanged rigid art height.
      // Both objects cover this point; the last painted bookcase must win.
      const upper = point(81, 18);
      await page.mouse.click(upper.x, upper.y);
      await expect(
        page.getByRole('heading', { name: 'Selected object: Workshop bookcase' })
      ).toBeVisible();
      expect(writes).toHaveLength(0);
      expect((await office(['layout', 'show'])).layout).toEqual(layout);
      const edge = point(81, -6.5);
      await page.mouse.move(upper.x, upper.y);
      await page.mouse.down();
      await page.mouse.move(edge.x, edge.y, { steps: 8 });
      await expect(canvas).toHaveAttribute('data-drop-validity', 'valid');
      expect(writes).toHaveLength(0);
      let saved = acknowledged();
      await page.mouse.up();
      await saved;
      const expected: typeof layout = {
        ...layout,
        objects: [
          behind,
          {
            ...bookcase,
            surface: { type: 'floor', base: { x: 2, y: 8, width: 7, height: 3 } },
            placement: { ...bookcase.placement, y: -8 },
          },
        ],
      };
      expect((await office(['layout', 'show'])).layout).toEqual(expected);
      // The opaque lower shelf crosses the north rim. A zero-turn rotation
      // preview paints this same artwork above architecture, providing an
      // independent pixel comparison against its committed layer.
      const rimStart = point(79, 0);
      const rimEnd = point(84, 0.75);
      const rimClip = {
        x: Math.ceil(rimStart.x),
        y: Math.ceil(rimStart.y),
        width: Math.floor(rimEnd.x) - Math.ceil(rimStart.x),
        height: Math.floor(rimEnd.y) - Math.ceil(rimStart.y),
      };
      const committedShelf = await page.screenshot({ clip: rimClip, scale: 'css' });
      // Artwork bounds: (76,-8.375)..(87,2.625). Move radially outward from
      // the bottom-right handle without changing the angle or saved rotation.
      const corner = point(87, 2.625);
      const radial = point(90, 5.625);
      await page.mouse.move(corner.x, corner.y);
      await page.mouse.down();
      await page.mouse.move(radial.x, radial.y, { steps: 4 });
      await expect(canvas).toHaveAttribute('data-drop-validity', 'valid');
      expect(await page.screenshot({ clip: rimClip, scale: 'css' })).toEqual(committedShelf);
      await page.mouse.up();
      expect(writes).toHaveLength(1);
      expect((await office(['layout', 'show'])).layout).toEqual(expected);
      await page.screenshot({ path: info.outputPath('bookcase-supported-overhang.png') });
      // Upper art is now outside the platform and far above its y=0..3 base.
      await page.keyboard.press('Escape');
      await page.mouse.click(edge.x, edge.y);
      await expect(
        page.getByRole('heading', { name: 'Selected object: Workshop bookcase' })
      ).toBeVisible();
      const invalid = point(81, -7.375);
      await page.mouse.move(edge.x, edge.y);
      await page.mouse.down();
      await page.mouse.move(invalid.x, invalid.y, { steps: 4 });
      // One tile is less than the gesture threshold at Fit; move two tiles instead.
      const unsupported = point(81, -8.25);
      await page.mouse.move(unsupported.x, unsupported.y, { steps: 4 });
      await expect(canvas).toHaveAttribute('data-drop-validity', 'invalid');
      await page.screenshot({ path: info.outputPath('bookcase-unsupported-base.png') });
      await page.mouse.up();
      expect(writes).toHaveLength(1);
      expect((await office(['layout', 'show'])).layout).toEqual(expected);
      saved = acknowledged();
      await page.getByRole('button', { name: 'Undo', exact: true }).click();
      await saved;
      expect((await office(['layout', 'show'])).layout).toEqual(layout);
      saved = acknowledged();
      await page.getByRole('button', { name: 'Redo', exact: true }).click();
      await saved;
      expect((await office(['layout', 'show'])).layout).toEqual(expected);
      await page.mouse.move(edge.x, edge.y);
      await page.mouse.down();
      await page.mouse.move(upper.x, upper.y, { steps: 8 });
      saved = acknowledged();
      await page.mouse.up();
      await saved;
      expected.objects[1] = { ...expected.objects[1]!, placement: { ...bookcase.placement } };
      expect((await office(['layout', 'show'])).layout).toEqual(expected);
      await page.getByText('Precise placement', { exact: true }).click();
      // Full artwork remains 11x11; physical 7x3 support rotates around its center.
      for (const [rotation, x, y] of [
        [1, 64, 24],
        [2, 60, 28],
        [3, 56, 24],
        [0, 60, 20],
      ]) {
        saved = acknowledged();
        await page.getByRole('button', { name: 'Rotate 90° clockwise' }).click();
        await saved;
        expected.objects[1] = {
          ...expected.objects[1]!,
          placement: { ...bookcase.placement, rotation: rotation!, x: x!, y: y! },
        };
        expect((await office(['layout', 'show'])).layout).toEqual(expected);
        await page.screenshot({ path: info.outputPath(`bookcase-base-direction-${rotation}.png`) });
      }
      await page.setViewportSize({ width: 390, height: 844 });
      await expect(page.getByRole('button', { name: 'Rotate 90° clockwise' })).toBeEnabled();
      saved = acknowledged();
      await page.getByRole('button', { name: 'Rotate 90° clockwise' }).click();
      await saved;
      expect((await office(['layout', 'show'])).layout.objects[1]).toEqual({
        ...expected.objects[1]!,
        placement: { ...bookcase.placement, rotation: 1, x: 64, y: 24 },
      });
      saved = acknowledged();
      await page.getByRole('button', { name: 'Undo', exact: true }).click();
      await saved;
      expect((await office(['layout', 'show'])).layout).toEqual(expected);
      await page.screenshot({ path: info.outputPath('bookcase-base-narrow.png') });
      await office(['stop']);
      started = await office(['start', '--port', String(await unusedLoopbackPort())]);
      await page.goto(started.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      expect((await office(['layout', 'show'])).layout).toEqual(expected);
    } finally {
      await office(['stop']);
    }
  });
});
