import { openKeyboardSelection, openOfficeDirectory } from './office-navigation.js';
import { expect, test } from '@playwright/test';
import { furnishedOfficeFixture } from './furnished-office-fixture.js';
import { fitWorldCoordinates } from './world-editor-gesture.js';

test('authors wall objects with pixel artwork, atomic coordinate input and auto-apply', async ({
  page,
}, info) => {
  const fixture = await furnishedOfficeFixture(page);
  const original = structuredClone(fixture.read().layout);
  const area = original.map.areas.find((area) => area.name === 'Studio')!;
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto(fixture.url);
  await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
  await openKeyboardSelection(page);
  await page.getByRole('combobox', { name: 'Area', exact: true }).selectOption(area.id);
  await expect(page.locator('.world-art-option')).toHaveCount(0);
  await page.getByRole('button', { name: 'Clear selection', exact: true }).click();
  const search = page.getByRole('searchbox', { name: 'Search objects' });
  await search.fill('no matching wall');
  await expect(page.getByText('No matching objects.', { exact: true })).toBeVisible();
  expect(fixture.writes).toEqual([]);
  await page.getByRole('button', { name: 'Clear search', exact: true }).click();
  await search.fill('observatory');
  await expect(page.getByRole('button', { name: 'Brass wall lamp', exact: true })).toHaveCount(0);
  const windowCard = page.getByRole('button', { name: 'Observatory window', exact: true });
  await windowCard.scrollIntoViewIfNeeded();
  await expect(windowCard.locator('svg').first()).toBeVisible();
  expect(await windowCard.locator('path').count()).toBeGreaterThan(0);
  const pictureBounds = (await windowCard.boundingBox())!;
  const inspectorBounds = (await page
    .getByRole('complementary', { name: 'Layout tools' })
    .boundingBox())!;
  expect(pictureBounds.y).toBeGreaterThanOrEqual(inspectorBounds.y);
  expect(pictureBounds.y + pictureBounds.height).toBeLessThanOrEqual(
    inspectorBounds.y + inspectorBounds.height
  );
  await page.screenshot({ path: info.outputPath('wall-library-picture-cards.png') });
  for (const name of [
    'Observatory window',
    'Brass wall lamp',
    'Orbit poster',
    'Crew sign',
    'Link plaque',
  ]) {
    if (name !== 'Observatory window')
      await page.getByRole('button', { name: 'Clear selection', exact: true }).click();
    if (name !== 'Observatory window') await expect(search).toHaveValue('');
    await page.getByRole('button', { name, exact: true }).click();
    await expect(
      page.getByRole('button', { name: 'Place on a suitable wall in this area' })
    ).toBeHidden();
    await expect(page.getByRole('region', { name: 'Selected area', exact: true })).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Delete', exact: true })).toBeVisible();
    await expect(page.locator('.world-object-preview > svg')).toBeVisible();
    await expect(page.getByRole('combobox', { name: 'Area', exact: true })).toHaveValue(area.id);
    if (name === 'Observatory window') {
      await page.screenshot({ path: info.outputPath('selected-object-inspector.png') });
      const selected = await page
        .getByRole('combobox', { name: 'Object', exact: true })
        .inputValue();
      await page.getByRole('button', { name: 'Delete', exact: true }).click();
      await expect(page.getByLabel('Area name', { exact: true })).toHaveValue('Studio');
      await page.getByRole('button', { name: 'Undo', exact: true }).click();
      await openKeyboardSelection(page);
      await page.getByRole('combobox', { name: 'Object', exact: true }).selectOption(selected);
      await page.getByRole('combobox', { name: 'Area', exact: true }).selectOption(area.id);
      await expect(page.getByLabel('Area name', { exact: true })).toHaveValue('Studio');
    }
    if (name === 'Crew sign') {
      await page.getByLabel('Display text', { exact: true }).fill('CREW');
      await page.getByRole('button', { name: 'Apply appearance' }).click();
    }
  }
  await page.getByText('Precise placement', { exact: true }).click();
  const x = page.getByLabel('X', { exact: true });
  await x.fill('');
  await x.pressSequentially('-12');
  // No NaN or partial negative coordinate has entered the document/history.
  await expect(x).toHaveValue('-12');
  await page.getByRole('button', { name: 'Apply coordinates' }).click();
  await page.getByRole('button', { name: 'Undo', exact: true }).click();
  await expect(x).toHaveValue('29');
  await expect(page.getByRole('region', { name: 'Layout changes' }).getByRole('status')).toHaveText(
    'All changes applied'
  );
  expect(fixture.writes.length).toBeGreaterThan(0);
  const saved = fixture.read().layout;
  expect(saved.map).toEqual(original.map);
  expect(saved.objects.slice(0, original.objects.length)).toEqual(original.objects);
  const added = saved.objects.slice(original.objects.length);
  expect(added.map((object) => object.kind)).toEqual([
    'window',
    'wallLight',
    'decoration',
    'decoration',
    'decoration',
  ]);
  expect(added.map((object) => object.placement.x)).toEqual([0, 12, 15, 21, 29]);
  for (const object of added)
    expect(object.surface).toEqual({
      type: 'wall',
      axis: 'horizontal',
      face: 'positive',
      elevation: 3,
    });
  expect(added[3]!.placement.customization).toEqual({ text: 'CREW' });
  await page.getByRole('button', { name: 'Fit office' }).click();
  await page.screenshot({ path: info.outputPath('world-wall-collection.png') });
  await page.goto('about:blank');
  await page.goto(fixture.url);
  await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
  expect(fixture.read().layout).toEqual(saved);
  expect(fixture.unexpected).toEqual([]);
});

