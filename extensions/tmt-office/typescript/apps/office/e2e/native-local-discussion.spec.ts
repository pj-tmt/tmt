import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import Database from '../../../../../../typescript/test/support/sqlite-oracle.js';
import { expect, test } from '@playwright/test';
import { openOfficeObjects } from './office-navigation.js';
import { installationWorldPoint } from './native-world-geometry.js';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';

/** Read-only observation: opening a panel must not materialize resources or dispatch work. */
function discussionState(databasePath: string) {
  const database = new Database(databasePath, { readonly: true });
  try {
    return {
      identities: database.prepare('SELECT count(*) FROM identities').pluck().get(),
      worlds: database.prepare('SELECT count(*) FROM office_local_worlds').pluck().get(),
      blocks: database.prepare('SELECT count(*) FROM office_local_blocks').pluck().get(),
      requests: database.prepare('SELECT count(*) FROM request_attempts').pluck().get(),
      operations: database.prepare('SELECT count(*) FROM office_board_operations').pluck().get(),
      entries: database
        .prepare(
          'SELECT id, thread_id, is_root, category_kind, author_kind, title, body FROM office_board_entries ORDER BY created_sequence'
        )
        .all(),
    };
  } finally {
    database.close();
  }
}

test('unusable storage fails before Office publishes readiness', async () => {
  await withSandbox(async (sandbox) => {
    const prefix = await installNativeOffice(sandbox);
    mkdirSync(sandbox.globalDir, { recursive: true });
    const original = 'Not a SQLite database: preserve this diagnostic fixture.';
    writeFileSync(sandbox.database, original);
    const office = (args: string[]) =>
      runCli(sandbox, ['office', '--prefix', prefix, ...args, '--json']);
    try {
      const started = await office(['start', '--port', String(await unusedLoopbackPort())]);
      expect(started.status).toBe(1);
      expect(JSON.parse(started.stdout).error.code).toBe('OFFICE_SERVICE_UNAVAILABLE');
      expect(readFileSync(sandbox.database, 'utf8')).toBe(original);
      expect(existsSync(path.join(sandbox.globalDir, 'office', 'runtime', 'service-v1.json'))).toBe(
        false
      );
      const status = await office(['status']);
      expect(status.status).toBe(0);
      expect(JSON.parse(status.stdout).service.running).toBe(false);
    } finally {
      const stopped = await office(['stop']);
      expect(stopped.status).toBe(0);
    }
  });
});

