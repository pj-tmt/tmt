import { mapGeometry } from '../src/world-map/map-source.js';
import { writeFileSync } from 'node:fs';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import type { Sandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { officeWorldFixture } from '../../../../../../typescript/test/support/office-world.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import { savedWorld } from './native-world-state.js';
import { installDrawObserver, observeIdleScene, sceneActivity } from './scene-observation.js';
import type { SceneActivity } from './scene-observation.js';
import type { WorldDocument } from '../src/world-map/world-contract.js';
import type { WorldSnapshot } from '../src/world-map/world-port.js';
import { WORKSHOP_DIGEST } from '../src/props/prop-contract.js';

async function officeIn(sandbox: Sandbox) {
  const prefix = await installNativeOffice(sandbox);
  return async (args: string[]) => {
    const result = await runCli(sandbox, ['office', '--prefix', prefix, ...args, '--json'], {
      deadlineMs: 30_000,
    });
    expect(result.status, result.stdout + result.stderr).toBe(0);
    return JSON.parse(result.stdout);
  };
}

async function paintCost(page: Page, action: () => Promise<unknown>) {
  const before = await sceneActivity(page);
  await action();
  const after = await sceneActivity(page);
  return {
    draws: after.draws - before.draws,
    submittedVertices: after.submittedVertices - before.submittedVertices,
    resources: after,
  };
}

async function pan(page: Page, direction: 1 | -1) {
  await page.mouse.move(640, 450);
  await page.mouse.down();
  await page.mouse.move(640 + direction * 480, 450 + direction * 300, { steps: 3 });
  await page.mouse.up();
  return sceneActivity(page);
}

test('a full tile-budget native world culls submissions, releases offscreen art, stays idle and reloads without cache growth', async ({
  page,
}, info) => {
  await withSandbox(async (sandbox) => {
    const office = await officeIn(sandbox);
    const initial: WorldSnapshot = await office(['layout', 'show']);
    const base = officeWorldFixture().layout;
    const layout: WorldDocument = {
      ...base,
      map: {
        ...base.map,
        floor: Array.from({ length: 512 }, (_, y) => ({
          y,
          start: 0,
          end: 512,
          areaId: base.map.primaryLobbyId,
        })),
      },
      objects: Array.from({ length: 64 }, (_, index) => ({
        id: `30000000-0000-4000-8000-${String(index + 1).padStart(12, '0')}`,
        kind: 'decoration',
        placement: {
          prop: `${WORKSHOP_DIGEST}/woven-rug`,
          footprint: { width: 16, height: 12 },
          x: 32 + (index % 8) * 64,
          y: 32 + Math.floor(index / 8) * 64,
          rotation: 0,
          customization: {
            text: `R${index}`,
            tint: `#${(((index + 1) * 0x1f1f1f) % 0xffffff).toString(16).padStart(6, '0')}`,
          },
        },
        surface: { type: 'floor' },
        extension: null,
      })),
    };
    expect(
      mapGeometry(layout.map).floor.reduce((sum, span) => sum + span.end - span.start, 0)
    ).toBe(262_144);
    const file = path.join(sandbox.root, 'capacity-world.json');
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
    const original = savedWorld(sandbox.database);
    expect(JSON.parse(original.layout)).toEqual(layout);
    let started = await office(['start', '--port', String(await unusedLoopbackPort())]);
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await installDrawObserver(page);
    const session = await page.context().newCDPSession(page);
    await session.send('Performance.enable');
    try {
      await page.setViewportSize({ width: 1280, height: 900 });
      const began = performance.now();
      await page.goto(started.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      const elapsedToReadyMs = performance.now() - began;
      const canvas = await page.locator('.office-canvas canvas').elementHandle();
      const fit = () => page.getByRole('button', { name: 'Fit office', exact: true }).click();
      const fitted = await paintCost(page, fit);
      expect(fitted.draws).toBeGreaterThan(0);
      expect(fitted.resources.texturesLive).toBeGreaterThanOrEqual(64);
      await observeIdleScene(page, info, 'full-map-fit');
      await page.screenshot({ path: info.outputPath('full-map-fit.png') });
      for (let index = 0; index < 10; index++)
        await page.getByRole('button', { name: 'Zoom in', exact: true }).click();
      const zoomed = await paintCost(page, () =>
        page.getByRole('button', { name: 'Zoom in', exact: true }).click()
      );
      expect(zoomed.draws).toBeGreaterThan(0);
      expect(zoomed.submittedVertices).toBeLessThan(fitted.submittedVertices);
      expect(zoomed.resources.texturesLive).toBeLessThan(fitted.resources.texturesLive);
      expect(zoomed.resources.texturesDeleted).toBeGreaterThan(fitted.resources.texturesDeleted);
      await observeIdleScene(page, info, 'full-map-zoomed');
      await page.screenshot({ path: info.outputPath('full-map-zoomed.png') });
      const revisits: SceneActivity[] = [];
      for (let cycle = 0; cycle < 3; cycle++) {
        for (let step = 0; step < 3; step++) await pan(page, 1);
        for (let step = 0; step < 3; step++) await pan(page, -1);
        revisits.push(await sceneActivity(page));
      }
      expect(revisits[2]!.texturesLive).toBe(revisits[1]!.texturesLive);
      expect(revisits[2]!.texturesCreated).toBeGreaterThan(revisits[1]!.texturesCreated);
      expect(revisits[2]!.texturesDeleted).toBeGreaterThan(revisits[1]!.texturesDeleted);
      expect(revisits[2]!.texturesLive).toBeLessThan(fitted.resources.texturesLive);
      await observeIdleScene(page, info, 'full-map-revisited');
      expect(savedWorld(sandbox.database)).toEqual(original);
      const refresh = async () => {
        const read = page.waitForResponse(
          (response) => new URL(response.url()).pathname === '/api/v1/local/world'
        );
        await page.getByRole('button', { name: 'Refresh office', exact: true }).click();
        expect((await read).status()).toBe(200);
        await expect(
          page.getByRole('button', { name: 'Refresh office', exact: true })
        ).toBeEnabled();
        return sceneActivity(page);
      };
      writeFileSync(file, JSON.stringify({ ...layout, objects: [] }));
      await office(['layout', 'apply', '--file', file, '--if-revision', String(original.revision)]);
      const empty = await refresh();
      expect(empty.texturesLive).toBeLessThan(revisits[2]!.texturesLive);
      expect(empty.texturesDeleted).toBeGreaterThan(revisits[2]!.texturesDeleted);
      writeFileSync(file, JSON.stringify(layout));
      await office([
        'layout',
        'apply',
        '--file',
        file,
        '--if-revision',
        String(original.revision + 1),
      ]);
      await refresh();
      await expect
        .poll(async () => (await sceneActivity(page)).texturesLive)
        .toBe(revisits[2]!.texturesLive);
      const restored = await sceneActivity(page);
      expect(restored.texturesLive).toBe(revisits[2]!.texturesLive);
      expect(await canvas!.evaluate((element) => element.isConnected)).toBe(true);
      const durable = savedWorld(sandbox.database);
      expect(JSON.parse(durable.layout)).toEqual(layout);
      await observeIdleScene(page, info, 'full-map-restored');
      const { metrics } = await session.send('Performance.getMetrics');
      const report = info.outputPath('world-capacity.json');
      writeFileSync(
        report,
        JSON.stringify(
          {
            tileCount: 262_144,
            floorSpanCount: 512,
            distinctArtVariants: 64,
            elapsedToReadyMs,
            fitted,
            zoomed,
            revisits,
            empty,
            restored,
            metrics: metrics.filter((metric) =>
              ['JSHeapUsedSize', 'TaskDuration', 'Nodes'].includes(metric.name)
            ),
            note: 'One headless run. Submitted vertices and explicit WebGL texture lifetimes are not GPU time/bytes; JS heap is not total browser memory. No cross-machine speed threshold.',
          },
          null,
          2
        )
      );
      await info.attach('world-capacity', { path: report, contentType: 'application/json' });
      await page.goto('about:blank');
      await office(['stop']);
      started = await office(['start', '--port', String(await unusedLoopbackPort())]);
      await page.goto(started.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      await observeIdleScene(page, info, 'full-map-restarted');
      expect(savedWorld(sandbox.database)).toEqual(durable);
      expect(errors).toEqual([]);
    } finally {
      await session.detach();
      await page.goto('about:blank');
      await office(['stop']);
      expect((await office(['status'])).service.running).toBe(false);
    }
  });
});

test('a connected sparse world culls thousands of occupied chunks without a continuous render loop', async ({
  page,
}, info) => {
  await withSandbox(async (sandbox) => {
    const office = await officeIn(sandbox);
    const initial: WorldSnapshot = await office(['layout', 'show']);
    const base = officeWorldFixture().layout;
    // A one-cell spine connects 98 full-width rows and two 16-row pads.
    // Nearly the dense fixture's tile budget, but spread over 6,200 floor chunks.
    // The expected counts are fixture arithmetic, not renderer-derived answers.
    const layout: WorldDocument = {
      ...base,
      objects: [],
      map: {
        ...base.map,
        floor: Array.from({ length: 3184 }, (_, y) => ({
          y,
          start: 0,
          end: y < 16 || y >= 3168 || y % 32 === 0 ? 1984 : 1,
          areaId: base.map.primaryLobbyId,
        })),
      },
    };
    expect(
      mapGeometry(layout.map).floor.reduce((sum, span) => sum + span.end - span.start, 0)
    ).toBe(260_974);
    const file = path.join(sandbox.root, 'sparse-world.json');
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
    const original = savedWorld(sandbox.database);
    expect(JSON.parse(original.layout)).toEqual(layout);
    const started = await office(['start', '--port', String(await unusedLoopbackPort())]);
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await installDrawObserver(page);
    try {
      await page.setViewportSize({ width: 1280, height: 900 });
      const began = performance.now();
      await page.goto(started.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      const elapsedToReadyMs = performance.now() - began;
      const fitted = await paintCost(page, () =>
        page.getByRole('button', { name: 'Fit office', exact: true }).click()
      );
      expect(fitted.draws).toBeGreaterThan(0);
      await observeIdleScene(page, info, 'sparse-fit');
      await page.screenshot({ path: info.outputPath('sparse-fit.png') });
      await page.mouse.move(640, 450);
      const zoomed = await paintCost(page, () => page.mouse.wheel(0, -4000));
      expect(zoomed.draws).toBeGreaterThan(0);
      expect(zoomed.submittedVertices).toBeGreaterThan(0);
      expect(zoomed.submittedVertices).toBeLessThan(fitted.submittedVertices);
      const revisits: SceneActivity[] = [];
      for (let cycle = 0; cycle < 3; cycle++) {
        for (let step = 0; step < 3; step++) await pan(page, 1);
        for (let step = 0; step < 3; step++) await pan(page, -1);
        revisits.push(await sceneActivity(page));
      }
      expect(revisits[2]!.draws).toBeGreaterThan(revisits[1]!.draws);
      expect(revisits[2]!.texturesLive).toBe(revisits[1]!.texturesLive);
      await observeIdleScene(page, info, 'sparse-revisited');
      await page.screenshot({ path: info.outputPath('sparse-zoomed.png') });
      expect(savedWorld(sandbox.database)).toEqual(original);
      expect(errors).toEqual([]);
      const report = info.outputPath('sparse-capacity.json');
      writeFileSync(
        report,
        JSON.stringify(
          {
            tileCount: 260_974,
            floorSpanCount: 3184,
            occupiedFloorChunks: 6200,
            elapsedToReadyMs,
            fitted,
            zoomed,
            revisits,
            note: 'One headless run; explicit WebGL submissions and resource counts, not GPU timing or bytes. No cross-machine speed threshold.',
          },
          null,
          2
        )
      );
      await info.attach('sparse-capacity', { path: report, contentType: 'application/json' });
    } finally {
      await page.goto('about:blank');
      await office(['stop']);
      expect((await office(['status'])).service.running).toBe(false);
    }
  });
});
