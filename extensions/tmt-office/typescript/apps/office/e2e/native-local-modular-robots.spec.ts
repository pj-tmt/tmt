import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import { openAgentDetails } from './office-navigation.js';
import type { ProfileProjection } from '../src/profiles/profile-contract.js';

test('bundled modular robots need no installation and preserve explicit selections after restart', async ({
  page,
}, info) => {
  await withSandbox(async (sandbox) => {
    const prefix = await installNativeOffice(sandbox);
    const cli = async (args: string[]) => {
      const result = await runCli(sandbox, [...args, '--json']);
      expect(result.status, result.stdout + result.stderr).toBe(0);
      return JSON.parse(result.stdout);
    };
    const office = (args: string[]) => cli(['office', '--prefix', prefix, ...args]);
    const names = ['Moss', 'Azure', 'Ember', 'Iris'];
    const identityIds: string[] = [];
    for (const name of names) {
      const created = await cli(['identity', 'create', name]);
      identityIds.push(created.identity.id);
    }
    const file = path.join(sandbox.root, 'robots.tmtavatar.json');
    const bytes = readFileSync('../../../../../contracts/office/modular-robots-v2.tmtavatar.json');
    writeFileSync(file, bytes);
    const validated = await office(['avatar', 'validate', '--file', file]);
    expect(validated).toMatchObject({ formatVersion: 2, cellCount: 6144 });
    expect(validated.avatars.map((avatar: { key: string }) => avatar.key)).toEqual(
      names.map((name) => name.toLowerCase())
    );
    const digest = 'sha256:e0877ba218cfedb4fe1f480dc3a2da7bb4a48ea12161312f9329292bc7842ea9';
    expect(await office(['avatar', 'show', '--local', digest])).toMatchObject({
      digest,
      catalogRevision: 0,
      builtin: true,
      installedAtMs: null,
    });
    const world = await office(['layout', 'show']);
    let started = await office(['start', '--port', String(await unusedLoopbackPort())]);
    try {
      await page.setViewportSize({ width: 1536, height: 1024 });
      await page.goto(started.url);
      for (const name of names) {
        await openAgentDetails(page, name);
        await page.getByRole('button', { name: 'Appearance', exact: true }).click();
        await page.getByLabel('Avatar art').selectOption(`${digest}/${name.toLowerCase()}`);
        await expect(
          page.locator('.profile-preview svg[shape-rendering="crispEdges"]')
        ).toHaveAttribute('viewBox', '0 0 32 48');
        await page.getByRole('button', { name: 'Save appearance', exact: true }).click();
        await expect(page.locator('.profile-preview')).toContainText('Saved · revision 1');
        await page.locator('.profile-preview').scrollIntoViewIfNeeded();
        await expect(page.locator('.profile-editor')).toHaveCSS('color', 'rgb(243, 234, 215)');
        await expect(page.locator('.profile-editor')).toHaveCSS(
          'background-color',
          'rgb(12, 38, 54)'
        );
        await page.screenshot({ path: info.outputPath(`robot-${name.toLowerCase()}.png`) });
        await page.locator('.profile-preview').screenshot({
          path: info.outputPath(`robot-${name.toLowerCase()}-preview.png`),
        });
      }
      expect(await office(['layout', 'show'])).toEqual(world);
      expect(
        await office(['avatar', 'install', '--local', '--file', file, '--if-revision', '0'])
      ).toMatchObject({
        digest,
        catalogRevision: 0,
        builtin: true,
        changed: false,
        installedAtMs: null,
      });
      await page.goto('about:blank');
      await office(['stop']);
      started = await office(['start', '--port', String(await unusedLoopbackPort())]);
      await page.goto(started.url);
      for (const name of names) {
        expect(await office(['profile', 'show', '--local', '--identity', name])).toMatchObject({
          revision: 1,
          profile: { avatarRef: `${digest}/${name.toLowerCase()}` },
        });
      }
      await page.screenshot({ path: info.outputPath('modular-robots-office.png') });
      // Explicitly populate the four existing offices for visual review. Creating
      // identities and selecting artwork above must not have assigned any room.
      const assigned = structuredClone(world.layout);
      let resident = 0;
      for (const module of assigned.map.modules) {
        if (module.slot.type !== 'office') continue;
        module.area.binding.identityId = identityIds[resident++];
      }
      expect(resident).toBe(4);
      const layoutFile = path.join(sandbox.root, 'resident-offices.json');
      writeFileSync(layoutFile, JSON.stringify(assigned));
      await office([
        'layout',
        'apply',
        '--file',
        layoutFile,
        '--if-revision',
        String(world.revision),
        '--legacy-basis',
        world.legacyBasis,
      ]);
      expect((await office(['layout', 'show'])).layout).toEqual(assigned);
      const refreshedWorld = page.waitForResponse(
        (response) => new URL(response.url()).pathname === '/api/v1/local/world'
      );
      await page.getByRole('button', { name: 'Refresh office', exact: true }).click();
      expect((await refreshedWorld).status()).toBe(200);
      await expect(page.getByRole('button', { name: 'Refresh office', exact: true })).toBeEnabled();
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      await page.getByRole('button', { name: 'Fit office', exact: true }).click();
      // These identities have no live endpoint: ownership must not fabricate
      // online presence. Active actor pixels have a separate controlled GPU test.
      await expect(page.getByText('5 areas · 0 online', { exact: true })).toBeVisible();
      await page.screenshot({ path: info.outputPath('modular-robots-assigned-offices.png') });
      const offlineCanvas = await page.locator('.office-canvas canvas').screenshot();
      const savedAssignment = await office(['layout', 'show']);
      // Visual-only input: native layout, artwork and ownership stay real, but
      // live endpoint verification is not the subject of this rendered fixture.
      await page.route('**/api/v1/local/profiles', async (route) => {
        const response = await route.fetch();
        expect(response.status()).toBe(200);
        const profiles: ProfileProjection[] = await response.json();
        expect(profiles.map((profile) => profile.presence)).toEqual(names.map(() => 'offline'));
        await route.fulfill({
          response,
          json: profiles.map((profile) => ({ ...profile, presence: 'active' })),
        });
      });
      await page.goto('about:blank');
      await page.goto(started.url);
      await expect(page.getByText('5 areas · 4 online', { exact: true })).toBeVisible();
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      await page.getByRole('button', { name: 'Fit office', exact: true }).click();
      const populatedCanvas = await page.locator('.office-canvas canvas').screenshot();
      expect(populatedCanvas.equals(offlineCanvas)).toBe(false);
      await page.screenshot({ path: info.outputPath('modular-robots-simulated-active.png') });
      expect(await office(['layout', 'show'])).toEqual(savedAssignment);
      await page.unroute('**/api/v1/local/profiles');
      expect(readFileSync(file)).toEqual(bytes);
    } finally {
      await page.goto('about:blank');
      await office(['stop']);
    }
  });
});
