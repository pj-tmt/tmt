import { openOfficeDirectory } from './office-navigation.js';
import { expect, test } from '@playwright/test';
import type { Request } from '@playwright/test';
import { captureWorldScene, installDrawObserver, observeIdleScene } from './scene-observation.js';
import { furnishedOfficeFixture } from './furnished-office-fixture.js';

test('world HUD preserves the canvas and profile edits persist independently of layout', async ({
  page,
}, info) => {
  await installDrawObserver(page);
  const fixture = await furnishedOfficeFixture(page);
  const world = structuredClone(fixture.read());
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto(fixture.url);
  await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
  await observeIdleScene(page, info, 'world-idle');
  const canvas = page.locator('.office-canvas canvas');
  const node = await canvas.elementHandle();
  const bounds = await canvas.boundingBox();
  // Sample the furnished Lobby, outside the agent-anchored overlay. Comparing
  // pixels underneath the HUD would test occlusion, not world/camera stability.
  const strip = { x: 400, y: 650, width: 64, height: 64 };
  const pixels = await page.screenshot({ clip: strip });
  await openOfficeDirectory(page);
  const alice = page.getByRole('button', { name: /Alice · Online/ });
  await alice.focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('complementary', { name: 'Agent details' })).toBeVisible();
  const hud = (await page.getByRole('dialog', { name: 'Agent conversation' }).boundingBox())!;
  expect(
    hud.x >= strip.x + strip.width ||
      hud.x + hud.width <= strip.x ||
      hud.y >= strip.y + strip.height ||
      hud.y + hud.height <= strip.y
  ).toBe(true);
  expect(await canvas.boundingBox()).toEqual(bounds);
  expect(await page.screenshot({ clip: strip })).toEqual(pixels);
  expect(await node!.evaluate((element) => element.isConnected)).toBe(true);
  const catalogReads = fixture.reads.filter((path) => path.endsWith('/avatar-catalog')).length;
  await page.getByRole('button', { name: 'Appearance', exact: true }).click();
  await page.getByLabel('Shirt mark').fill('UX');
  await page.getByRole('button', { name: 'Save appearance' }).click();
  await expect(page.getByText('Saved · revision 2', { exact: true })).toBeVisible();
  expect(fixture.profiles[0]!.profile.appearance.shirtMark).toBe('UX');
  expect(fixture.profileWrites).toHaveLength(1);
  for (const style of fixture.profiles[0]!.catalog.hairStyles) {
    await page.getByLabel('Hair style').selectOption(style);
    const avatar = page.locator('.profile-preview .profile-avatar');
    await expect(avatar.locator('circle, image, script, img')).toHaveCount(0);
    // Accessible name/mark glyphs live outside the bounded indexed-pixel artwork.
    await expect(avatar.locator('svg[shape-rendering="crispEdges"] foreignObject')).toHaveCount(0);
    await expect(avatar.locator('svg[shape-rendering="crispEdges"]')).toHaveCount(1);
    await expect(avatar.locator('.avatar-name')).toHaveText('Alice');
  }
  expect(fixture.reads.filter((path) => path.endsWith('/avatar-catalog'))).toHaveLength(
    catalogReads
  );
  await page.setViewportSize({ width: 390, height: 844 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
  await expect(page.getByRole('complementary', { name: 'Office directory' })).toBeHidden();
  await page.getByLabel('Shirt mark').fill('NEW');
  const close = page.getByRole('button', { name: 'Close agent conversation' });
  const closeBounds = await close.boundingBox();
  expect(await page.locator('.office-map').evaluate((element) => element.scrollTop)).toBe(0);
  await page.getByRole('tabpanel', { name: 'Info' }).evaluate((body) => {
    body.scrollTop = body.scrollHeight;
  });
  expect(await page.locator('.office-map').evaluate((element) => element.scrollTop)).toBe(0);
  expect(await close.boundingBox()).toEqual(closeBounds);
  await expect(close).toBeInViewport();
  await page.screenshot({ path: info.outputPath('world-appearance-narrow.png') });
  await openOfficeDirectory(page);
  await expect(page.getByRole('complementary', { name: 'Agent details' })).toBeHidden();
  await expect(page.getByRole('complementary', { name: 'Office directory' })).toBeVisible();
  await page.getByRole('button', { name: /Alice · Online/ }).click();
  await expect(page.getByLabel('Shirt mark')).toHaveValue('NEW');
  await close.click();
  await expect(page.getByRole('button', { name: 'Office menu' })).toBeFocused();
  expect(fixture.profileWrites).toHaveLength(1);
  expect(fixture.writes).toEqual([]);
  expect(fixture.read()).toEqual(world);
  expect(fixture.unexpected).toEqual([]);
});

test('viewport changes preserve the world framing and a manually panned view without rebuilding the canvas', async ({
  page,
}, info) => {
  await installDrawObserver(page);
  const fixture = await furnishedOfficeFixture(page);
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto(fixture.url);
  await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
  const canvas = page.locator('.office-canvas canvas');
  const node = await canvas.elementHandle();
  await page.setViewportSize({ width: 390, height: 844 });
  await observeIdleScene(page, info, 'resized-portrait');
  const portrait = await captureWorldScene(page, info, 'resized-portrait.png');
  await page.getByRole('button', { name: 'Fit office' }).click();
  await observeIdleScene(page, info, 'fitted-portrait');
  expect(await captureWorldScene(page, info, 'fitted-portrait.png')).toEqual(portrait);
  await page.screenshot({ path: info.outputPath('world-resized-portrait.png') });

  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.getByRole('button', { name: 'Zoom in' }).click();
  await page.mouse.move(700, 500);
  await page.mouse.down({ button: 'middle' });
  await page.mouse.move(820, 570);
  await page.mouse.up({ button: 'middle' });
  await observeIdleScene(page, info, 'panned-desktop');
  const panned = await captureWorldScene(page, info, 'panned-desktop.png');
  await page.setViewportSize({ width: 390, height: 844 });
  await observeIdleScene(page, info, 'panned-portrait');
  await page.setViewportSize({ width: 1440, height: 1000 });
  await observeIdleScene(page, info, 'restored-desktop');
  expect(await captureWorldScene(page, info, 'restored-desktop.png')).toEqual(panned);
  expect(await node!.evaluate((element) => element.isConnected)).toBe(true);
  expect(fixture.writes).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('directory growth and empty presence do not rebuild terrain or animate an idle world', async ({
  page,
}, info) => {
  await installDrawObserver(page);
  const fixture = await furnishedOfficeFixture(page);
  const world = structuredClone(fixture.read());
  const people = Array.from({ length: 24 }, (_, index) => ({
    ...fixture.profiles[0]!,
    identityId: `20000000-0000-4000-8000-${String(index + 1).padStart(12, '0')}`,
    identityName: `Agent ${index + 1}`,
    presence: index % 4 !== 0 ? 'active' : 'offline',
  }));
  await page.route('**/api/v1/local/profiles', (route) => route.fulfill({ json: people }));
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto(fixture.url);
  await expect(page.getByRole('button', { name: 'Office menu' })).toBeVisible();
  await expect(
    page.locator('#office-navigation').getByText('Directory · 24', { exact: true })
  ).toHaveCount(1);
  await observeIdleScene(page, info, '24-agents-unchanged-terrain');
  const canvas = page.locator('.office-canvas canvas');
  const node = await canvas.elementHandle();
  await page.mouse.move(700, 650);
  await page.mouse.wheel(0, -200);
  await page.mouse.down();
  await page.mouse.move(780, 690);
  await page.mouse.up();
  await observeIdleScene(page, info, 'world-after-navigation');
  expect(await node!.evaluate((element) => element.isConnected)).toBe(true);
  expect(fixture.read()).toEqual(world);
  await page.route('**/api/v1/local/profiles', (route) => route.fulfill({ json: [] }));
  await page.getByRole('button', { name: 'Refresh office' }).click();
  await expect(page.getByRole('button', { name: 'Office menu' })).toBeVisible();
  await expect(
    page.locator('#office-navigation').getByText('Directory · 0', { exact: true })
  ).toHaveCount(1);
  await expect(canvas).toBeVisible();
  expect(fixture.read()).toEqual(world);
  expect(fixture.writes).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
  await page.screenshot({ path: info.outputPath('world-no-present-agents.png') });
});

test('missing local session gives recoverable CLI guidance', async ({ page }) => {
  await page.goto('http://127.0.0.1:4176/local');
  await expect(page.getByRole('alert')).toHaveText(
    'Local Office could not initialize. Rerun tmt office start and reopen its URL.'
  );
});

test('offline board renders plain text and preserves owner edits, replies and moderation', async ({
  page,
}) => {
  const token = 'b'.repeat(43);
  const threadId = '11111111-1111-4111-8111-111111111111';
  const identityReplyId = '22222222-2222-4222-8222-222222222222';
  const ownerReplyId = '33333333-3333-4333-8333-333333333333';
  let revision = 1;
  let title = 'What we learned';
  let body = '<script>still plain text</script>';
  let identityReplyDeleted = false;
  let ownerReply: Record<string, unknown> | undefined;
  const category = { kind: 'general' };
  const owner = { kind: 'owner' };
  const entry = () => ({
    id: threadId,
    threadId,
    category,
    author: owner,
    revision,
    deleted: false,
    createdAtMs: 1,
    updatedAtMs: revision,
    title,
    body,
  });
  const replies = () => [
    {
      id: identityReplyId,
      threadId,
      category,
      author: {
        kind: 'identity',
        identityId: '44444444-4444-4444-8444-444444444444',
        name: 'Alice',
      },
      revision: identityReplyDeleted ? 2 : 1,
      deleted: identityReplyDeleted,
      createdAtMs: 2,
      updatedAtMs: identityReplyDeleted ? 4 : 2,
      ...(identityReplyDeleted ? {} : { body: 'Please document the edge case.' }),
    },
    ...(ownerReply ? [ownerReply] : []),
  ];
  const authorized = (request: Request) => {
    expect(request.headers().authorization).toBe(`Bearer ${token}`);
    expect(request.headers().origin).toBe('http://127.0.0.1:4176');
  };
  await page.route('**/api/v1/local/board/categories/list', async (route) => {
    authorized(route.request());
    await route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({ categories: [category], nextCursor: null, boardRevision: revision }),
    });
  });
  await page.route('**/api/v1/local/board/threads/list', async (route) => {
    authorized(route.request());
    await route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({
        threads: [
          {
            ...entry(),
            body: undefined,
            replyCount: replies().filter((reply) => !reply.deleted).length,
            activitySequence: revision,
          },
        ],
        nextCursor: null,
        boardRevision: revision,
      }),
    });
  });
  await page.route('**/api/v1/local/board/threads/show', async (route) => {
    authorized(route.request());
    await route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({
        thread: entry(),
        replies: replies(),
        nextCursor: null,
        boardRevision: revision,
      }),
    });
  });
  await page.route(`**/api/v1/local/board/entries/${threadId}`, async (route) => {
    authorized(route.request());
    expect(route.request().method()).toBe('PUT');
    const input = route.request().postDataJSON();
    expect(input.actor).toBeUndefined();
    title = input.title;
    body = input.body;
    revision += 1;
    await route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({
        entryId: threadId,
        revision,
        changed: true,
        operationId: input.operationId,
      }),
    });
  });
  await page.route(`**/api/v1/local/board/threads/${threadId}/replies`, async (route) => {
    authorized(route.request());
    const input = route.request().postDataJSON();
    ownerReply = {
      id: ownerReplyId,
      threadId,
      category,
      author: owner,
      revision: 1,
      deleted: false,
      createdAtMs: 3,
      updatedAtMs: 3,
      body: input.body,
    };
    revision += 1;
    await route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({
        entryId: ownerReplyId,
        threadId,
        revision: 1,
        created: true,
        operationId: input.operationId,
      }),
    });
  });
  await page.route(`**/api/v1/local/board/entries/${identityReplyId}`, async (route) => {
    authorized(route.request());
    const input = route.request().postDataJSON();
    expect(input.moderate).toBe(true);
    identityReplyDeleted = true;
    revision += 1;
    await route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({
        entryId: identityReplyId,
        revision: 2,
        deleted: true,
        changed: true,
        moderated: true,
        operationId: input.operationId,
      }),
    });
  });

  await page.goto(`http://127.0.0.1:4176/local/board#token=${token}`);
  await expect(page).toHaveURL('http://127.0.0.1:4176/local/board');
  await expect(page.getByRole('heading', { name: title })).toBeVisible();
  await expect(page.getByText('<script>still plain text</script>')).toBeVisible();
  expect(await page.locator('.board-body script').count()).toBe(0);

  await page.getByRole('button', { name: 'Edit' }).click();
  await page.getByLabel('Message').first().fill('Updated owner note');
  await page.getByRole('button', { name: 'Save edit' }).click();
  await expect(page.getByRole('button', { name: 'Save edit' })).toBeHidden();
  await expect(
    page.locator('.board-entry').first().getByText('Updated owner note', { exact: true })
  ).toBeVisible();

  const replyForm = page.locator('.board-reply-form');
  await replyForm.getByRole('textbox', { name: 'Message' }).fill('Owner follow-up');
  await replyForm.getByRole('button', { name: 'Reply as owner' }).click();
  await expect(page.getByText('Owner follow-up')).toBeVisible();

  page.once('dialog', (dialog) => void dialog.accept());
  await page.getByRole('button', { name: 'Moderate delete' }).click();
  await expect(page.getByText('Deleted entry')).toBeVisible();

  await page.goto(`http://127.0.0.1:4176/local/board#token=${token}`);
  await expect(page.getByText('Updated owner note')).toBeVisible();
  await expect(page.getByText('Owner follow-up')).toBeVisible();
});
