import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import Database from '../../../../../../typescript/test/support/sqlite-oracle.js';
import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { readRasterCells } from '../../../../../../typescript/test/support/indexed-raster.js';
import { builtinFurniture } from '../src/blocks/block-contract.js';
import type { Furniture } from '../src/blocks/block-contract.js';
import { officeWorldFixture } from '../../../../../../typescript/test/support/office-world.js';
import type { WorldDocument } from '../src/world-map/world-contract.js';
import {
  BUILTIN_DIGEST,
  WORKSHOP_DIGEST,
  COMMONS_DIGEST,
  WHITEBOARD_DIGEST,
  BROADCASTER_DIGEST,
  STUDY_DIGEST,
  WALL_DIGEST,
  MODULAR_WORKSTATION_DIGEST,
  MODULAR_MOUNTED_DIGEST,
  MODULAR_LOUNGE_DIGEST,
  MODULAR_FACILITIES_DIGEST,
  MODULAR_RECEPTION_DIGEST,
  DIRECTIONAL_WORKSTATION_DIGEST,
  DIRECTIONAL_LOUNGE_DIGEST,
  DIRECTIONAL_RECEPTION_DIGEST,
  DIRECTIONAL_FACILITIES_DIGEST,
} from '../src/props/prop-contract.js';
import { PROP_DIRECTIONS } from '../src/props/prop-contract.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import { savedWorld } from './native-world-state.js';
import { openKeyboardSelection } from './office-navigation.js';

/** Stable placement IDs in a fixed world, without generated default furniture. */
function worldLayout(placements: Furniture[]): WorldDocument {
  return {
    ...officeWorldFixture().layout,
    objects: placements.map((placement, index) => ({
      id: `30000000-0000-4000-8000-${String(index + 1).padStart(12, '0')}`,
      kind: 'decoration',
      placement,
      surface: { type: 'floor' },
      extension: null,
    })),
  };
}

async function editPlacement(page: Page, index = 0) {
  await openKeyboardSelection(page);
  await page
    .getByRole('combobox', { name: 'Object', exact: true })
    .selectOption(`30000000-0000-4000-8000-${String(index + 1).padStart(12, '0')}`);
}

async function saveLayout(page: Page) {
  await expect(page.getByRole('region', { name: 'Layout changes' }).getByRole('status')).toHaveText(
    'All changes applied'
  );
}

async function refreshWorld(page: Page) {
  // The old canvas may still be ready during React's refresh transition. Wait
  // for this refresh's world read before observing the newly mounted scene.
  // Built-in-only worlds do not need a custom catalog HTTP request.
  const resolved = page.waitForResponse(
    (response) => new URL(response.url()).pathname === '/api/v1/local/world',
    { timeout: 10_000 }
  );
  await page.getByRole('button', { name: 'Refresh office' }).click();
  expect((await resolved).status()).toBe(200);
  await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
}

/** Observe composited pixels without depending on the renderer's private scene graph. */
async function signalColorMask(page: Page) {
  const screenshot = await page.locator('.office-canvas canvas').screenshot();
  return page.evaluate(async (base64) => {
    const bytes = Uint8Array.from(atob(base64), (character) => character.charCodeAt(0));
    const bitmap = await createImageBitmap(new Blob([bytes], { type: 'image/png' }));
    try {
      const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
      const context = canvas.getContext('2d')!;
      context.drawImage(bitmap, 0, 0);
      const pixels = context.getImageData(0, 0, bitmap.width, bitmap.height).data;
      const mask = new Uint8Array(bitmap.width * bitmap.height);
      let count = 0;
      for (let index = 0; index < mask.length; index++) {
        const offset = index * 4;
        if (
          pixels[offset] === 255 &&
          pixels[offset + 1] === 85 &&
          pixels[offset + 2] === 51 &&
          pixels[offset + 3] === 255
        ) {
          mask[index] = 1;
          count++;
        }
      }
      const hash = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', mask)))
        .map((byte) => byte.toString(16).padStart(2, '0'))
        .join('');
      // Independent v1 fixture oracle: 36x36 floor at 5/8 depth, a
      // sixteen-tile rear wall, 8x12 total padding and a reserved HUD band.
      const margin = Math.min(bitmap.height * 0.2, Math.max(96, bitmap.height * 0.08));
      const scale = Math.min((bitmap.width * 0.84) / 44, (bitmap.height - 2 * margin) / 50.5);
      const left = (bitmap.width - 44 * scale) / 2;
      const top = (bitmap.height - 50.5 * scale) / 2;
      const sample = (x: number, y: number) =>
        mask[
          Math.floor(top + (20 + y) * scale) * bitmap.width + Math.floor(left + (4 + x) * scale)
        ];
      const signalSamples = [
        ...Array.from({ length: 14 }, (_, index) => sample(5 + index + 0.5, 1.375)),
        ...[
          [1, 0],
          [0, 1],
          [1, 1],
          [2, 1],
          [1, 2],
        ].map(([x, y]) => sample(4 + ((x! + 0.5) * 2) / 3, 3 + ((y! + 0.5) * 2) / 3)),
      ];
      const transparentSamples = [
        [0, 0],
        [2, 0],
        [0, 2],
        [2, 2],
      ].map(([x, y]) => sample(4 + ((x! + 0.5) * 2) / 3, 3 + ((y! + 0.5) * 2) / 3));
      // A missing pack must render every retained footprint, not silently hide it.
      // Probe the opaque amber border at the midpoint of each placement's top edge.
      const placeholderSamples = [
        [5, 3],
        ...Array.from({ length: 14 }, (_, i) => [5.5 + i, 0.875]),
      ].map(([x, y]) => {
        const offset =
          4 *
          (Math.floor(top + (20 + y!) * scale) * bitmap.width +
            Math.floor(left + (4 + x!) * scale));
        return Array.from(pixels.slice(offset, offset + 4));
      });
      return {
        count,
        hash,
        signalSamples,
        transparentSamples,
        placeholderSamples,
        width: bitmap.width,
        height: bitmap.height,
      };
    } finally {
      bitmap.close();
    }
  }, screenshot.toString('base64'));
}

