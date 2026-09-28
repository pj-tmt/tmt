import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';
import { furnishedOfficeFixture } from '../furnished-office-fixture.js';
import { fitWorldCoordinates } from '../world-editor-gesture.js';
import { DIRECTIONAL_WORKSTATION_DIGEST } from '../../src/props/prop-contract.js';
import type { WorldDocument } from '../../src/world-map/world-contract.js';
import { worldGeometry } from '../../src/rendering/world-geometry.js';

const lobby = '10000000-0000-4000-8000-000000000001';
const office = '10000000-0000-4000-8000-000000000002';
const meeting = '10000000-0000-4000-8000-000000000003';
const layout: WorldDocument = {
  version: 1,
  map: {
    version: 8,
    primaryLobbyId: lobby,
    modules: [
      {
        area: { id: lobby, name: 'Lobby', binding: { type: 'lobby' } },
        slot: { type: 'lobby' },
        material: 'workshop',
      },
      {
        area: { id: office, name: 'Office', binding: { type: 'personal', identityId: null } },
        slot: { type: 'office', column: 0, row: -1 },
        material: 'workshop',
      },
      {
        area: {
          id: meeting,
          name: 'Planning',
          binding: { type: 'meeting', roomId: '40000000-0000-4000-8000-000000000001' },
        },
        slot: { type: 'office', column: 1, row: -1 },
        material: 'workshop',
      },
    ],
  },
  objects: [
    {
      id: '30000000-0000-4000-8000-000000000001',
      kind: 'decoration',
      surface: { type: 'floor', base: { x: 2, y: 8, width: 7, height: 3 } },
      placement: {
        prop: `${DIRECTIONAL_WORKSTATION_DIGEST}/workstation-bookcase`,
        footprint: { width: 11, height: 11 },
        x: 8,
        y: -8,
        rotation: 0,
      },
      extension: null,
    },
  ],
};

async function openScene(page: Page, withActor = false) {
  const fixture = await furnishedOfficeFixture(page);
  if (withActor)
    await page.route('**/api/v1/local/requests/list', (route) =>
      route.fulfill({ json: { items: [], nextBefore: null } })
    );
  const sceneLayout: WorldDocument =
    withActor && layout.map.version === 8
      ? {
          ...layout,
          map: {
            ...layout.map,
            modules: layout.map.modules.map((module) =>
              module.area.id === office
                ? {
                    ...module,
                    area: {
                      ...module.area,
                      binding: {
                        type: 'personal' as const,
                        identityId: fixture.profiles[0]!.identityId,
                      },
                    },
                  }
                : module
            ),
          },
        }
      : layout;
  const unexpectedWrites: string[] = [];
  // Reuse the existing local HTTP fixture; override only this scenario's world.
  // No mock save can masquerade as native admission or durability evidence.
  await page.route('**/api/v1/local/world', async (route) => {
    if (route.request().method() !== 'GET') {
      unexpectedWrites.push(route.request().method());
      await route.fulfill({ status: 400, json: { error: 'VISUAL_FIXTURE_READ_ONLY' } });
      return;
    }
    await route.fulfill({ json: { ...fixture.read(), layout: sceneLayout } });
  });
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.goto(fixture.url);
  await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
  await page.evaluate(() => document.fonts.ready);
  // Independent v8 fixture extent: floor (0,-56)..(120,91), plus shell framing.
  const point = await fitWorldCoordinates(page, { x: -4, y: -60, width: 128, height: 159 });
  return {
    point,
    sceneLayout,
    verify: () => {
      expect(errors).toEqual([]);
      expect(unexpectedWrites).toEqual([]);
      expect(fixture.writes).toEqual([]);
      expect(fixture.unexpected).toEqual([]);
    },
  };
}

