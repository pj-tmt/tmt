import { expect, test } from '@playwright/test';
import robots from '../../../../contracts/modular-robots-v2.tmtavatar.json' with { type: 'json' };
import central from '../../../../contracts/modules-central-grid-vectors.json' with { type: 'json' };
import { decodeModuleMap } from '../src/world-map/module-contract.js';
import { furnishedOfficeFixture } from './furnished-office-fixture.js';
import { captureWorldScene, installDrawObserver } from './scene-observation.js';

for (const layout of ['retained', 'modular'] as const) {
  test(`${layout} online actors use each admitted robot and restore the default when its pack is absent`, async ({
    page,
  }, info) => {
    await installDrawObserver(page);
    const fixture = await furnishedOfficeFixture(page);
    const world = structuredClone(fixture.read());
    if (layout === 'modular') {
      const map = decodeModuleMap({
        ...central.map,
        modules: central.map.modules.slice(0, 5).map((module, index) => ({
          ...module,
          area:
            index === 1 || index === 2
              ? {
                  ...module.area,
                  binding: {
                    type: 'personal',
                    identityId: fixture.profiles[index - 1]!.identityId,
                  },
                }
              : module.area,
        })),
      });
      // An empty modular shell isolates walls, circulation and actor visibility.
      // It is not the furnished native default or a legacy-data migration.
      await page.route('**/api/v1/local/world', (route) =>
        route.fulfill({
          json: { ...world, layout: { ...world.layout, map, objects: [] } },
        })
      );
    }
    const digest = 'sha256:e0877ba218cfedb4fe1f480dc3a2da7bb4a48ea12161312f9329292bc7842ea9';
    let installed = true;
    // Native admission and persistence are covered by native-local-modular-robots.
    // This fixture controls only HTTP input; scene projection and GPU rendering are real.
    await page.route('**/api/v1/local/avatar-catalog', (route) =>
      route.fulfill({
        json: {
          catalogRevision: installed ? 1 : 2,
          packs: installed ? [{ digest, pack: robots }] : [],
        },
      })
    );
    await page.setViewportSize({ width: 1536, height: 1024 });
    const enter = async () => {
      await page.goto('about:blank');
      await page.goto(fixture.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
    };
    await enter();
    const baseline = await captureWorldScene(page, info, 'default-robots.png');
    const frames: Buffer[] = [];
    for (const avatar of robots.avatars) {
      for (const profile of fixture.profiles) profile.profile.avatarRef = `${digest}/${avatar.key}`;
      await enter();
      const frame = await captureWorldScene(page, info, `${avatar.key}-world.png`);
      expect(frame.equals(baseline)).toBe(false);
      for (const previous of frames) expect(frame.equals(previous)).toBe(false);
      frames.push(frame);
    }
    installed = false;
    await enter();
    expect(await captureWorldScene(page, info, 'unavailable-robots.png')).toEqual(baseline);
    expect(
      fixture.profiles.every((profile) => profile.profile.avatarRef === `${digest}/iris`)
    ).toBe(true);
    expect(fixture.read()).toEqual(world);
    expect(fixture.writes).toEqual([]);
    expect(fixture.profileWrites).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
  });
}