function propState(databasePath: string, digest: string) {
  const database = new Database(databasePath, { readonly: true });
  try {
    const catalogRevision = database
      .prepare('SELECT revision FROM office_prop_catalog WHERE singleton = 1')
      .pluck()
      .get() as number;
    const candidate = database
      .prepare(
        'SELECT digest, bytes, prop_count AS propCount, installed_revision AS installedRevision, installed_at_ms AS installedAtMs FROM office_prop_packs WHERE digest = ?'
      )
      .get(digest) as
      | {
          digest: string;
          bytes: Buffer;
          propCount: number;
          installedRevision: number;
          installedAtMs: number;
        }
      | undefined;
    return { catalogRevision, candidate };
  } finally {
    database.close();
  }
}

/** Sample fixed authored rug pixels, independently of the renderer's tint code. */
async function rugChannelSamples(page: Page, rotation: number) {
  const screenshot = await page.locator('.office-canvas canvas').screenshot({
    path: test.info().outputPath(`rug-pixel-probe-${rotation}.png`),
  });
  return page.evaluate(
    async ({ base64, rotation }) => {
      const bytes = Uint8Array.from(atob(base64), (character) => character.charCodeAt(0));
      const bitmap = await createImageBitmap(new Blob([bytes], { type: 'image/png' }));
      try {
        const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
        const context = canvas.getContext('2d')!;
        context.drawImage(bitmap, 0, 0);
        // Fixed v1 fixture: 22.5 projected floor depth + 16 rear wall +
        // 12 padding. Upright art keeps its height and anchors at its base.
        const margin = Math.min(bitmap.height * 0.2, Math.max(96, bitmap.height * 0.08));
        const scale = Math.min((bitmap.width * 0.84) / 44, (bitmap.height - 2 * margin) / 50.5);
        const left = (bitmap.width - 44 * scale) / 2;
        const top = (bitmap.height - 50.5 * scale) / 2;
        const sample = (x: number, y: number) =>
          Array.from(
            context.getImageData(
              Math.floor(left + (4 + x) * scale),
              Math.floor(top + (20 + y) * scale),
              1,
              1
            ).data
          );
        // Each authored frame contains palette entry 89 (#304730ff) at these
        // positions. The side-view probes stay below the rear wall's shadow
        // after centered rotation. Art is eight source pixels per tile.
        const [x, y] = [
          [8, 11],
          [37, 30],
          [63, 12],
          [25, 36],
        ][rotation]!;
        return {
          customized: sample(
            (rotation % 2 ? 6 : 4) + (x! + 0.5) / 8,
            (((rotation % 2 ? 8 : 10) + (rotation % 2 ? 16 : 12)) * 5) / 8 -
              (rotation % 2 ? 16 : 12) +
              (y! + 0.5) / 8
          ),
          original: sample(20 + 8.5 / 8, (18 * 5) / 8 - 12 + 11.5 / 8),
        };
      } finally {
        bitmap.close();
      }
    },
    { base64: screenshot.toString('base64'), rotation }
  );
}

