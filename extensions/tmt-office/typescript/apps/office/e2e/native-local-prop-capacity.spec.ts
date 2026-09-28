import Database from '../../../../../../typescript/test/support/sqlite-oracle.js';
import { createHash } from 'node:crypto';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import type { Sandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import { officeWorldFixture } from '../../../../../../typescript/test/support/office-world.js';
import type { WorldDocument } from '../src/world-map/world-contract.js';
import { installDrawObserver, observeIdleScene } from './scene-observation.js';
import { openKeyboardSelection } from './office-navigation.js';
import {
  capacityPropSource,
  CAPACITY_PACK_BYTES,
  CAPACITY_PACK_CELLS,
} from './prop-capacity-fixture.js';

function capacityCommands(sandbox: Sandbox, prefix: string) {
  const cli = async (args: string[]) => {
    const result = await runCli(sandbox, [...args, '--json'], { deadlineMs: 30_000 });
    expect(result.status, `${args.join(' ')}: ${result.stdout}`).toBe(0);
    expect(result.stderr).toBe('');
    return { value: JSON.parse(result.stdout), bytes: Buffer.byteLength(result.stdout) };
  };
  return { cli, office: (args: string[]) => cli(['office', '--prefix', prefix, ...args]) };
}

function fullCapacitySource(index: number, propCount: 8 | 16) {
  const source = capacityPropSource(index, propCount);
  expect(Buffer.byteLength(source)).toBe(CAPACITY_PACK_BYTES);
  const document = JSON.parse(source);
  expect(document.palette).toHaveLength(256);
  expect(document.props).toHaveLength(propCount);
  const cells = document.props.reduce(
    (total: number, prop: { frames: string[][] }) =>
      total +
      prop.frames.reduce((sum, frame) => sum + frame.reduce((n, row) => n + row.length / 2, 0), 0),
    0
  );
  expect(cells).toBe(CAPACITY_PACK_CELLS);
  return source;
}

function storedCatalog(database: Database.Database) {
  const packs = database
    .prepare(
      'SELECT digest, bytes, prop_count, installed_revision, installed_at_ms FROM office_prop_packs ORDER BY digest'
    )
    .all() as {
    digest: string;
    bytes: Buffer;
    prop_count: number;
    installed_revision: number;
    installed_at_ms: number;
  }[];
  return {
    catalog: database.prepare('SELECT * FROM office_prop_catalog').get(),
    sources: {
      count: packs.length,
      bytes: packs.reduce((total, pack) => total + pack.bytes.length, 0),
      props: packs.reduce((total, pack) => total + pack.prop_count, 0),
    },
    packs: packs.map(({ bytes, ...metadata }) => ({
      ...metadata,
      contentHash: createHash('sha256').update(bytes).digest('hex'),
    })),
  };
}

/** Sample authored grayscale cells from every placement, not labels or scene internals. */
async function capacityPixels(page: Page, outputPath: string) {
  const screenshot = await page.locator('.office-canvas canvas').screenshot({ path: outputPath });
  return page.evaluate(async (base64) => {
    const bytes = Uint8Array.from(atob(base64), (character) => character.charCodeAt(0));
    const bitmap = await createImageBitmap(new Blob([bytes], { type: 'image/png' }));
    try {
      const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
      const context = canvas.getContext('2d')!;
      context.drawImage(bitmap, 0, 0);
      // Authored 36x64 floor with 5/8 floor depth and full sixteen-tile walls:
      // camera bounds (-4,-20,44,68), with the reserved HUD framing band.
      // Upright art stays unscaled and bottom-anchored. No production projection
      // is the oracle; these samples test the visible frame, not hidden back rows.
      const margin = Math.min(bitmap.height * 0.2, Math.max(96, bitmap.height * 0.08));
      const scale = Math.min((bitmap.width * 0.84) / 44, (bitmap.height - 2 * margin) / 68);
      const left = (bitmap.width - 44 * scale) / 2;
      const top = (bitmap.height - 68 * scale) / 2;
      return Array.from({ length: 16 }, (_, index) =>
        [
          // Stay beyond the west wall's three-tile inward silhouette.
          [30, 7],
          [34, 3],
        ].map(([x, y]) =>
          Array.from(
            context.getImageData(
              Math.floor(left + (4 + (index % 2) * 16 + (x! + 0.5) / 8) * scale),
              Math.floor(
                top + (20 + ((4 + Math.floor(index / 2) * 2) * 5) / 8 - 2 + (y! + 0.5) / 8) * scale
              ),
              1,
              1
            ).data
          )
        )
      );
    } finally {
      bitmap.close();
    }
  }, screenshot.toString('base64'));
}

test('sixteen full v2 packs resolve in the browser and reject catalog overflow without mutation', async ({
  page,
}, testInfo) => {
  await installDrawObserver(page);
  await withSandbox(async (sandbox) => {
    const prefix = await installNativeOffice(sandbox);
    const { office } = capacityCommands(sandbox, prefix);
    const file = path.join(sandbox.root, 'capacity.tmtprop.json');
    const digests: string[] = [];
    for (let index = 0; index < 16; index++) {
      const source = fullCapacitySource(index, 16);
      await writeFile(file, source);
      const installed = await office([
        'prop',
        'install',
        '--local',
        '--file',
        file,
        '--if-revision',
        String(index),
      ]);
      digests.push(installed.value.digest);
    }
    expect(new Set(digests).size).toBe(16);
    const pageResult = await office(['prop', 'list', '--local', '--limit', '20']);
    expect(pageResult.value.packs).toHaveLength(16);
    expect(pageResult.value.excluded).toEqual([]);
    expect(pageResult.value.nextCursor).toBeNull();
    // Summaries must not accidentally embed the multi-megabyte raster documents.
    expect(pageResult.bytes).toBeLessThan(512 * 1024);
    const layoutFile = path.join(sandbox.root, 'capacity-layout.json');
    const initial = (await office(['layout', 'show'])).value;
    const fixture = officeWorldFixture().layout;
    const layout: WorldDocument = {
      ...fixture,
      // Leave enough foreground depth for all sixteen raster samples to remain
      // visible above the full-height front wall. This is a catalog capacity
      // fixture, not a cutaway/occlusion test.
      map: {
        ...fixture.map,
        floor: Array.from({ length: 64 }, (_, y) => ({
          y,
          start: 0,
          end: 36,
          areaId: fixture.map.primaryLobbyId,
        })),
      },
      objects: digests.slice(0, 16).map((digest, index) => ({
        id: `30000000-0000-4000-8000-${String(index + 1).padStart(12, '0')}`,
        kind: 'decoration',
        surface: { type: 'floor' },
        extension: null,
        placement: {
          prop: `${digest}/tile-0`,
          footprint: { width: 16, height: 2 },
          x: (index % 2) * 16,
          y: 2 + Math.floor(index / 2) * 2,
          rotation: 0,
        },
      })),
    };
    await writeFile(layoutFile, JSON.stringify(layout));
    await office([
      'layout',
      'apply',
      '--file',
      layoutFile,
      '--if-revision',
      '0',
      '--legacy-basis',
      initial.legacyBasis,
    ]);
    const stored = () => {
      const db = new Database(sandbox.database, { readonly: true });
      try {
        return {
          ...storedCatalog(db),
          world: db
            .prepare(
              'SELECT layout_revision, layout_json, layout_updated_at_ms FROM office_local_worlds'
            )
            .get(),
        };
      } finally {
        db.close();
      }
    };
    const before = stored();
    expect(before.sources).toEqual({ count: 16, bytes: 16 * CAPACITY_PACK_BYTES, props: 256 });
    // Sixteen full packs exhaust the existing 256-prop catalog, not its
    // 64-pack quota. A rejected seventeenth pack must neither evict nor mutate.
    await writeFile(file, fullCapacitySource(16, 16));
    const overflow = await runCli(
      sandbox,
      [
        'office',
        '--prefix',
        prefix,
        'prop',
        'install',
        '--local',
        '--file',
        file,
        '--if-revision',
        '16',
        '--json',
      ],
      { deadlineMs: 30_000 }
    );
    expect(overflow.status).not.toBe(0);
    expect(JSON.parse(overflow.stdout).error.code).toBe('OFFICE_PROP_LIMIT');
    expect(stored()).toEqual(before);
    const started = (await office(['start', '--port', String(await unusedLoopbackPort())])).value;
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    const session = await page.context().newCDPSession(page);
    await session.send('Performance.enable');
    try {
      await page.setViewportSize({ width: 1280, height: 900 });
      // Capture real native bytes while forwarding them unchanged, without
      // depending on DevTools retaining the multi-megabyte response body.
      // Browser decoding/rendering still consume that response. The diagnostic
      // elapsed time includes interception and is not a raw-network benchmark.
      const resolvedBodies: Buffer[] = [];
      await page.route('**/api/v1/local/props/resolve', async (route) => {
        if (route.request().postDataJSON().digests.length !== 16) {
          await route.continue();
          return;
        }
        const response = await route.fetch();
        expect(response.status()).toBe(200);
        const body = await response.body();
        resolvedBodies.push(body);
        await route.fulfill({ response, body });
      });
      const began = performance.now();
      await page.goto(started.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      expect(resolvedBodies.length).toBeGreaterThan(0);
      const body = resolvedBodies[0]!;
      expect(body.byteLength).toBeLessThanOrEqual(16 * CAPACITY_PACK_BYTES);
      const result = JSON.parse(body.toString());
      expect(result.catalogRevision).toBe(16);
      expect(result.unavailable).toEqual([]);
      expect(result.packs.map((entry: { digest: string }) => entry.digest).sort()).toEqual(
        digests.slice(0, 16).sort()
      );
      await openKeyboardSelection(page);
      await expect(
        page.getByRole('combobox', { name: 'Object', exact: true }).locator('option')
      ).toHaveText([
        'Select an object',
        ...Array.from({ length: 16 }, (_, index) => `${index + 1} · Capacity tile 0`),
      ]);
      await page.getByText('Keyboard selection', { exact: true }).click();
      const elapsedToReadyMs = performance.now() - began;
      await observeIdleScene(page, testInfo, 'full-v2-catalog');
      // The fixture's frame-zero palette indices at (30,7) and (34,3) are 218 and 162.
      expect(await capacityPixels(page, testInfo.outputPath('full-v2-catalog-pixels.png'))).toEqual(
        Array.from({ length: 16 }, () => [
          [218, 218, 218, 255],
          [162, 162, 162, 255],
        ])
      );
      const { metrics } = await session.send('Performance.getMetrics');
      const measurement = {
        installedSourceBytes: 16 * CAPACITY_PACK_BYTES,
        resolvedSourceBytes: 16 * CAPACITY_PACK_BYTES,
        resolvedCells: 16 * CAPACITY_PACK_CELLS,
        responseBytes: body.byteLength,
        catalogSummaryBytes: pageResult.bytes,
        elapsedToReadyMs,
        metrics: metrics.filter((entry) =>
          ['JSHeapUsedSize', 'Nodes', 'TaskDuration', 'ScriptDuration'].includes(entry.name)
        ),
        note: 'One headless browser run; elapsed time includes navigation and editor setup. Heap is not peak process or GPU memory. No cross-machine performance claim.',
      };
      const report = testInfo.outputPath('full-v2-capacity.json');
      await writeFile(report, JSON.stringify(measurement, null, 2));
      await testInfo.attach('full-v2-capacity', { path: report, contentType: 'application/json' });
      await page.screenshot({ path: testInfo.outputPath('full-v2-capacity.png') });
      expect(stored()).toEqual(before);
      expect(errors).toEqual([]);
    } finally {
      await session.detach();
      await page.goto('about:blank');
      await office(['stop']);
      expect((await office(['status'])).value.service.running).toBe(false);
    }
  });
});

test('a full twenty-row v2 candidate page preserves exact packs and bounded metadata', async () => {
  const testInfo = test.info();
  await withSandbox(async (sandbox) => {
    const prefix = await installNativeOffice(sandbox);
    const { office } = capacityCommands(sandbox, prefix);
    const file = path.join(sandbox.root, 'candidate.tmtprop.json');
    const digests: string[] = [];
    // Eight larger props still use every cell and source byte. Twenty packs
    // then fit the existing total-prop quota instead of testing an impossible
    // twenty-times-sixteen catalog. Browser max-prop-count is covered above.
    for (let index = 0; index < 20; index++) {
      await writeFile(file, fullCapacitySource(index, 8));
      const installed = await office([
        'prop',
        'install',
        '--local',
        '--file',
        file,
        '--if-revision',
        String(index),
      ]);
      digests.push(installed.value.digest);
    }
    expect(new Set(digests).size).toBe(20);
    const db = new Database(sandbox.database, { readonly: true });
    try {
      const before = storedCatalog(db);
      expect(before.sources).toEqual({ count: 20, bytes: 20 * CAPACITY_PACK_BYTES, props: 160 });
      const listed = await office(['prop', 'list', '--local', '--limit', '20']);
      expect(listed.value.packs.map((pack: { digest: string }) => pack.digest)).toEqual(
        digests.sort()
      );
      expect(listed.value.excluded).toEqual([]);
      expect(listed.value.nextCursor).toBeNull();
      expect(listed.value.catalogRevision).toBe(20);
      expect(listed.bytes).toBeLessThan(512 * 1024);
      const first = await office(['prop', 'list', '--local', '--limit', '19']);
      expect(first.value.nextCursor).toEqual(expect.any(String));
      const last = await office([
        'prop',
        'list',
        '--local',
        '--limit',
        '19',
        '--cursor',
        first.value.nextCursor,
      ]);
      expect([...first.value.packs, ...last.value.packs]).toEqual(listed.value.packs);
      expect(last.value.nextCursor).toBeNull();
      expect(storedCatalog(db)).toEqual(before);
      const report = testInfo.outputPath('full-v2-candidate-page.json');
      await writeFile(
        report,
        JSON.stringify(
          {
            candidateSourceBytes: before.sources.bytes,
            candidateCells: 20 * CAPACITY_PACK_CELLS,
            catalogSummaryBytes: listed.bytes,
            candidates: before.sources.count,
            props: before.sources.props,
          },
          null,
          2
        )
      );
      await testInfo.attach('full-v2-candidate-page', {
        path: report,
        contentType: 'application/json',
      });
    } finally {
      db.close();
    }
  });
});
