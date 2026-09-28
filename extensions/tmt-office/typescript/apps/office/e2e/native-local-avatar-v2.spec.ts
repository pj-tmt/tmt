import Database from '../../../../../../typescript/test/support/sqlite-oracle.js';
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import { openAgentDetails } from './office-navigation.js';

test('avatar v2 survives preview, selection, missing-art fallback and exact reinstall without rewriting the profile', async ({
  page,
}, testInfo) => {
  await withSandbox(async (sandbox) => {
    const prefix = await installNativeOffice(sandbox);
    const created = await runCli(sandbox, ['identity', 'create', 'Artist', '--json']);
    expect(created.status, created.stdout).toBe(0);
    const identityId = JSON.parse(created.stdout).identity.id as string;
    const bytes = readFileSync(
      '../../../../../contracts/office/avatar-pack-v2-sample.tmtavatar.json'
    );
    const file = path.join(sandbox.root, 'artist.tmtavatar.json');
    writeFileSync(file, bytes);
    const office = async (args: string[]) => {
      const result = await runCli(sandbox, ['office', '--prefix', prefix, ...args, '--json']);
      expect(result.status, result.stdout).toBe(0);
      return JSON.parse(result.stdout);
    };
    const stored = () => {
      const db = new Database(sandbox.database, { readonly: true });
      try {
        return {
          profile: db
            .prepare('SELECT * FROM office_local_profiles WHERE identity_id = ?')
            .get(identityId),
          packs: db.prepare('SELECT digest, bytes FROM office_avatar_packs ORDER BY digest').all(),
        };
      } finally {
        db.close();
      }
    };
    const before = stored();
    const bundled = path.join(sandbox.root, 'workshop.tmtavatar.json');
    writeFileSync(
      bundled,
      readFileSync('../../../../../contracts/office/workshop-robot-v2.tmtavatar.json')
    );
    expect(await office(['avatar', 'validate', '--file', bundled])).toMatchObject({
      formatVersion: 2,
      cellCount: 1536,
      avatars: [{ key: 'workshop-robot', raster: { width: 32, height: 48 } }],
    });
    expect(stored()).toEqual(before);
    const validated = await office(['avatar', 'validate', '--file', file]);
    const digest = 'sha256:371a80e4b0e24cd0d8f5b948e506074ab3be536529c8220db3819e0a34d76831';
    expect(validated).toMatchObject({
      digest,
      formatVersion: 2,
      cellCount: 1536,
      avatars: [{ key: 'signal-bot', raster: { width: 32, height: 48 } }],
    });
    expect(stored()).toEqual(before);
    const installed = await office([
      'avatar',
      'install',
      '--local',
      '--file',
      file,
      '--if-revision',
      '0',
    ]);
    expect(installed).toMatchObject({ digest, catalogRevision: 1, changed: true });
    expect(stored().packs).toEqual([{ digest, bytes }]);
    expect(await office(['avatar', 'show', '--local', digest])).toMatchObject({
      formatVersion: 2,
      cellCount: 1536,
    });
    let started = await office(['start', '--port', String(await unusedLoopbackPort())]);
    const enter = async () => {
      // The token fragment is consumed. Re-adding it to the same world URL is
      // same-document navigation, not a fresh catalog read or profile mount.
      await page.goto('about:blank');
      await page.goto(started.url);
      await openAgentDetails(page, 'Artist');
      await page.getByRole('button', { name: 'Appearance', exact: true }).click();
      await expect(page.getByLabel('Avatar art')).toBeVisible();
      // Document width alone misses controls clipped inside a scrollable HUD.
      const panel = page.getByRole('complementary', { name: 'Agent details' });
      await expect(panel).toBeVisible();
      expect(await panel.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(
        true
      );
      const bounds = (await panel.boundingBox())!;
      for (const field of await panel.locator('input, select, textarea, button').all()) {
        const rect = await field.boundingBox();
        if (!rect) continue;
        expect(rect.x).toBeGreaterThanOrEqual(bounds.x);
        expect(rect.x + rect.width).toBeLessThanOrEqual(bounds.x + bounds.width);
      }
    };
    const raster = page.locator('.profile-preview svg[shape-rendering="crispEdges"]');
    try {
      await page.setViewportSize({ width: 1280, height: 900 });
      const beforePreview = stored();
      const preview = await office(['avatar', 'preview', '--file', file]);
      await page.goto(preview.url);
      await expect(page.getByRole('heading', { name: 'Detailed signal robot' })).toBeVisible();
      await expect(page.locator('svg[shape-rendering="crispEdges"]')).toHaveAttribute(
        'viewBox',
        '0 0 32 48'
      );
      await expect(page.locator('path[fill="#b3f6d9ff"]')).toHaveCount(1);
      expect(stored()).toEqual(beforePreview);
      await page.screenshot({ path: testInfo.outputPath('avatar-v2-native-preview.png') });
      await enter();
      await page.getByLabel('Avatar art').selectOption(`${digest}/signal-bot`);
      await expect(raster).toHaveAttribute('viewBox', '0 0 32 48');
      const customPixels = await raster.innerHTML();
      await page.getByRole('button', { name: 'Save appearance' }).click();
      await expect(page.locator('.profile-preview')).toContainText('Saved · revision 1');
      const saved = stored().profile;
      expect(saved).toBeDefined();
      expect(await office(['profile', 'show', '--local', '--identity', 'Artist'])).toMatchObject({
        revision: 1,
        profile: { avatarRef: `${digest}/signal-bot` },
      });
      await page.screenshot({ path: testInfo.outputPath('avatar-v2-native-selected.png') });
      await office(['avatar', 'remove', '--local', digest, '--if-revision', '1']);
      await enter();
      await expect(
        page.getByText('Avatar · unavailable, showing saved default appearance')
      ).toBeVisible();
      await expect(raster).toHaveAttribute('viewBox', '0 0 32 48');
      expect(await raster.innerHTML()).not.toBe(customPixels);
      await page.screenshot({ path: testInfo.outputPath('avatar-v2-native-default-fallback.png') });
      await page.setViewportSize({ width: 390, height: 844 });
      await enter();
      await page.screenshot({ path: testInfo.outputPath('avatar-v2-native-default-narrow.png') });
      await page.setViewportSize({ width: 1280, height: 900 });
      expect(stored().profile).toEqual(saved);
      expect(stored().packs).toEqual([]);
      expect(
        await office(['avatar', 'install', '--local', '--file', file, '--if-revision', '2'])
      ).toMatchObject({ digest, catalogRevision: 3 });
      await office(['stop']);
      started = await office(['start', '--port', String(await unusedLoopbackPort())]);
      await enter();
      await expect(raster).toHaveAttribute('viewBox', '0 0 32 48');
      await expect(page.getByLabel('Avatar art')).toHaveValue(`${digest}/signal-bot`);
      expect(await raster.innerHTML()).toBe(customPixels);
      expect(stored()).toEqual({ profile: saved, packs: [{ digest, bytes }] });
      expect(readFileSync(file)).toEqual(bytes);
    } finally {
      await office(['stop']);
    }
  });
});