test('directional workshop art previews exact pixels and saves upright views across restart', async ({
  browser,
  page,
}, testInfo) => {
  test.setTimeout(120_000);
  await withSandbox(async (sandbox) => {
    const prefix = await installNativeOffice(sandbox);
    const office = async (args: string[]) => {
      const result = await runCli(sandbox, ['office', '--prefix', prefix, ...args, '--json'], {
        deadlineMs: 30_000,
      });
      expect(result.status, result.stdout).toBe(0);
      return JSON.parse(result.stdout);
    };
    // Furniture belongs to the installation world; no identity is required.
    const file = path.resolve('../../../../../contracts/office/workshop-furniture-v2.tmtprop.json');
    const source = JSON.parse(readFileSync(file, 'utf8')) as {
      palette: string[];
      props: {
        key: string;
        label: string;
        footprint: { width: number; height: number };
        frames: string[][];
      }[];
    };
    const definition = source.props[0]!;
    const validated = await office(['prop', 'validate', '--file', file]);
    expect(validated.formatVersion).toBe(2);
    // Edge-touching artwork remains admitted, but authors receive an explicit
    // per-frame advisory rather than an unqualified quality approval.
    expect(validated.warnings).toEqual(
      source.props
        .filter((prop) => !['lounge-sofa', 'coffee-table'].includes(prop.key))
        .flatMap((prop) =>
          [0, 1, 2, 3].map((rotation) => ({
            code: 'opaque-edge',
            prop: prop.key,
            rotation,
            message: expect.any(String),
          }))
        )
    );
    const catalog = await office(['prop', 'list', '--local']);
    expect(catalog).toMatchObject({
      catalogRevision: 0,
      builtins: [
        { digest: BUILTIN_DIGEST, formatVersion: 1, builtin: true },
        { digest: WORKSHOP_DIGEST, formatVersion: 2, builtin: true },
        { digest: COMMONS_DIGEST, formatVersion: 2, builtin: true },
        { digest: WHITEBOARD_DIGEST, formatVersion: 2, builtin: true },
        { digest: BROADCASTER_DIGEST, formatVersion: 2, builtin: true },
        { digest: STUDY_DIGEST, formatVersion: 2, builtin: true },
        { digest: WALL_DIGEST, formatVersion: 2, builtin: true },
        { digest: MODULAR_WORKSTATION_DIGEST, formatVersion: 2, builtin: true },
        { digest: MODULAR_MOUNTED_DIGEST, formatVersion: 2, builtin: true },
        { digest: MODULAR_LOUNGE_DIGEST, formatVersion: 2, builtin: true },
        { digest: MODULAR_FACILITIES_DIGEST, formatVersion: 2, builtin: true },
        { digest: MODULAR_RECEPTION_DIGEST, formatVersion: 2, builtin: true },
        { digest: DIRECTIONAL_WORKSTATION_DIGEST, formatVersion: 2, builtin: true },
        { digest: DIRECTIONAL_LOUNGE_DIGEST, formatVersion: 2, builtin: true },
        { digest: DIRECTIONAL_RECEPTION_DIGEST, formatVersion: 2, builtin: true },
        { digest: DIRECTIONAL_FACILITIES_DIGEST, formatVersion: 2, builtin: true },
      ],
      packs: [],
      nextCursor: null,
    });
    const installed = await office([
      'prop',
      'install',
      '--local',
      '--file',
      file,
      '--if-revision',
      '0',
    ]);
    expect(validated.digest).toBe(WORKSHOP_DIGEST);
    expect(installed).toMatchObject({
      digest: validated.digest,
      catalogRevision: 0,
      changed: false,
    });
    expect(propState(sandbox.database, validated.digest)).toEqual({
      catalogRevision: 0,
      candidate: undefined,
    });
    const object = {
      prop: `${validated.digest}/${definition.key}`,
      footprint: definition.footprint,
      x: 11,
      y: 12,
      rotation: 0,
    };
    const layoutFile = path.join(sandbox.root, 'workshop-layout.json');
    writeFileSync(layoutFile, JSON.stringify(worldLayout([object])));
    const initial = await office(['layout', 'show']);
    await office([
      'layout',
      'apply',
      '--legacy-basis',
      initial.legacyBasis,
      '--file',
      layoutFile,
      '--if-revision',
      '0',
    ]);
    let started = await office(['start', '--port', String(await unusedLoopbackPort())]);
    try {
      const beforePreview = savedWorld(sandbox.database);
      const preview = await office(['prop', 'preview', '--file', file]);
      expect(preview.digest).toBe(validated.digest);
      const context = await browser.newContext();
      try {
        const previewPage = await context.newPage();
        for (const packName of [
          'workshop-furniture',
          'study-furniture',
          'commons-props',
          'whiteboard-props',
          'broadcaster-props',
        ]) {
          // The host intentionally keeps at most four live previews. Finish
          // that review session before the fifth pack; do not raise the quota
          // or wait out its TTL merely to produce a contact sheet.
          if (packName === 'broadcaster-props') {
            await previewPage.goto('about:blank');
            await office(['stop']);
            started = await office(['start', '--port', String(await unusedLoopbackPort())]);
          }
          const previewFile = path.resolve(
            `../../../../../contracts/office/${packName}-v2.tmtprop.json`
          );
          const authored = JSON.parse(readFileSync(previewFile, 'utf8')) as typeof source;
          const currentPreview =
            previewFile === file
              ? preview
              : await office(['prop', 'preview', '--file', previewFile]);
          await previewPage.setViewportSize({ width: 1280, height: 900 });
          await previewPage.goto(currentPreview.url);
          for (const previewProp of authored.props) {
            for (const [rotation, direction] of PROP_DIRECTIONS.entries()) {
              const art = previewPage.getByRole('img', {
                name: `${previewProp.label} · ${direction}`,
                exact: true,
              });
              await expect(art).toBeVisible();
              // Each view needs inspection space, not a tall strip squeezed
              // between six other props. Exact raster admission remains below.
              expect((await art.boundingBox())!.width).toBeGreaterThanOrEqual(150);
              const expected = previewProp.frames[rotation]!.flatMap((row, y) =>
                row
                  .match(/../g)!
                  .flatMap((index, x) =>
                    index === '00'
                      ? []
                      : [[String(x), String(y), authored.palette[Number.parseInt(index, 16)]!]]
                  )
              );
              expect(await art.evaluate(readRasterCells)).toEqual(expected);
            }
          }
          await previewPage.screenshot({
            path: testInfo.outputPath(`${packName}-directions.png`),
            fullPage: true,
          });
          if (previewFile === file) {
            await previewPage.setViewportSize({ width: 390, height: 844 });
            expect(
              await previewPage.evaluate(() => document.documentElement.scrollWidth)
            ).toBeLessThanOrEqual(390);
            for (const art of await previewPage.getByRole('img').all())
              expect((await art.boundingBox())!.width).toBeGreaterThanOrEqual(90);
            await previewPage.screenshot({
              path: testInfo.outputPath('workshop-preview-narrow.png'),
              fullPage: true,
            });
          }
        }
      } finally {
        await context.close();
      }
      expect(savedWorld(sandbox.database)).toEqual(beforePreview);
      expect(propState(sandbox.database, validated.digest)).toEqual({
        catalogRevision: 0,
        candidate: undefined,
      });
      await page.setViewportSize({ width: 1440, height: 1000 });
      await page.goto(started.url);
      // A 12x8 desk turns around its fixed ground center, not its top-left.
      const deskAt = (rotation: number) => ({
        ...object,
        rotation,
        x: object.x + (rotation % 2 ? 2 : 0),
        y: object.y - (rotation % 2 ? 2 : 0),
      });
      for (let rotation = 0; rotation < 4; rotation += 1) {
        if (rotation > 0) {
          await editPlacement(page);
          if ((await page.locator('.world-placement-details').getAttribute('open')) === null)
            await page.getByText('Precise placement', { exact: true }).click();
          await page.getByRole('button', { name: 'Rotate 90° clockwise', exact: true }).click();
          expect(savedWorld(sandbox.database).revision).toBe(rotation);
          expect(JSON.parse(savedWorld(sandbox.database).layout)).toEqual(
            worldLayout([deskAt(rotation - 1)])
          );
          await saveLayout(page);
        }
        expect(savedWorld(sandbox.database).revision).toBe(rotation + 1);
        expect(JSON.parse(savedWorld(sandbox.database).layout)).toEqual(
          worldLayout([deskAt(rotation)])
        );
        await page.screenshot({ path: testInfo.outputPath(`workshop-desk-${rotation}.png`) });
      }
      const saved = savedWorld(sandbox.database);
      await office(['stop']);
      started = await office(['start', '--port', String(await unusedLoopbackPort())]);
      await page.goto(started.url);
      await editPlacement(page);
      await expect(page.getByRole('combobox', { name: 'Object', exact: true })).toHaveValue(
        worldLayout([object]).objects[0]!.id
      );
      await page.getByRole('button', { name: 'Clear selection', exact: true }).click();
      expect(savedWorld(sandbox.database)).toEqual(saved);
      expect(propState(sandbox.database, validated.digest)).toEqual({
        catalogRevision: 0,
        candidate: undefined,
      });
      await page.screenshot({ path: testInfo.outputPath('workshop-desk-restored.png') });
      const placed = (key: string, x: number, y: number, rotation = 0) => {
        const prop = source.props.find((entry) => entry.key === key)!;
        return { prop: `${validated.digest}/${key}`, footprint: prop.footprint, x, y, rotation };
      };
      const furnished = [
        placed('woven-rug', 8, 10),
        placed('oak-desk', 10, 11),
        placed('green-chair', 13, 18),
        placed('leafy-plant', 2, 3),
        placed('leafy-plant', 26, 3, 2),
      ];
      writeFileSync(layoutFile, JSON.stringify(worldLayout(furnished)));
      await office(['layout', 'apply', '--file', layoutFile, '--if-revision', '4']);
      await refreshWorld(page);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      expect(JSON.parse(savedWorld(sandbox.database).layout)).toEqual(worldLayout(furnished));
      await page.screenshot({ path: testInfo.outputPath('workshop-furnished-native.png') });

      // Two instances share one definition but not their per-placement values.
      // Exact color probes must be on exposed floor, not below a solid side
      // crown or the full-height foreground wall. Wall editing
      // and occlusion are exercised separately by local-office-composition.
      const rugs = [placed('woven-rug', 4, 10), placed('woven-rug', 20, 6)];
      // This 16x12 rug keeps the same ground center after each quarter turn.
      const rugAt = (rotation: number) => ({
        ...rugs[0]!,
        rotation,
        x: rotation % 2 ? 6 : 4,
        y: rotation % 2 ? 8 : 10,
      });
      writeFileSync(layoutFile, JSON.stringify(worldLayout(rugs)));
      await office(['layout', 'apply', '--file', layoutFile, '--if-revision', '5']);
      await refreshWorld(page);
      await editPlacement(page);
      const beforeCustomization = savedWorld(sandbox.database);
      await page.getByLabel('Tint color', { exact: true }).fill('#ff0080');
      await page.getByLabel('Display text', { exact: true }).fill('STUDIO');
      await page.getByRole('button', { name: 'Apply appearance', exact: true }).click();
      expect(savedWorld(sandbox.database)).toEqual(beforeCustomization);
      for (let rotation = 0; rotation < 4; rotation++) {
        if (rotation > 0) {
          await editPlacement(page);
          if ((await page.locator('.world-placement-details').getAttribute('open')) === null)
            await page.getByText('Precise placement', { exact: true }).click();
          await page.getByRole('button', { name: 'Rotate 90° clockwise', exact: true }).click();
        }
        await saveLayout(page);
        expect(savedWorld(sandbox.database).revision).toBe(7 + rotation);
        expect(JSON.parse(savedWorld(sandbox.database).layout)).toEqual(
          worldLayout([
            { ...rugAt(rotation), customization: { tint: '#ff0080', text: 'STUDIO' } },
            rugs[1]!,
          ])
        );
        await page.screenshot({ path: testInfo.outputPath(`workshop-custom-rug-${rotation}.png`) });
        expect(await rugChannelSamples(page, rotation)).toEqual({
          customized: [71, 0, 36, 255],
          original: [48, 71, 48, 255],
        });
      }
      const customized = savedWorld(sandbox.database);
      await office(['stop']);
      started = await office(['start', '--port', String(await unusedLoopbackPort())]);
      await page.goto(started.url);
      await editPlacement(page);
      await expect(page.getByLabel('Tint color', { exact: true })).toHaveValue('#ff0080');
      await expect(page.getByLabel('Display text', { exact: true })).toHaveValue('STUDIO');
      expect(savedWorld(sandbox.database)).toEqual(customized);
      await page.getByRole('button', { name: 'Clear selection', exact: true }).click();
      expect(await rugChannelSamples(page, 3)).toEqual({
        customized: [71, 0, 36, 255],
        original: [48, 71, 48, 255],
      });
      await editPlacement(page);
      await page.getByRole('button', { name: 'Restore original color' }).click();
      await page.getByLabel('Display text', { exact: true }).fill('');
      await page.getByRole('button', { name: 'Apply appearance', exact: true }).click();
      await saveLayout(page);
      expect(savedWorld(sandbox.database).revision).toBe(11);
      expect(JSON.parse(savedWorld(sandbox.database).layout)).toEqual(
        worldLayout([rugAt(3), rugs[1]!])
      );
    } finally {
      await office(['stop']);
    }
  });
});