test('renders saved terrain and real furniture beneath floating HUD on desktop and narrow screens', async ({
  page,
}, info) => {
  const fixture = await furnishedOfficeFixture(page);
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto(fixture.url);
  await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
  await expect(page.getByRole('button', { name: 'Office menu' })).toHaveAttribute(
    'aria-expanded',
    'false'
  );
  const canvas = page.locator('.office-canvas canvas');
  const bounds = await canvas.boundingBox();
  expect(bounds).toMatchObject({ x: 0, y: 0, width: 1440, height: 1000 });
  await page.screenshot({ path: info.outputPath('world-desktop.png') });
  await openOfficeDirectory(page);
  await page.getByRole('button', { name: /Alice · Online/ }).click();
  await expect(page.getByRole('complementary', { name: 'Agent details' })).toBeVisible();
  const inspector = page.getByRole('complementary', { name: 'Agent inspector' });
  await expect(inspector.getByRole('heading', { name: 'Alice', exact: true })).toBeVisible();
  await expect(page.getByRole('region', { name: 'Layout changes' })).toBeHidden();
  await expect(page.getByRole('heading', { name: 'Furniture & devices' })).toBeHidden();
  const dock = (await inspector.boundingBox())!;
  expect(dock.x + dock.width).toBe(1420);
  expect(dock.y + dock.height).toBe(980);
  await page.getByRole('button', { name: 'Zoom in' }).click();
  expect(await inspector.boundingBox()).toEqual(dock);
  await expect(canvas).toHaveCount(1);
  expect(await canvas.boundingBox()).toEqual(bounds);
  const objectActions = page
    .getByRole('navigation', { name: 'Office objects' })
    .getByRole('button');
  await expect(page.getByText('No interactive objects in this area.')).toBeVisible();
  await expect(objectActions).toHaveCount(0);
  await page.getByText('Other areas · 1', { exact: true }).click();
  await expect(page.getByRole('region', { name: 'Tools in Lobby' })).toBeVisible();
  // Check readable action geometry, not merely visible elements: the former
  // global nav flex layout squeezed each label into a character-wide column.
  async function readableActions() {
    await expect(objectActions).toHaveCount(3);
    const boxes = await objectActions.evaluateAll((buttons) =>
      buttons.map((button) => {
        const rect = button.getBoundingClientRect();
        return { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
      })
    );
    for (const [index, box] of boxes.entries()) {
      expect(box.width).toBeGreaterThan(200);
      expect(box.height).toBeLessThanOrEqual(64);
      if (index > 0)
        expect(box.y).toBeGreaterThanOrEqual(boxes[index - 1]!.y + boxes[index - 1]!.height);
    }
  }
  await readableActions();
  await page.screenshot({ path: info.outputPath('world-agent-hud.png') });
  await page.setViewportSize({ width: 390, height: 844 });
  await readableActions();
  const narrow = (await inspector.boundingBox())!;
  expect(narrow.x).toBeGreaterThanOrEqual(0);
  expect(narrow.x + narrow.width).toBeLessThanOrEqual(390);
  expect(narrow.y + narrow.height).toBe(836);
  await page.screenshot({ path: info.outputPath('world-agent-hud-narrow.png') });
  // Synthetic history only; this scenario proves layout/draft retention, not native delivery.
  await page.route('**/api/v1/local/requests/list', (route) =>
    route.fulfill({ json: { items: [], nextBefore: null } })
  );
  await page.getByRole('button', { name: 'Message Alice', exact: true }).click();
  const message = inspector.getByRole('textbox', { name: 'Message', exact: true });
  await expect(
    inspector.getByText('Start a conversation with Alice.', { exact: true })
  ).toBeVisible();
  const emptyComposer = await inspector.locator('.conversation-compose').boundingBox();
  const emptyFeed = await inspector.getByRole('log').boundingBox();
  await message.fill('Keep this draft while I inspect the room.');
  await expect(inspector.getByRole('button', { name: 'Discard draft' })).toHaveCount(0);
  expect(await inspector.locator('.conversation-compose').boundingBox()).toEqual(emptyComposer);
  expect(await inspector.getByRole('log').boundingBox()).toEqual(emptyFeed);
  await expect(message).toBeInViewport();
  expect(await inspector.boundingBox()).toEqual(narrow);
  const composer = (await inspector.locator('.conversation-compose').boundingBox())!;
  expect(composer.y + composer.height).toBeCloseTo(narrow.y + narrow.height - 1, 0);
  // Hold the next real background history poll, not a synthetic loading flag.
  // Refresh must neither insert a toolbar row nor move the focused composer.
  let releaseRefresh!: () => void;
  const refreshGate = new Promise<void>((resolve) => {
    releaseRefresh = resolve;
  });
  await page.route('**/api/v1/local/requests/list', async (route) => {
    await refreshGate;
    await route.fulfill({ json: { items: [], nextBefore: null } });
  });
  const feedBounds = await inspector.getByRole('log').boundingBox();
  await page.waitForRequest('**/api/v1/local/requests/list');
  await expect(inspector.getByRole('button', { name: 'Refresh', exact: true })).toBeDisabled();
  expect(await inspector.boundingBox()).toEqual(narrow);
  expect(await inspector.locator('.conversation-compose').boundingBox()).toEqual(composer);
  expect(await inspector.getByRole('log').boundingBox()).toEqual(feedBounds);
  await expect(message).toBeFocused();
  await expect(message).toHaveValue('Keep this draft while I inspect the room.');
  await expect(inspector.getByText('Updating…', { exact: true })).toHaveCount(0);
  releaseRefresh();
  await expect(inspector.getByRole('button', { name: 'Refresh', exact: true })).toBeEnabled();
  expect(await inspector.locator('.conversation-compose').boundingBox()).toEqual(composer);
  await expect(message).toBeFocused();
  await page.screenshot({ path: info.outputPath('world-agent-chat-narrow.png') });
  await message.press('Escape');
  await expect(page.getByRole('heading', { name: 'Furniture & devices' })).toBeVisible();
  await openOfficeDirectory(page);
  await page.getByRole('button', { name: /Alice · Online/ }).click();
  await page.getByRole('button', { name: 'Message Alice', exact: true }).click();
  await expect(message).toHaveValue('Keep this draft while I inspect the room.');
  await page.getByRole('button', { name: 'Close agent conversation' }).click();
  await page.getByRole('button', { name: 'Fit office' }).click();
  await page.screenshot({ path: info.outputPath('world-narrow.png') });
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
  expect(fixture.writes).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('keyboard selection and precise placement share auto-apply history and persistence', async ({
  page,
}, info) => {
  const fixture = await furnishedOfficeFixture(page);
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto(fixture.url);
  await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
  await openKeyboardSelection(page);
  const original = structuredClone(fixture.read().layout);
  await page
    .getByRole('combobox', { name: 'Object', exact: true })
    .selectOption(original.objects[0]!.id);
  await page.getByText('Precise placement', { exact: true }).click();
  const x = page.getByLabel('X', { exact: true });
  await x.fill(String(original.objects[0]!.placement.x + 4));
  await page.getByRole('button', { name: 'Apply coordinates' }).click();
  await expect(page.getByRole('button', { name: 'Undo', exact: true })).toBeEnabled();
  await page.screenshot({ path: info.outputPath('world-object-draft.png') });
  await page.getByRole('button', { name: 'Undo', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Undo', exact: true })).toBeDisabled();
  await page.getByRole('button', { name: 'Redo', exact: true }).click();
  await expect(page.getByRole('region', { name: 'Layout changes' }).getByRole('status')).toHaveText(
    'All changes applied'
  );
  expect(fixture.read().layout.map).toEqual(original.map);
  expect(fixture.read().layout.objects[0]!.placement).not.toEqual(original.objects[0]!.placement);
  expect(fixture.read().layout.objects.slice(1)).toEqual(original.objects.slice(1));
  await page.reload();
  await expect(page.getByRole('alert')).toContainText('reopen its URL');
  await page.goto('about:blank');
  await page.goto(fixture.url);
  await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
  await expect(page.locator('.office-canvas canvas')).toHaveCount(1);
  expect(fixture.unexpected).toEqual([]);
});

test('pointer cancellation does not move objects and property changes auto-apply', async ({
  page,
}) => {
  const fixture = await furnishedOfficeFixture(page);
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto(fixture.url);
  await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
  await openKeyboardSelection(page);
  const original = structuredClone(fixture.read().layout);
  const board = original.objects.find((object) => object.extension?.binding.kind === 'whiteboard')!;
  await page.getByRole('combobox', { name: 'Object', exact: true }).selectOption(board.id);
  const point = await fitWorldCoordinates(page, { x: -4, y: -20, width: 80, height: 75.5 });
  const start = point(23, 29.125);
  await page.mouse.move(start.x, start.y);
  await page.mouse.down();
  const canvas = page.locator('.office-canvas canvas');
  await expect.poll(() => canvas.evaluate((element) => element.hasPointerCapture(1))).toBe(true);
  await page.mouse.move(start.x + 40, start.y, { steps: 4 });
  await page
    .locator('.office-canvas canvas')
    .dispatchEvent('pointercancel', { pointerId: 1, button: 0 });
  await page.mouse.up();
  expect(fixture.read().layout).toEqual(original);
  await page
    .getByRole('combobox', { name: 'Area', exact: true })
    .selectOption(original.map.primaryLobbyId);
  await page.getByLabel('Area name', { exact: true }).fill('Shared lobby');
  await expect(page.getByRole('region', { name: 'Layout changes' }).getByRole('status')).toHaveText(
    'All changes applied'
  );
  await page.reload();
  await expect(page.getByRole('alert')).toContainText('reopen its URL');
  await page.goto('about:blank');
  await page.goto(fixture.url);
  await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
  await openKeyboardSelection(page);
  expect(fixture.read().layout.map.areas[0]!.name).toBe('Shared lobby');
  expect(fixture.writes.length).toBeGreaterThan(0);
  expect(fixture.unexpected).toEqual([]);
});

test('mounting a whiteboard on a wall retains its resource and opens it through the projected canvas hit area', async ({
  page,
}, info) => {
  const fixture = await furnishedOfficeFixture(page);
  const board = fixture
    .read()
    .layout.objects.find((object) => object.extension?.binding.kind === 'whiteboard')!;
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto(fixture.url);
  await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
  await openKeyboardSelection(page);
  await page.getByRole('combobox', { name: 'Object', exact: true }).selectOption(board.id);
  await page.getByText('Precise placement', { exact: true }).click();
  await page.getByRole('combobox', { name: 'Placement surface', exact: true }).selectOption('wall');
  await page.getByLabel('Y', { exact: true }).fill('0');
  await page.getByLabel('Elevation', { exact: true }).fill('2');
  await page.getByRole('button', { name: 'Apply coordinates' }).click();
  expect(fixture.reads.filter((path) => path.includes('/whiteboards/'))).toEqual([]);
  await expect(page.getByRole('region', { name: 'Layout changes' }).getByRole('status')).toHaveText(
    'All changes applied'
  );
  const mounted = fixture.read().layout.objects.find((object) => object.id === board.id)!;
  expect(mounted.extension).toEqual(board.extension);
  expect(mounted.placement).toEqual({ ...board.placement, y: 0 });
  expect(mounted.surface).toEqual({
    type: 'wall',
    axis: 'horizontal',
    face: 'positive',
    elevation: 2,
  });
  // Independent fixture dimensions: 72x76, floor depth 5/8, wall rise 16,
  // and Fit margins yield [-4,-20,80,75.5].
  // The whiteboard occupies [17,-10,12,8] after mounting; its old floor center is
  // [23,26.625]. Click the raised body, not an accessible-button or callback shortcut.
  const point = await fitWorldCoordinates(page, { x: -4, y: -20, width: 80, height: 75.5 });
  // Pan below the floating toolbar, then drag from inside the mounted body.
  // The grab offset must not become another elevation/edge offset.
  await page.mouse.move(850, 550);
  await page.mouse.down({ button: 'middle' });
  await page.mouse.move(850, 730);
  await page.mouse.up({ button: 'middle' });
  const start = point(23, -6);
  const end = point(27, -6);
  await page.mouse.move(start.x, start.y + 180);
  await page.mouse.down();
  await page.mouse.move(end.x, end.y + 180, { steps: 5 });
  await page.mouse.up();
  await expect(page.getByLabel('X', { exact: true })).toHaveValue('21');
  await expect(page.getByLabel('Y', { exact: true })).toHaveValue('0');
  await expect(page.getByLabel('Elevation', { exact: true })).toHaveValue('2');
  await page.getByRole('button', { name: 'Undo', exact: true }).click();
  await expect(page.getByLabel('X', { exact: true })).toHaveValue('17');
  await expect(page.getByRole('button', { name: 'Undo', exact: true })).toBeEnabled();
  await page.getByRole('button', { name: 'Redo', exact: true }).click();
  await expect(page.getByRole('region', { name: 'Layout changes' }).getByRole('status')).toHaveText(
    'All changes applied'
  );
  await page.getByRole('button', { name: 'Fit office' }).click();
  expect(fixture.read().layout.objects.find((object) => object.id === board.id)).toEqual({
    ...mounted,
    placement: { ...mounted.placement, x: 21 },
  });
  await page.screenshot({ path: info.outputPath('world-wall-whiteboard.png') });
  const oldFloor = point(23, 26.625);
  await page.mouse.click(oldFloor.x, oldFloor.y);
  await expect(page.getByRole('dialog', { name: 'Whiteboard', exact: true })).not.toBeVisible();
  await page.mouse.move(end.x, end.y);
  await expect(page.locator('.office-canvas canvas')).toHaveCSS('cursor', 'grab');
  const action = point(27, -12);
  await page.mouse.move(action.x, action.y);
  await expect(page.locator('.office-canvas canvas')).toHaveCSS('cursor', 'pointer');
  await page.mouse.click(action.x, action.y);
  const panel = page.getByRole('dialog', { name: 'Whiteboard', exact: true });
  await expect(panel).toBeVisible();
  await expect(panel.getByLabel('Whiteboard drawing surface')).toBeVisible();
  // The Vite development entry runs StrictMode's mount/dispose/remount probe.
  // It may start an aborted GET; prove exactly one completed read, no other
  // resource, and cancellation of every additional request rather than accepting
  // an arbitrary duplicate-read count.
  const resourceReads = fixture.reads.filter((path) => path.includes('/whiteboards/'));
  expect([...new Set(resourceReads)]).toEqual(['/api/v1/local/whiteboards/lobby']);
  await expect
    .poll(() => fixture.completedReads.filter((path) => path.includes('/whiteboards/')))
    .toEqual(['/api/v1/local/whiteboards/lobby']);
  const cancelled = fixture.failedReads.filter((request) => request.path.includes('/whiteboards/'));
  expect(cancelled).toHaveLength(resourceReads.length - 1);
  for (const request of cancelled) expect(request.error).toBe('net::ERR_ABORTED');
  await panel.getByRole('button', { name: 'Close whiteboard' }).click();
  expect(fixture.writes.length).toBeGreaterThanOrEqual(2);
  expect(fixture.unexpected).toEqual([]);
});

test('web links stay inert through authoring, save and review until an explicit no-opener click', async ({
  page,
  context,
}, info) => {
  const fixture = await furnishedOfficeFixture(page);
  const object = fixture.read().layout.objects.find((object) => !object.extension)!;
  const destinations: string[] = [];
  await context.route('https://example.com/**', async (route) => {
    destinations.push(route.request().url());
    await route.fulfill({ contentType: 'text/html', body: '<h1>Isolated web destination</h1>' });
  });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto(fixture.url);
  await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
  await openKeyboardSelection(page);
  await page.getByRole('combobox', { name: 'Object', exact: true }).selectOption(object.id);
  await expect(page.getByRole('form', { name: 'Web link' })).toBeHidden();
  await page.getByText('Object action', { exact: true }).click();
  const form = page.getByRole('form', { name: 'Web link' });
  await form.getByLabel('Web destination').fill('https://user:secret@example.com/docs');
  await form.getByRole('button', { name: 'Attach link to object' }).click();
  await expect(form.getByRole('alert')).toContainText('credentials');
  expect(fixture.writes).toEqual([]);
  await form.getByLabel('Web destination').fill('https://example.com/tmt-guide');
  await form.getByRole('button', { name: 'Attach link to object' }).click();
  await expect(page.getByRole('region', { name: 'Layout changes' }).getByRole('status')).toHaveText(
    'All changes applied'
  );
  expect(destinations).toEqual([]);
  expect(context.pages()).toHaveLength(1);
  expect(fixture.read().layout.objects.find((item) => item.id === object.id)).toEqual({
    ...object,
    extension: {
      definition: 'tmt-link',
      binding: { kind: 'external-link', url: 'https://example.com/tmt-guide' },
    },
  });
  await openOfficeDirectory(page);
  await page.getByRole('button', { name: /Alice · Online/ }).click();
  await page.getByText('Other areas · 1', { exact: true }).click();
  await page.getByRole('button', { name: 'Review web destination' }).click();
  const panel = page.getByRole('dialog', { name: 'Web destination', exact: true });
  await expect(panel.getByText('https://example.com', { exact: true })).toBeVisible();
  expect(destinations).toEqual([]);
  await page.screenshot({ path: info.outputPath('world-link-review.png') });
  await panel.getByRole('button', { name: 'Close web destination' }).click();
  expect(context.pages()).toHaveLength(1);
  await page.getByRole('button', { name: 'Review web destination' }).click();
  const popupPromise = page.waitForEvent('popup');
  await panel.getByRole('link', { name: 'Open website ↗' }).click();
  const popup = await popupPromise;
  await expect(popup.getByRole('heading', { name: 'Isolated web destination' })).toBeVisible();
  expect(await popup.evaluate(() => window.opener)).toBeNull();
  expect(destinations).toEqual(['https://example.com/tmt-guide']);
  await popup.close();
  expect(fixture.writes).toHaveLength(1);
  expect(fixture.unexpected).toEqual([]);
});