test('selection accents identify area use and stay behind overlapping art and agent labels', async ({
  page,
}) => {
  const { point, sceneLayout, verify } = await openScene(page, true);
  for (const [name, x, y] of [
    ['Office', 30, -40],
    ['Planning', 100, -40],
    ['Lobby', 30, 30],
  ] as const) {
    const floor = point(x, y);
    await page.mouse.click(floor.x, floor.y);
    await expect(page.getByLabel('Area name', { exact: true })).toHaveValue(name);
    await expect(page).toHaveScreenshot(`selection-${name.toLowerCase()}.png`);
  }
  const geometry = worldGeometry(sceneLayout);
  const actor = geometry.projection.projectUpright(geometry.actorSlots(office)[0]!);
  const target = point(actor.x + actor.width / 2, actor.y + actor.height / 2);
  await page.mouse.click(target.x, target.y);
  await expect(page.getByRole('heading', { name: 'Alice', exact: true })).toBeVisible();
  await expect(page).toHaveScreenshot('selection-agent.png');
  await page.keyboard.press('Escape');
  await expect(page.getByRole('heading', { name: 'Furniture & devices' })).toBeVisible();
  verify();
});

test('accepted platforms, rim overhang and held rotation remain visually stable', async ({
  page,
}) => {
  const { point, verify } = await openScene(page);
  await page.keyboard.press('Escape');
  await expect(page).toHaveScreenshot('platforms.png');
  // Pick the upper artwork outside its physical base and above the platform rim.
  const upper = point(13, -6);
  await page.mouse.click(upper.x, upper.y);
  await expect(
    page.getByRole('heading', { name: 'Selected object: Workshop bookcase' })
  ).toBeVisible();
  await expect(page).toHaveScreenshot('bookcase-selected.png');
  const corner = point(19, 2.625);
  await page.mouse.move(corner.x, corner.y);
  await page.mouse.down();
  // Turn about the artwork center; keep the gesture held to expose stale-source art.
  const turn = point(8, 2.625);
  await page.mouse.move(turn.x, turn.y, { steps: 6 });
  await expect(page.locator('.office-canvas canvas')).toHaveAttribute(
    'data-drop-validity',
    'invalid'
  );
  await expect(page).toHaveScreenshot('bookcase-held-turn.png');
  await page.keyboard.press('Escape');
  await page.mouse.up();
  await page.mouse.click(upper.x, upper.y);
  await expect(page).toHaveScreenshot('bookcase-selected.png');
  await expect(page.getByRole('button', { name: 'Undo', exact: true })).toBeDisabled();
  verify();
});

test('area inspector and creation controls retain spacing at desktop and narrow widths', async ({
  page,
}) => {
  const { point, verify } = await openScene(page);
  const floor = point(30, -40);
  await page.mouse.click(floor.x, floor.y);
  const tools = page.getByRole('complementary', { name: 'Layout tools' });
  await expect(tools.getByLabel('Area name', { exact: true })).toHaveValue('Office');
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 1000 });
    const gaps = await tools.evaluate((element) => {
      const finishes = element.querySelector('.room-material-choices')!;
      const resident = finishes.nextElementSibling!;
      const removal = element.querySelector('.world-area-removal')!;
      return {
        resident: resident.getBoundingClientRect().top - finishes.getBoundingClientRect().bottom,
        removal: removal.getBoundingClientRect().top - resident.getBoundingClientRect().bottom,
      };
    });
    expect(gaps.resident).toBeGreaterThanOrEqual(16);
    expect(gaps.removal).toBeGreaterThanOrEqual(16);
    await expect(tools).toHaveScreenshot(`area-inspector-${width}.png`);
  }
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: 'Fit office', exact: true }).click();
  const ghost = point(-48, 10);
  await page.mouse.move(ghost.x, ghost.y);
  await page.mouse.click(ghost.x, ghost.y);
  const creation = page.getByRole('region', { name: 'New area', exact: true });
  await expect(creation).toBeVisible();
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 1000 });
    await expect(creation).toHaveScreenshot(`create-office-${width}.png`);
  }
  await creation.getByRole('button', { name: 'Meeting room', exact: true }).click();
  const meetingForm = creation.getByRole('region', { name: 'Create meeting space', exact: true });
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 1000 });
    await expect(
      meetingForm.getByRole('textbox', { name: 'Room name', exact: true })
    ).toBeVisible();
    const gap = await meetingForm.evaluate((element) => {
      const input = element.querySelector('input')!.getBoundingClientRect();
      const save = element.querySelector('button[type="submit"]')!.getBoundingClientRect();
      return save.top - input.bottom;
    });
    expect(gap).toBeGreaterThanOrEqual(16);
    await expect(creation).toHaveScreenshot(`create-meeting-${width}.png`);
  }
  await page.keyboard.press('Escape');
  verify();
});
