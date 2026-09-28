import { randomUUID } from 'node:crypto';
import { writeFileSync } from 'node:fs';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import { installationWorldPoint } from './native-world-geometry.js';
import { openKeyboardSelection } from './office-navigation.js';
import { captureWorldScene, installDrawObserver } from './scene-observation.js';
import {
  BUILTIN_CATALOG,
  WORKSHOP_DIGEST,
  MODULAR_LOUNGE_DIGEST,
  DIRECTIONAL_LOUNGE_DIGEST,
  DIRECTIONAL_WORKSTATION_DIGEST,
  DIRECTIONAL_RECEPTION_DIGEST,
  DIRECTIONAL_FACILITIES_DIGEST,
} from '../src/props/prop-contract.js';
import type { WorldSnapshot } from '../src/world-map/world-port.js';
import type { WorldObject } from '../src/world-map/world-contract.js';

test('retained sofa gains genuine corner rotation with atomic art Undo, Redo and restart', async ({
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
    const sofa: WorldObject = {
      id: randomUUID(),
      kind: 'decoration',
      surface: { type: 'floor' },
      extension: null,
      placement: {
        prop: `${MODULAR_LOUNGE_DIGEST}/lounge-sofa`,
        footprint: { width: 16, height: 16 },
        x: 24,
        y: 16,
        rotation: 0,
      },
    };
    const edgeDesk: WorldObject = {
      ...sofa,
      id: randomUUID(),
      placement: {
        prop: `${WORKSHOP_DIGEST}/oak-desk`,
        footprint: { width: 12, height: 8 },
        x: 48,
        y: 0,
        rotation: 0,
      },
    };
    const retained = [...initial.layout.objects, edgeDesk];
    const layout = { ...initial.layout, objects: [...retained, sofa] };
    const file = path.join(sandbox.root, 'retained-sofa.json');
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
    const acknowledgement = () =>
      page.waitForResponse(
        (response) =>
          response.request().method() === 'PUT' &&
          new URL(response.url()).pathname === '/api/v1/local/world' &&
          response.status() === 200
      );
    try {
      await page.setViewportSize({ width: 1536, height: 1024 });
      await installDrawObserver(page);
      await page.goto(started.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      await openKeyboardSelection(page);
      await page.getByRole('combobox', { name: 'Object', exact: true }).selectOption(sofa.id);
      const canvas = page.locator('.office-canvas canvas');
      const view = (await canvas.boundingBox())!;
      // This 12x8 desk fits initially but a centered turn crosses the north edge.
      await openKeyboardSelection(page);
      await page.getByRole('combobox', { name: 'Object', exact: true }).selectOption(edgeDesk.id);
      await page.getByText('Precise placement', { exact: true }).click();
      await expect(page.getByRole('button', { name: 'Rotate 90° clockwise' })).toBeDisabled();
      const invalidCorner = installationWorldPoint(view, 60, -1);
      const invalidEnd = installationWorldPoint(view, 60, 7);
      const originalHandle = installationWorldPoint(view, 48, 7);
      // Centered 12x8 -> 8x12 turn: origin (50,-2), projected bottom y=8.75.
      const candidateHandle = installationWorldPoint(view, 54, 8.75);
      async function handlePixels() {
        const image = await page.screenshot({ scale: 'css' });
        return page.evaluate(
          async ({ bytes, points }) => {
            const bitmap = await createImageBitmap(
              new Blob([new Uint8Array(bytes)], { type: 'image/png' })
            );
            const surface = document.createElement('canvas');
            surface.width = bitmap.width;
            surface.height = bitmap.height;
            const context = surface.getContext('2d')!;
            context.drawImage(bitmap, 0, 0);
            bitmap.close();
            // Sample inside the circular handle but outside the rectangle stroke.
            return points.map(({ x, y }) =>
              Array.from(context.getImageData(Math.round(x + 2), Math.round(y + 2), 1, 1).data)
            );
          },
          { bytes: [...image], points: [originalHandle, candidateHandle] }
        );
      }
      const handleFill = [16, 43, 53, 255];
      expect((await handlePixels())[0]).toEqual(handleFill);
      await page.mouse.move(invalidCorner.x, invalidCorner.y);
      await page.mouse.down();
      await page.mouse.move(invalidEnd.x, invalidEnd.y, { steps: 8 });
      await expect(canvas).toHaveAttribute('data-drop-validity', 'invalid');
      const heldHandles = await handlePixels();
      expect(heldHandles[0]).not.toEqual(handleFill);
      expect(heldHandles[1]).toEqual(handleFill);
      await page.screenshot({ path: info.outputPath('rotation-invalid.png') });
      await page.mouse.up();
      expect((await handlePixels())[0]).toEqual(handleFill);
      expect(writes).toHaveLength(0);
      expect((await office(['layout', 'show'])).layout).toEqual(layout);
      await expect(page.getByRole('button', { name: 'Undo', exact: true })).toBeDisabled();
      await openKeyboardSelection(page);
      await page.getByRole('combobox', { name: 'Object', exact: true }).selectOption(sofa.id);
      // Keep the 16x16 pixel-comparison fixture inside one v8 lattice cell:
      // source (24,16), bottom anchor (24,32), painted top (24,12).
      const origin = installationWorldPoint(view, 24, 12);
      const corner = installationWorldPoint(view, 40, 12);
      const end = installationWorldPoint(view, 40, 28);
      // The rotated shallow base keeps its center but moves the upright envelope
      // down to y=15.5. Compare the common interior of both envelopes, excluding
      // both old selection handles and the candidate/committed outline.
      const rotatedOrigin = installationWorldPoint(view, 24, 15.5);
      const clip = {
        x: origin.x + 8,
        y: rotatedOrigin.y + 8,
        width: end.x - origin.x - 16,
        height: end.y - rotatedOrigin.y - 16,
      };
      const originalImage = await page.screenshot({ clip });
      async function dragCorner() {
        await page.mouse.move(corner.x, corner.y);
        await page.mouse.down();
        await page.mouse.move(end.x, end.y, { steps: 8 });
        await expect(canvas).toHaveAttribute('data-drop-validity', 'valid');
      }
      await dragCorner();
      await page.keyboard.press('Escape');
      await page.mouse.up();
      expect(await page.screenshot({ clip })).toEqual(originalImage);
      expect(writes).toHaveLength(0);
      expect((await office(['layout', 'show'])).layout).toEqual(layout);
      await openKeyboardSelection(page);
      await page.getByRole('combobox', { name: 'Object', exact: true }).selectOption(sofa.id);
      await dragCorner();
      expect(writes).toHaveLength(0);
      expect((await office(['layout', 'show'])).layout).toEqual(layout);
      await page.screenshot({ path: info.outputPath('retained-sofa-rotation-preview.png') });
      const previewImage = await page.screenshot({ clip, path: info.outputPath('held-art.png') });
      expect(previewImage.equals(originalImage)).toBe(false);
      let saved = acknowledgement();
      await page.mouse.up();
      await saved;
      // The held preview is the eventual object, not a translucent second copy
      // superimposed over the original orientation.
      expect(await page.screenshot({ clip, path: info.outputPath('committed-art.png') })).toEqual(
        previewImage
      );
      await page.screenshot({ path: info.outputPath('retained-sofa-rotation-committed.png') });
      const rotated = await office(['layout', 'show']);
      const expected = {
        ...layout,
        objects: [
          ...retained,
          {
            ...sofa,
            surface: { type: 'floor', base: { x: 1, y: 10, width: 14, height: 6 } },
            placement: {
              ...sofa.placement,
              prop: `${DIRECTIONAL_LOUNGE_DIGEST}/lounge-sofa`,
              rotation: 1,
              x: 29,
              y: 21,
            },
          },
        ],
      };
      expect(rotated.layout).toEqual(expected);
      expect(rotated.revision).toBe(2);
      expect(writes).toHaveLength(1);
      saved = acknowledgement();
      await page.getByRole('button', { name: 'Undo', exact: true }).click();
      await saved;
      expect((await office(['layout', 'show'])).layout).toEqual(layout);
      saved = acknowledgement();
      await page.getByRole('button', { name: 'Redo', exact: true }).click();
      await saved;
      expect((await office(['layout', 'show'])).layout).toEqual(expected);
      await page.getByText('Precise placement', { exact: true }).click();
      for (const [rotation, x, y] of [
        [2, 24, 26],
        [3, 19, 21],
        [0, 24, 16],
      ] as const) {
        saved = acknowledgement();
        await page.getByRole('button', { name: 'Rotate 90° clockwise' }).click();
        await saved;
        expected.objects.at(-1)!.placement.rotation = rotation;
        expected.objects.at(-1)!.placement.x = x;
        expected.objects.at(-1)!.placement.y = y;
        expect((await office(['layout', 'show'])).layout).toEqual(expected);
        await page.screenshot({ path: info.outputPath(`sofa-direction-${rotation}.png`) });
      }
      expect(writes).toHaveLength(6);
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

test('directional furniture gallery renders all admitted views at desktop and narrow sizes', async ({
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
    let snapshot: WorldSnapshot = await office(['layout', 'show']);
    const digests = [
      DIRECTIONAL_WORKSTATION_DIGEST,
      DIRECTIONAL_LOUNGE_DIGEST,
      DIRECTIONAL_RECEPTION_DIGEST,
      DIRECTIONAL_FACILITIES_DIGEST,
    ];
    const entries = BUILTIN_CATALOG.filter((p) => digests.includes(p.digest)).flatMap((pack) =>
      pack.pack.props.map((prop) => ({ pack, prop }))
    );
    expect(entries).toHaveLength(12);
    const furniture: WorldObject[] = entries.map(({ pack, prop }, index) => ({
      id: randomUUID(),
      kind: 'decoration',
      surface: { type: 'floor' },
      extension: null,
      placement: {
        prop: `${pack.digest}/${prop.key}`,
        footprint: prop.footprint,
        x: 4 + (index % 5) * 20,
        y: 4 + Math.floor(index / 5) * 22,
        rotation: 0,
      },
    }));
    const hosts = snapshot.layout.objects
      .filter((object) => object.extension)
      .map((object, index) => ({
        ...object,
        placement: { ...object.placement, x: 4 + index * 24, y: 70 },
      }));
    const file = path.join(sandbox.root, 'directional-gallery.json');
    const started = await office(['start', '--port', String(await unusedLoopbackPort())]);
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    const images = new Set<string>();
    try {
      await page.setViewportSize({ width: 1536, height: 1024 });
      await installDrawObserver(page);
      for (const rotation of [0, 1, 2, 3]) {
        const layout = {
          ...snapshot.layout,
          objects: [
            ...hosts,
            ...furniture.map((object) => ({
              ...object,
              placement: { ...object.placement, rotation },
            })),
          ],
        };
        writeFileSync(file, JSON.stringify(layout));
        await office([
          'layout',
          'apply',
          '--file',
          file,
          '--if-revision',
          String(snapshot.revision),
          ...(snapshot.legacyBasis ? ['--legacy-basis', snapshot.legacyBasis] : []),
        ]);
        snapshot = await office(['layout', 'show']);
        expect(snapshot.layout).toEqual(layout);
        // Read the externally applied revision through the existing local session;
        // the bootstrap token is intentionally consumed on first navigation.
        if (rotation === 0) await page.goto(started.url);
        else {
          const refreshed = page.waitForResponse(
            (response) => new URL(response.url()).pathname === '/api/v1/local/world'
          );
          await page.getByRole('button', { name: 'Refresh office' }).click();
          const response = await refreshed;
          expect(response.status()).toBe(200);
          expect((await response.json()).layout).toEqual(layout);
        }
        await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
        const image = await captureWorldScene(page, info, `furniture-gallery-${rotation}.png`);
        images.add(image.toString('base64'));
      }
      expect(images.size).toBe(4);
      await page.setViewportSize({ width: 390, height: 844 });
      await page.screenshot({ path: info.outputPath('furniture-gallery-narrow.png') });
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        390
      );
      expect(errors).toEqual([]);
    } finally {
      await office(['stop']);
    }
  });
});
