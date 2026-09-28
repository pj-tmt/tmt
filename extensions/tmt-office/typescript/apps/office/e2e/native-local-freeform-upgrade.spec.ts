import { writeFileSync } from 'node:fs';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import { savedWorld } from './native-world-state.js';
import {
  officeWorldFixture,
  legacyLobbyObjects,
} from '../../../../../../typescript/test/support/office-world.js';
import vectors from '../../../../../../contracts/office/modules-v2-vectors.json' with { type: 'json' };
import { decodeModuleMap } from '../src/world-map/module-contract.js';
import { mapGeometry } from '../src/world-map/map-source.js';
import type { WorldSnapshot } from '../src/world-map/world-port.js';

test('free-form conversion preserves objects and resource attachments through auto-apply, history and restart', async ({
  page,
}, info) => {
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
    const desk = officeWorldFixture().layout.objects[0]!;
    const source = {
      version: 1,
      // Retain the old generated floor as a real v1 source, not a version toggle.
      map: mapGeometry(decodeModuleMap(vectors.starter)),
      objects: [
        ...legacyLobbyObjects(),
        {
          ...desk,
          id: '30000000-0000-4000-8000-000000000090',
          placement: { ...desk.placement, x: 8, y: 52 },
        },
      ],
    };
    const file = path.join(sandbox.root, 'freeform.json');
    writeFileSync(file, JSON.stringify(source));
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
    const before = savedWorld(sandbox.database);
    const nextWorldWrite = () =>
      page.waitForResponse(
        (response) =>
          new URL(response.url()).pathname === '/api/v1/local/world' &&
          response.request().method() === 'PUT'
      );
    const open = async () => {
      const started = await office<{ url: string }>([
        'start',
        '--port',
        String(await unusedLoopbackPort()),
      ]);
      await page.goto(started.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
    };
    try {
      await page.setViewportSize({ width: 1536, height: 1024 });
      await open();
      const firstPreviewWrite = nextWorldWrite();
      await page.getByRole('button', { name: 'Preview modular layout', exact: true }).click();
      await expect(page.getByRole('button', { name: 'Add floor', exact: true })).toHaveCount(0);
      expect((await firstPreviewWrite).status()).toBe(200);
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      const previewed = savedWorld(sandbox.database);
      expect(previewed.worldId).toBe(before.worldId);
      expect(previewed.revision).toBeGreaterThan(before.revision);
      const firstUndoWrite = nextWorldWrite();
      await page.getByRole('button', { name: 'Undo', exact: true }).click();
      expect((await firstUndoWrite).status()).toBe(200);
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      const restored = savedWorld(sandbox.database);
      expect(restored.worldId).toBe(before.worldId);
      expect(restored.revision).toBeGreaterThan(previewed.revision);
      expect(JSON.parse(restored.layout)).toEqual(JSON.parse(before.layout));
      const secondPreviewWrite = nextWorldWrite();
      await page.getByRole('button', { name: 'Preview modular layout', exact: true }).click();
      expect((await secondPreviewWrite).status()).toBe(200);
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      const secondUndoWrite = nextWorldWrite();
      await page.getByRole('button', { name: 'Undo', exact: true }).click();
      expect((await secondUndoWrite).status()).toBe(200);
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      await expect(
        page.getByRole('button', { name: 'Preview modular layout', exact: true })
      ).toBeVisible();
      await expect(page.getByRole('button', { name: 'Add floor', exact: true })).toHaveCount(0);
      const redoWrite = nextWorldWrite();
      await page.getByRole('button', { name: 'Redo', exact: true }).click();
      expect((await redoWrite).status()).toBe(200);
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      const saved = await office<WorldSnapshot>(['layout', 'show']);
      expect(saved.layout.map.version).toBe(4);
      if (saved.layout.map.version !== 4) throw new Error('Expected central modules');
      expect(
        saved.layout.map.modules
          .map((module) => module.area)
          .sort((a, b) => a.id.localeCompare(b.id))
      ).toEqual([...source.map.areas].sort((a, b) => a.id.localeCompare(b.id)));
      expect(saved.layout.objects).toEqual(
        source.objects.map((object) => ({
          ...object,
          placement: {
            ...object.placement,
            y: object.id.endsWith('000000000090') ? 100 : object.placement.y,
          },
        }))
      );
      await page.getByRole('button', { name: 'Fit office', exact: true }).click();
      await page.screenshot({ path: info.outputPath('freeform-converted.png') });
      await office(['stop']);
      await page.goto('about:blank');
      await open();
      expect((await office<WorldSnapshot>(['layout', 'show'])).layout).toEqual(saved.layout);
      expect(JSON.parse(savedWorld(sandbox.database).layout)).toEqual(saved.layout);
    } finally {
      await office(['stop']);
    }
  });
});
