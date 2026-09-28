import { expect, test } from '@playwright/test';
import { furnishedOfficeFixture } from './furnished-office-fixture.js';
import { fitWorldCoordinates } from './world-editor-gesture.js';

test('click selects, direct drag applies once, cancellation never moves, and Undo persists', async ({
  page,
}, info) => {
  const fixture = await furnishedOfficeFixture(page);
  const original = structuredClone(fixture.read().layout);
  const board = original.objects.find((object) => object.extension?.binding.kind === 'whiteboard')!;
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto(fixture.url);
  await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
  await expect(page.getByRole('button', { name: 'Edit layout', exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Save layout', exact: true })).toHaveCount(0);
  // Independent retained fixture: 72×76 floor, depth 5/8, rise 16.
  // Floor whiteboard [17,45,12,8] has projected body [17,25.125,12,8].
  const point = await fitWorldCoordinates(page, { x: -4, y: -20, width: 80, height: 75.5 });
  const start = point(23, 29.125),
    end = point(27, 29.125);
  await page.mouse.click(start.x, start.y);
  await expect(page.getByRole('heading', { name: /^Selected object:/ })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Open whiteboard', exact: true })).toBeVisible();
  expect(fixture.writes).toHaveLength(0);
  // A rejected drop is presentation only: no persistence or history entry.
  await page.mouse.move(start.x, start.y);
  await page.mouse.down();
  const outside = point(-1, 29.125);
  await page.mouse.move(outside.x, outside.y, { steps: 5 });
  await expect(page.locator('.office-canvas canvas')).toHaveAttribute(
    'data-drop-validity',
    'invalid'
  );
  await page.screenshot({ path: info.outputPath('invalid-drop-preview.png') });
  await page.mouse.up();
  await expect(page.getByRole('button', { name: 'Undo', exact: true })).toBeDisabled();
  expect(fixture.read().layout).toEqual(original);
  expect(fixture.writes).toHaveLength(0);
  await page.mouse.move(start.x, start.y);
  await page.mouse.down();
  await page.mouse.move(end.x, end.y, { steps: 5 });
  await expect(page.locator('.office-canvas canvas')).toHaveAttribute(
    'data-drop-validity',
    'valid'
  );
  expect(fixture.writes).toHaveLength(0);
  await page.mouse.up();
  await expect.poll(() => fixture.writes.length).toBe(1);
  expect(fixture.read().layout.objects.find((object) => object.id === board.id)!.placement.x).toBe(
    21
  );
  expect(fixture.read().layout.map).toEqual(original.map);
  await page.screenshot({ path: info.outputPath('direct-object-properties.png') });
  await page.getByRole('button', { name: 'Undo', exact: true }).click();
  await expect.poll(() => fixture.writes.length).toBe(2);
  expect(fixture.read().layout).toEqual(original);
  await page.keyboard.press('ControlOrMeta+Shift+z');
  await expect.poll(() => fixture.writes.length).toBe(3);
  expect(fixture.read().layout.objects.find((object) => object.id === board.id)!.placement.x).toBe(
    21
  );
  await page.keyboard.press('ControlOrMeta+z');
  await expect.poll(() => fixture.writes.length).toBe(4);
  expect(fixture.read().layout).toEqual(original);
  await page.mouse.move(start.x, start.y);
  await page.mouse.down();
  await page.mouse.move(end.x, end.y, { steps: 4 });
  await page.locator('.office-canvas canvas').dispatchEvent('pointercancel', { pointerId: 1 });
  await page.mouse.up();
  await expect(page.getByRole('status')).toHaveText('All changes applied');
  expect(fixture.read().layout).toEqual(original);
  await page.mouse.move(start.x, start.y);
  await page.mouse.down();
  await page.mouse.move(end.x, end.y, { steps: 4 });
  await page.keyboard.press('Escape');
  await page.mouse.up();
  await expect(page.getByRole('button', { name: 'Redo', exact: true })).toBeEnabled();
  expect(fixture.writes).toHaveLength(4);
  expect(fixture.read().layout).toEqual(original);
  await expect(page.getByRole('heading', { name: 'Furniture & devices' })).toBeVisible();
  // Bare floor chooses the room, not a tool or object dropdown.
  const floor = point(1, 44);
  await page.mouse.click(floor.x, floor.y);
  await expect(page.getByLabel('Area name', { exact: true })).toHaveValue('Lobby');
  await page.getByLabel('Area name', { exact: true }).fill('Shared lobby');
  await expect.poll(() => fixture.read().layout.map.areas[0]!.name).toBe('Shared lobby');
  await page.screenshot({ path: info.outputPath('direct-room-properties.png') });
  expect(fixture.unexpected).toEqual([]);
});