test('lobby discussions preserve the world and drafts, persist explicit posts, and navigate on mobile', async ({
  page,
}, testInfo) => {
  test.setTimeout(120_000);
  await withSandbox(async (sandbox) => {
    const prefix = await installNativeOffice(sandbox);
    const office = async (args: string[]) => {
      const result = await runCli(sandbox, ['office', '--prefix', prefix, ...args, '--json'], {
        deadlineMs: 30_000,
      });
      expect(result.status, result.stdout).toBe(0);
      return JSON.parse(result.stdout);
    };
    let started = await office(['start', '--port', String(await unusedLoopbackPort())]);
    const responses: Promise<void>[] = [];
    const traffic: unknown[] = [];
    page.on('requestfailed', (request) => {
      const endpoint = new URL(request.url()).pathname;
      if (endpoint.startsWith('/api/')) {
        traffic.push({ endpoint, error: request.failure()?.errorText });
      }
    });
    page.on('response', (response) => {
      const endpoint = new URL(response.url()).pathname;
      if (endpoint.startsWith('/api/')) {
        responses.push(
          response
            .text()
            .then((body) => {
              traffic.push({ endpoint, status: response.status(), body });
            })
            .catch((error) => {
              traffic.push({ endpoint, error: String(error) });
            })
        );
      }
    });
    try {
      await page.setViewportSize({ width: 1440, height: 1000 });
      // Exact lifecycle screenshots exclude the intentional bridge glow.
      await page.emulateMedia({ reducedMotion: 'reduce' });
      await page.goto(started.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      const initial = discussionState(sandbox.database);
      const initialWorld = await office(['layout', 'show']);
      expect(initial).toMatchObject({
        identities: 0,
        blocks: 0,
        requests: 0,
        operations: 0,
        entries: [],
      });

      const canvas = page.locator('.office-canvas canvas');
      const canvasBounds = await canvas.boundingBox();
      expect(canvasBounds).not.toBeNull();
      // Discussion footprint [64,18,12,12] projects upright to [64,14.25,12,12].
      const boardPoint = installationWorldPoint(canvasBounds!, 86, 20.25);
      await page.screenshot({ path: testInfo.outputPath('lobby-spatial-board.png') });
      await page.mouse.move(boardPoint.x, boardPoint.y);
      await expect(canvas).toHaveCSS('cursor', 'grab');
      await page.mouse.down();
      await page.mouse.move(boardPoint.x + 30, boardPoint.y);
      await page.mouse.up();
      await expect(page.getByRole('dialog', { name: 'Discussion board' })).not.toBeVisible();
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      await page.getByRole('button', { name: 'Undo', exact: true }).click();
      await expect(
        page.getByRole('region', { name: 'Layout changes' }).getByRole('status')
      ).toHaveText('All changes applied');
      expect((await office(['layout', 'show'])).layout).toEqual(initialWorld.layout);
      const resourceBaseline = discussionState(sandbox.database);
      expect(resourceBaseline).toEqual({ ...initial, worlds: 1 });
      await page.getByRole('button', { name: 'Fit office', exact: true }).click();

      // Observe the browser's real pointer ID rather than assuming the mouse uses 1.
      await canvas.evaluate((element) =>
        element.addEventListener(
          'pointerdown',
          (event) => {
            element.setAttribute('data-test-pointer', String((event as PointerEvent).pointerId));
          },
          { once: true }
        )
      );
      await page.mouse.move(boardPoint.x, boardPoint.y);
      await page.mouse.down();
      const pointerId = Number(await canvas.getAttribute('data-test-pointer'));
      expect(pointerId).toBeGreaterThan(0);
      await canvas.dispatchEvent('pointercancel', { pointerId, button: 0, bubbles: true });
      await page.mouse.up();
      await canvas.evaluate((element) => element.removeAttribute('data-test-pointer'));
      await expect(page.getByRole('dialog', { name: 'Discussion board' })).not.toBeVisible();
      expect(discussionState(sandbox.database)).toEqual(resourceBaseline);

      // A changed camera makes replacement/refitting detectable, not just canvas presence.
      // Keep the crop below the lobby HUD: its focus-visible ring legitimately
      // changes after keyboard input, whereas the painted world must not move.
      const worldStrip = {
        x: Math.floor(boardPoint.x - 150),
        y: Math.floor(boardPoint.y - 130),
        width: 220,
        height: 160,
      };
      const unpanned = await page.screenshot({ clip: worldStrip });
      await page.mouse.move(1000, 650);
      await page.mouse.down();
      await page.mouse.move(1040, 670);
      await page.mouse.up();
      const panned = await page.screenshot({ clip: worldStrip });
      expect(panned.equals(unpanned)).toBe(false);
      await openOfficeObjects(page);
      const openBoard = page.getByRole('button', { name: 'Open discussion board', exact: true });
      // Keyboard focus intentionally lights only the selected object. Compare a neutral
      // canvas state across the modal lifecycle so the transient action label is excluded.
      // A neutral Lobby interior, independently projected after the known pan.
      // Fixed screen coordinates can accidentally cover the focused board when
      // the starter's fitted bounds change.
      const roomStart = installationWorldPoint(canvasBounds!, 8, 55);
      const roomEnd = installationWorldPoint(canvasBounds!, 36, 77);
      const roomStrip = {
        x: Math.ceil(roomStart.x + 40),
        y: Math.ceil(roomStart.y + 20),
        width: Math.floor(roomEnd.x - roomStart.x),
        height: Math.floor(roomEnd.y - roomStart.y),
      };
      const beforeFocus = await page.screenshot({ clip: roomStrip });
      const beforeHighlight = await page.screenshot({ clip: worldStrip });
      await openBoard.focus();
      const focusedBoard = await page.screenshot({ clip: worldStrip });
      expect(focusedBoard.equals(beforeHighlight)).toBe(false);
      expect((await page.screenshot({ clip: roomStrip })).equals(beforeFocus)).toBe(true);
      await page.getByRole('button', { name: 'Fit office', exact: true }).focus();
      await page.mouse.move(10, 900);
      const beforeBoard = await page.screenshot({ clip: worldStrip });
      writeFileSync(testInfo.outputPath('world-before-dialog.png'), beforeBoard);
      await openBoard.focus();
      await openBoard.press('Enter');
      const board = page.getByRole('dialog', { name: 'Discussion board', exact: true });
      await expect(board.getByText('No posts in this category yet.')).toBeVisible();
      await expect(board.getByLabel('Order')).toHaveValue('updated');
      expect(discussionState(sandbox.database)).toEqual(resourceBaseline);
      await board.getByRole('button', { name: 'New post', exact: true }).click();
      const title = 'A quieter corner for code review';
      const body =
        'Keep the shared board for discoveries.\nBring focused reviews to a meeting room.';
      await board.getByLabel('Title', { exact: true }).fill(title);
      await board.getByRole('textbox', { name: 'Message', exact: true }).fill(body);
      await page.keyboard.press('Escape');
      await expect(board).not.toBeVisible();
      await expect(openBoard).toBeFocused();
      await page.getByRole('button', { name: 'Fit office', exact: true }).focus();
      await page.mouse.move(10, 900);
      const afterBoard = await page.screenshot({ clip: worldStrip });
      writeFileSync(testInfo.outputPath('world-after-dialog.png'), afterBoard);
      expect(afterBoard.equals(beforeBoard)).toBe(true);
      expect(discussionState(sandbox.database)).toEqual(resourceBaseline);
      // Reopen the very same draft by clicking the physical object after the known pan.
      const pannedBoard = { x: boardPoint.x + 40, y: boardPoint.y + 20 };
      await page.mouse.move(pannedBoard.x, pannedBoard.y);
      await expect(canvas).toHaveCSS('cursor', 'grab');
      const actionY = installationWorldPoint(canvasBounds!, 86, 13.25).y + 20;
      await page.mouse.move(pannedBoard.x, actionY);
      await expect(canvas).toHaveCSS('cursor', 'pointer');
      await page.mouse.click(pannedBoard.x, actionY);
      await expect(board).toBeVisible();
      await expect(board.getByLabel('Title', { exact: true })).toHaveValue(title);
      await expect(board.getByRole('textbox', { name: 'Message', exact: true })).toHaveValue(body);
      await page.keyboard.press('Escape');
      await expect(canvas).toBeFocused();
      expect(discussionState(sandbox.database)).toEqual(resourceBaseline);
      await openOfficeObjects(page);
      await openBoard.focus();
      await openBoard.press('Enter');
      await expect(board.getByLabel('Title', { exact: true })).toHaveValue(title);
      await expect(board.getByRole('textbox', { name: 'Message', exact: true })).toHaveValue(body);
      await board.getByRole('button', { name: 'Post as owner', exact: true }).click();
      await expect(board.getByRole('heading', { name: title, exact: true })).toBeVisible();
      const posted = discussionState(sandbox.database);
      expect(posted).toMatchObject({
        identities: 0,
        blocks: 0,
        requests: 0,
        operations: 1,
        entries: [{ is_root: 1, category_kind: 'general', author_kind: 'owner', title, body }],
      });
      expect(posted.entries).toHaveLength(1);
      const replyForm = board.locator('.board-reply-form');
      await replyForm
        .getByRole('textbox', { name: 'Message', exact: true })
        .fill('Agreed. The lobby stays open for sharing.');
      await replyForm.getByRole('button', { name: 'Reply as owner' }).click();
      await expect(board.locator('.board-replies .board-body')).toHaveText([
        'Agreed. The lobby stays open for sharing.',
      ]);
      const replied = discussionState(sandbox.database);
      expect(replied.operations).toBe(2);
      expect(replied.requests).toBe(0);
      expect(replied.entries).toHaveLength(2);
      expect(replied.entries[0]).toEqual(posted.entries[0]);
      expect(replied.entries[1]).toMatchObject({
        thread_id: (posted.entries[0] as { id: string }).id,
        is_root: 0,
        author_kind: 'owner',
        body: 'Agreed. The lobby stays open for sharing.',
      });
      await page.screenshot({ path: testInfo.outputPath('discussion-desktop.png') });

      await page.setViewportSize({ width: 390, height: 844 });
      await board.getByRole('button', { name: 'Back to threads' }).click();
      const threadButton = board.getByRole('button', { name: new RegExp(title) });
      await expect(threadButton).toBeVisible();
      await expect(board.getByRole('region', { name: 'Selected discussion' })).not.toBeVisible();
      await threadButton.click();
      await expect(board.getByRole('heading', { name: title, exact: true })).toBeVisible();
      await expect(threadButton).not.toBeVisible();
      const bounds = await board.evaluate((element) => ({
        width: element.clientWidth,
        content: element.scrollWidth,
        right: element.getBoundingClientRect().right,
      }));
      expect(bounds.content).toBeLessThanOrEqual(bounds.width);
      expect(bounds.right).toBeLessThanOrEqual(390);
      await page.screenshot({ path: testInfo.outputPath('discussion-mobile.png') });
      await board.getByRole('button', { name: 'Back to threads' }).click();
      await expect(threadButton).toBeFocused();
      await board.getByRole('button', { name: 'Close discussion board' }).click();
      expect(discussionState(sandbox.database)).toEqual(replied);
      // A fresh browser view after a service restart reads the same durable discussion.
      await office(['stop']);
      started = await office(['start', '--port', String(await unusedLoopbackPort())]);
      await page.setViewportSize({ width: 1440, height: 1000 });
      await page.goto(started.url);
      await openOfficeObjects(page);
      await page.getByRole('button', { name: 'Open discussion board', exact: true }).click();
      await expect(board.getByRole('heading', { name: title, exact: true })).toBeVisible();
      await expect(board.locator('.board-replies .board-body')).toHaveText([
        'Agreed. The lobby stays open for sharing.',
      ]);
      expect(discussionState(sandbox.database)).toEqual(replied);
      await board.getByRole('button', { name: 'Close discussion board' }).click();
      // If every renderer is unavailable, the same accessible entry and board service remain.
      // Blocking WebGL alone is insufficient: Pixi can fall back to WebGPU or Canvas.
      await page.addInitScript(() => {
        Object.defineProperty(navigator, 'gpu', { configurable: true, value: undefined });
        HTMLCanvasElement.prototype.getContext = () => null;
      });
      // Start a new document, then supply the session fragment again. It is stripped
      // on boot and retained only in memory, so reload alone loses authorization.
      await page.goto('about:blank');
      await page.goto(started.url);
      await openOfficeObjects(page);
      expect(
        await page.evaluate(() => document.createElement('canvas').getContext('2d'))
      ).toBeNull();
      await expect(
        page.getByText(
          'The map could not initialize. Your rooms are still accessible from the agent directory.'
        )
      ).toBeVisible();
      const fallback = page.getByRole('button', { name: 'Open discussion board', exact: true });
      await fallback.focus();
      await fallback.press('Enter');
      await expect(board.getByRole('heading', { name: title, exact: true })).toBeVisible();
      expect(discussionState(sandbox.database)).toEqual(replied);
    } finally {
      try {
        await Promise.all(responses);
        // Synthetic fixture content only; never capture authorization headers or session URLs.
        const evidence = testInfo.outputPath('local-api-traffic.json');
        writeFileSync(evidence, JSON.stringify(traffic, null, 2));
        await testInfo.attach('local-api-traffic', {
          path: evidence,
          contentType: 'application/json',
        });
      } finally {
        await office(['stop']);
      }
    }
  });
});