test('data-only prop reaches catalog, preview, world renderer and placeholder lifecycle', async ({
  browser,
  page,
}, testInfo) => {
  test.setTimeout(150_000);
  await withSandbox(async (sandbox) => {
    const prefix = await installNativeOffice(sandbox);
    // Catalog admission and placement do not require an identity or private room.
    const propFile = path.join(sandbox.root, 'studio.tmtprop.json');
    const fullLabel = '\\'.repeat(80);
    const capacityProps = Array.from({ length: 14 }, (_, index) => ({
      key: `prop-${String(index).padStart(27, '0')}`,
      label: fullLabel,
      footprint: { width: 1, height: 1 },
      pixels: ['1'],
    }));
    writeFileSync(
      propFile,
      JSON.stringify({
        formatVersion: 1,
        label: 'Studio custom',
        credit: 'Acceptance fixture',
        license: 'CC0-1.0',
        palette: ['#00000000', '#ff5533ff'],
        props: [
          {
            key: 'signal-lamp',
            label: 'Signal lamp',
            footprint: { width: 2, height: 2 },
            pixels: ['010', '111', '010'],
          },
          ...capacityProps,
        ],
      })
    );
    const office = (args: string[]) =>
      runCli(sandbox, ['office', '--prefix', prefix, ...args, '--json'], {
        deadlineMs: 30_000,
      });
    const validated = await office(['prop', 'validate', '--file', propFile]);
    expect(validated.status, validated.stdout).toBe(0);
    const digest = JSON.parse(validated.stdout).digest as string;
    expect(digest).toMatch(/^sha256:[0-9a-f]{64}$/);
    const stoppedPreview = await office(['prop', 'preview', '--file', propFile]);
    expect(stoppedPreview.status).toBe(1);
    expect(stoppedPreview.stdout).toContain('OFFICE_SERVICE_NOT_RUNNING');
    const installed = await office([
      'prop',
      'install',
      '--local',
      '--file',
      propFile,
      '--if-revision',
      '0',
    ]);
    expect(installed.status, installed.stdout).toBe(0);
    expect(JSON.parse(installed.stdout)).toMatchObject({
      digest,
      catalogRevision: 1,
      builtin: false,
      changed: true,
    });
    const installedState = propState(sandbox.database, digest);
    expect(installedState).toMatchObject({
      catalogRevision: 1,
      candidate: {
        digest,
        propCount: 15,
        installedRevision: 1,
        installedAtMs: expect.any(Number),
      },
    });
    expect(installedState.candidate!.bytes).toEqual(readFileSync(propFile));
    expect(installedState.candidate!.installedAtMs).toBeGreaterThan(0);
    const listed = await office(['prop', 'list', '--local']);
    expect(listed.status, listed.stdout).toBe(0);
    expect(JSON.parse(listed.stdout)).toMatchObject({
      catalogRevision: 1,
      builtins: [
        { digest: BUILTIN_DIGEST, builtin: true },
        { digest: WORKSHOP_DIGEST, builtin: true },
        { digest: COMMONS_DIGEST, builtin: true },
        { digest: WHITEBOARD_DIGEST, builtin: true },
        { digest: BROADCASTER_DIGEST, builtin: true },
        { digest: STUDY_DIGEST, builtin: true },
        { digest: WALL_DIGEST, builtin: true },
        { digest: MODULAR_WORKSTATION_DIGEST, builtin: true },
        { digest: MODULAR_MOUNTED_DIGEST, builtin: true },
        { digest: MODULAR_LOUNGE_DIGEST, builtin: true },
        { digest: MODULAR_FACILITIES_DIGEST, builtin: true },
        { digest: MODULAR_RECEPTION_DIGEST, builtin: true },
        { digest: DIRECTIONAL_WORKSTATION_DIGEST, builtin: true },
        { digest: DIRECTIONAL_LOUNGE_DIGEST, builtin: true },
        { digest: DIRECTIONAL_RECEPTION_DIGEST, builtin: true },
        { digest: DIRECTIONAL_FACILITIES_DIGEST, builtin: true },
      ],
      packs: [{ digest, builtin: false }],
    });
    const layoutFile = path.join(sandbox.root, 'world-layout.json');
    writeFileSync(
      layoutFile,
      JSON.stringify(
        worldLayout([
          {
            prop: `${digest}/signal-lamp`,
            footprint: { width: 2, height: 2 },
            x: 4,
            y: 6,
            rotation: 0,
          },
          ...capacityProps.map((prop, index) => ({
            prop: `${digest}/${prop.key}`,
            footprint: prop.footprint,
            // Keep exact-pixel probes clear of architectural side-wall reveals.
            x: 5 + index,
            y: 2,
            rotation: 0,
          })),
          builtinFurniture('desk', 10, 10, 0),
        ])
      )
    );
    const initial = await office(['layout', 'show']);
    expect(initial.status, initial.stdout).toBe(0);
    const applied = await office([
      'layout',
      'apply',
      '--legacy-basis',
      JSON.parse(initial.stdout).legacyBasis,
      '--file',
      layoutFile,
      '--if-revision',
      '0',
    ]);
    expect(applied.status, applied.stdout).toBe(0);
    expect(Buffer.byteLength(applied.stdout)).toBeGreaterThan(4_096);
    expect(JSON.parse(applied.stdout)).toMatchObject({
      revision: 1,
      layout: { version: 1 },
    });
    expect(JSON.parse(applied.stdout).layout.objects).toHaveLength(16);
    const shown = await office(['layout', 'show']);
    expect(shown.status, shown.stdout).toBe(0);
    expect(Buffer.byteLength(shown.stdout)).toBeGreaterThan(4_096);
    expect(JSON.parse(shown.stdout)).toMatchObject({
      revision: 1,
      layout: { version: 1 },
    });
    expect(JSON.parse(shown.stdout).layout.objects).toHaveLength(16);
    const storedWorld = savedWorld(sandbox.database);
    expect(storedWorld).toMatchObject({
      worldId: expect.stringMatching(/^[0-9a-f-]{36}$/),
      revision: 1,
      updatedAtMs: expect.any(Number),
    });
    expect(storedWorld.updatedAtMs).toBeGreaterThan(0);
    expect(JSON.parse(storedWorld.layout)).toEqual(JSON.parse(readFileSync(layoutFile, 'utf8')));

    let started = await office(['start', '--port', String(await unusedLoopbackPort())]);
    expect(started.status, started.stdout).toBe(0);
    let sessionUrl = JSON.parse(started.stdout).url as string;
    try {
      const beforePreviews = {
        prop: propState(sandbox.database, digest),
        world: savedWorld(sandbox.database),
      };
      const preview = await office(['prop', 'preview', '--file', propFile]);
      expect(preview.status, preview.stdout).toBe(0);
      const firstPreview = JSON.parse(preview.stdout);
      const repeatedPreview = await office(['prop', 'preview', '--file', propFile]);
      expect(repeatedPreview.status, repeatedPreview.stdout).toBe(0);
      expect(JSON.parse(repeatedPreview.stdout)).toMatchObject({
        previewId: firstPreview.previewId,
        expiresAtMs: firstPreview.expiresAtMs,
      });
      for (let index = 1; index <= 3; index += 1) {
        const variant = path.join(sandbox.root, `studio-${index}.tmtprop.json`);
        const document = JSON.parse(readFileSync(propFile, 'utf8'));
        document.label = `Studio custom ${index}`;
        writeFileSync(variant, JSON.stringify(document));
        const accepted = await office(['prop', 'preview', '--file', variant]);
        expect(accepted.status, accepted.stdout).toBe(0);
      }
      const overflow = path.join(sandbox.root, 'studio-overflow.tmtprop.json');
      const overflowDocument = JSON.parse(readFileSync(propFile, 'utf8'));
      overflowDocument.label = 'Studio overflow';
      writeFileSync(overflow, JSON.stringify(overflowDocument));
      const limited = await office(['prop', 'preview', '--file', overflow]);
      expect(limited.status).toBe(1);
      expect(limited.stdout).toContain('OFFICE_PROP_PREVIEW_LIMIT');
      const previewContext = await browser.newContext();
      try {
        const previewPage = await previewContext.newPage();
        await previewPage.goto(firstPreview.url);
        await expect(previewPage.getByRole('heading', { name: 'Signal lamp' })).toBeVisible();
        expect(
          (await previewPage.locator('body').evaluate(readRasterCells)).filter(
            ([, , color]) => color === '#ff5533ff'
          )
        ).toHaveLength(19);
      } finally {
        await previewContext.close();
      }
      expect({
        prop: propState(sandbox.database, digest),
        world: savedWorld(sandbox.database),
      }).toEqual(beforePreviews);

      await page.setViewportSize({ width: 1280, height: 900 });
      await page.goto(sessionUrl);
      await editPlacement(page);
      await expect(
        page.getByRole('combobox', { name: 'Object', exact: true }).locator('option')
      ).toHaveCount(17);
      await page.getByRole('button', { name: 'Clear selection', exact: true }).click();
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      await expect.poll(async () => (await signalColorMask(page)).count).toBeGreaterThan(0);
      const renderedSignal = await signalColorMask(page);
      expect(renderedSignal.signalSamples).toEqual(Array(19).fill(1));
      expect(renderedSignal.transparentSamples).toEqual(Array(4).fill(0));
      await page.screenshot({
        path: testInfo.outputPath('builtin-custom-props-desktop.png'),
        fullPage: true,
      });
      await page.setViewportSize({ width: 390, height: 844 });
      await editPlacement(page);
      const selection = page.getByRole('combobox', { name: 'Object', exact: true });
      await expect(selection).toBeVisible();
      const tools = await page.getByRole('complementary', { name: 'Layout tools' }).boundingBox();
      const selector = await selection.boundingBox();
      expect(tools).not.toBeNull();
      expect(selector).not.toBeNull();
      expect(tools!.x).toBeGreaterThanOrEqual(0);
      expect(tools!.x + tools!.width).toBeLessThanOrEqual(390);
      expect(selector!.x).toBeGreaterThanOrEqual(tools!.x);
      expect(selector!.x + selector!.width).toBeLessThanOrEqual(tools!.x + tools!.width);
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        390
      );
      await expect(selection.locator('option')).toHaveCount(17);
      await page.screenshot({
        path: testInfo.outputPath('builtin-custom-props-narrow.png'),
        fullPage: true,
      });
      await page.getByRole('button', { name: 'Clear selection', exact: true }).click();

      const removed = await office(['prop', 'remove', '--local', digest, '--if-revision', '1']);
      expect(removed.status, removed.stdout).toBe(0);
      expect(propState(sandbox.database, digest)).toEqual({
        catalogRevision: 2,
        candidate: undefined,
      });
      expect(savedWorld(sandbox.database)).toEqual(storedWorld);
      await refreshWorld(page);
      await page.setViewportSize({ width: 1280, height: 900 });
      await expect.poll(async () => (await signalColorMask(page)).count).toBe(0);
      expect((await signalColorMask(page)).placeholderSamples).toEqual(
        Array.from({ length: 15 }, () => [245, 216, 144, 255])
      );
      await page.screenshot({
        path: testInfo.outputPath('unavailable-props-desktop.png'),
        fullPage: true,
      });
      await page.setViewportSize({ width: 390, height: 844 });
      await page.screenshot({
        path: testInfo.outputPath('unavailable-props-narrow.png'),
        fullPage: true,
      });

      const restored = await office([
        'prop',
        'install',
        '--local',
        '--file',
        propFile,
        '--if-revision',
        '2',
      ]);
      expect(restored.status, restored.stdout).toBe(0);
      const restoredState = propState(sandbox.database, digest);
      expect(restoredState).toMatchObject({
        catalogRevision: 3,
        candidate: { digest, propCount: 15, installedRevision: 3 },
      });
      expect(restoredState.candidate!.bytes).toEqual(readFileSync(propFile));
      expect(savedWorld(sandbox.database)).toEqual(storedWorld);
      await refreshWorld(page);
      await page.setViewportSize({ width: 1280, height: 900 });
      await expect.poll(() => signalColorMask(page)).toEqual(renderedSignal);

      const database = new Database(sandbox.database);
      try {
        database
          .prepare('UPDATE office_prop_packs SET bytes = ? WHERE digest = ?')
          .run(Buffer.from('{}'), digest);
      } finally {
        database.close();
      }
      expect(savedWorld(sandbox.database)).toEqual(storedWorld);
      await refreshWorld(page);
      await expect.poll(async () => (await signalColorMask(page)).count).toBe(0);
      expect((await signalColorMask(page)).placeholderSamples).toEqual(
        Array.from({ length: 15 }, () => [245, 216, 144, 255])
      );

      expect((await office(['stop'])).status).toBe(0);
      started = await office(['start', '--port', String(await unusedLoopbackPort())]);
      expect(started.status, started.stdout).toBe(0);
      sessionUrl = JSON.parse(started.stdout).url;
      expect(savedWorld(sandbox.database)).toEqual(storedWorld);
      await page.goto(sessionUrl);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      await expect.poll(async () => (await signalColorMask(page)).count).toBe(0);
      expect((await signalColorMask(page)).placeholderSamples).toEqual(
        Array.from({ length: 15 }, () => [245, 216, 144, 255])
      );
    } finally {
      const stopped = await office(['stop']);
      expect(stopped.status, stopped.stdout).toBe(0);
      expect(JSON.parse(stopped.stdout)).toMatchObject({ running: false });
      expect(typeof JSON.parse(stopped.stdout).changed).toBe('boolean');
    }
  });
});
