import Database from '../../../../../../typescript/test/support/sqlite-oracle.js';
import { expect, test } from '@playwright/test';
import { openOfficeObjects } from './office-navigation.js';
import { installationWorldPoint } from './native-world-geometry.js';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';

function stored(databasePath: string) {
  const database = new Database(databasePath, { readonly: true });
  try {
    return {
      documents: database.prepare('SELECT count(*) FROM office_whiteboards').pluck().get(),
      document: database
        .prepare('SELECT revision, scene FROM office_whiteboards WHERE document_id=?')
        .get('lobby') as { revision: number; scene: string } | undefined,
      operations: database
        .prepare('SELECT count(*) FROM office_whiteboard_operations')
        .pluck()
        .get(),
      requests: database.prepare('SELECT count(*) FROM request_attempts').pluck().get(),
    };
  } finally {
    database.close();
  }
}

test('the spatial and keyboard whiteboard entries share a lazy draft without dispatching work', async ({
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
    const started = await office(['start', '--port', String(await unusedLoopbackPort())]);
    let reads = 0;
    page.on('request', (request) => {
      if (
        request.method() === 'GET' &&
        new URL(request.url()).pathname === '/api/v1/local/whiteboards/lobby'
      )
        reads++;
    });
    try {
      await page.setViewportSize({ width: 1440, height: 1000 });
      await page.goto(started.url);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      expect(reads).toBe(0);
      expect(stored(sandbox.database)).toEqual({
        documents: 0,
        document: undefined,
        operations: 0,
        requests: 0,
      });
      const world = page.locator('.office-canvas canvas');
      const bounds = (await world.boundingBox())!;
      // Whiteboard footprint [82,18,16,16] projects upright to [82,13.75,16,16].
      const point = installationWorldPoint(bounds, 106, 21.75);
      await page.mouse.move(point.x, point.y);
      await expect(world).toHaveCSS('cursor', 'grab');
      await page.screenshot({ path: testInfo.outputPath('lobby-both-functional-boards.png') });
      // The expanded action label is above the artwork. Clicking it must keep
      // the same target through pointerdown/up rather than collapsing on press.
      const actionY = installationWorldPoint(bounds, 106, 12.75).y;
      await page.mouse.move(point.x, actionY);
      await expect(world).toHaveCSS('cursor', 'pointer');
      await page.mouse.click(point.x, actionY);
      const panel = page.getByRole('dialog', { name: 'Whiteboard', exact: true });
      await expect(panel.getByText('New board · not saved')).toBeVisible();
      expect(reads).toBe(1);
      const surface = panel.getByLabel('Whiteboard drawing surface');
      await panel.getByRole('button', { name: 'Text', exact: true }).click();
      await surface.click({ position: { x: 70, y: 80 } });
      await panel
        .getByRole('textbox', { name: 'Text', exact: true })
        .fill('One shared thinking space');
      await panel.getByRole('button', { name: 'Apply text', exact: true }).click();
      await expect(panel.getByText('1 elements · click to select')).toBeVisible();
      await panel.getByRole('button', { name: 'Close whiteboard' }).click();
      await expect(panel).not.toBeVisible();
      await expect(world).toBeFocused();
      await openOfficeObjects(page);
      const entry = page.getByRole('button', { name: 'Open whiteboard', exact: true });
      await entry.focus();
      await entry.press('Enter');
      await expect(panel.getByRole('textbox', { name: 'Text', exact: true })).toHaveValue(
        'One shared thinking space'
      );
      expect(reads).toBe(1);
      await page.keyboard.press('Escape');
      await expect(panel).not.toBeVisible();
      await expect(entry).toBeFocused();
      await page.mouse.move(point.x, point.y);
      await expect(world).toHaveCSS('cursor', 'grab');
      await page.mouse.move(point.x, actionY);
      await expect(world).toHaveCSS('cursor', 'pointer');
      await page.mouse.click(point.x, actionY);
      await expect(panel.getByText('1 elements · click to select')).toBeVisible();
      await panel.getByRole('button', { name: 'Erase', exact: true }).click();
      await surface.click({ position: { x: 90, y: 90 } });
      await expect(panel.getByText('0 elements · click to select')).toBeVisible();
      await panel.getByRole('button', { name: 'Undo', exact: true }).click();
      await expect(panel.getByText('1 elements · click to select')).toBeVisible();
      expect(stored(sandbox.database)).toEqual({
        documents: 0,
        document: undefined,
        operations: 0,
        requests: 0,
      });
      await page.screenshot({ path: testInfo.outputPath('spatial-whiteboard-draft.png') });
      await panel.getByRole('button', { name: 'Save', exact: true }).click();
      await expect(panel.getByText('Saved · revision 1')).toBeVisible();
      const saved = stored(sandbox.database);
      expect(saved).toMatchObject({
        documents: 1,
        document: { revision: 1 },
        operations: 1,
        requests: 0,
      });
      expect(JSON.parse(saved.document!.scene).elements).toEqual([
        expect.objectContaining({ kind: 'text', text: 'One shared thinking space' }),
      ]);
      await page.setViewportSize({ width: 390, height: 844 });
      await expect(panel.getByRole('button', { name: 'Close whiteboard' })).toBeVisible();
      await page.screenshot({
        path: testInfo.outputPath('spatial-whiteboard-mobile.png'),
        fullPage: true,
      });
      expect(await panel.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(
        true
      );
      await panel.getByRole('button', { name: 'Close whiteboard' }).click();
      // A distinct route reads the same saved resource, not another extension-owned copy.
      const direct = new URL(started.url);
      direct.pathname = '/local/whiteboards/lobby';
      await page.goto('about:blank');
      await page.goto(direct.href);
      await expect(page.getByText('Saved · revision 1')).toBeVisible();
      await expect(page.getByText('1 elements · click to select')).toBeVisible();
      expect(stored(sandbox.database)).toEqual(saved);
    } finally {
      await page.goto('about:blank');
      await office(['stop']);
      expect((await office(['status'])).service.running).toBe(false);
    }
  });
});

test('whiteboard gestures, text and undo persist only explicit saves and survive a fresh browser document', async ({
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
    const started = await office(['start', '--port', String(await unusedLoopbackPort())]);
    const url = new URL(started.url);
    url.pathname = '/local/whiteboards/lobby';
    try {
      await page.setViewportSize({ width: 1440, height: 1000 });
      await page.goto(url.href);
      await expect(page.getByText('0 elements · click to select')).toBeVisible();
      expect(stored(sandbox.database)).toEqual({
        documents: 0,
        document: undefined,
        operations: 0,
        requests: 0,
      });
      const canvas = page.getByLabel('Whiteboard drawing surface');
      const bounds = (await canvas.boundingBox())!;
      const at = (x: number, y: number) => ({
        x: bounds.x + (bounds.width * x) / 1600,
        y: bounds.y + (bounds.height * y) / 1000,
      });
      async function drag(tool: string, start: [number, number], end: [number, number]) {
        await page.getByRole('button', { name: tool, exact: true }).click();
        const from = at(...start),
          to = at(...end);
        await page.mouse.move(from.x, from.y);
        await page.mouse.down();
        await page.mouse.move(to.x, to.y, { steps: 6 });
        await page.mouse.up();
      }
      await drag('Box', [750, 180], [1200, 420]);
      await expect(page.getByText('1 elements · click to select')).toBeVisible();
      await page.getByRole('button', { name: 'Undo', exact: true }).click();
      await expect(page.getByText('0 elements · click to select')).toBeVisible();
      await page.getByRole('button', { name: 'Redo', exact: true }).click();
      await expect(page.getByText('1 elements · click to select')).toBeVisible();
      await drag('Arrow', [520, 330], [740, 300]);
      await drag('Ellipse', [850, 540], [1230, 770]);
      await drag('Pen', [250, 650], [640, 730]);
      await page.getByRole('button', { name: 'Note', exact: true }).click();
      const note = at(130, 170);
      await page.mouse.click(note.x, note.y);
      await page
        .getByRole('textbox', { name: 'Text', exact: true })
        .fill('Office ideas\nKeep the host API small.\nDraw, review, then share.');
      await page.getByRole('button', { name: 'Apply text', exact: true }).click();
      await expect(page.getByText('5 elements · click to select')).toBeVisible();
      await drag('Select', [180, 200], [240, 270]);

      // A captured pen gesture canceled with Escape is not a sixth element.
      await page.getByRole('button', { name: 'Pen', exact: true }).click();
      const from = at(100, 820),
        to = at(650, 880);
      await canvas.focus();
      await page.mouse.move(from.x, from.y);
      await page.mouse.down();
      await page.mouse.move(to.x, to.y);
      await page.keyboard.press('Escape');
      await page.mouse.up();
      await expect(page.getByText('5 elements · click to select')).toBeVisible();
      expect(stored(sandbox.database)).toEqual({
        documents: 0,
        document: undefined,
        operations: 0,
        requests: 0,
      });
      await page.screenshot({ path: testInfo.outputPath('whiteboard-desktop-draft.png') });
      await page.getByRole('button', { name: 'Save', exact: true }).click();
      await expect(page.getByText('Saved · revision 1')).toBeVisible();
      const saved = stored(sandbox.database);
      expect(saved).toMatchObject({
        documents: 1,
        operations: 1,
        requests: 0,
        document: { revision: 1 },
      });
      const elements = JSON.parse(saved.document!.scene).elements;
      expect(elements.map((item: { kind: string }) => item.kind)).toEqual([
        'rectangle',
        'arrow',
        'ellipse',
        'stroke',
        'note',
      ]);
      expect(elements[4]).toMatchObject({
        x: 190,
        y: 240,
        text: 'Office ideas\nKeep the host API small.\nDraw, review, then share.',
      });
      // The session token lives only in memory; use the original authorized URL in a fresh document.
      await page.goto('about:blank');
      await page.goto(url.href);
      await expect(page.getByText('Saved · revision 1')).toBeVisible();
      await expect(page.getByText('5 elements · click to select')).toBeVisible();
      expect(stored(sandbox.database)).toEqual(saved);
      await page.setViewportSize({ width: 390, height: 844 });
      await expect(async () => {
        const raster = await canvas.evaluate((element) => {
          const surface = element as HTMLCanvasElement;
          return {
            width: surface.width,
            expected: Math.round(
              surface.getBoundingClientRect().width * Math.min(devicePixelRatio, 2)
            ),
          };
        });
        expect(raster.width).toBe(raster.expected);
      }).toPass();
      const visibleBottomEdge = await canvas.evaluate((element) => {
        const surface = element as HTMLCanvasElement;
        const x = Math.round((1000 / 1600) * surface.width),
          y = Math.round((420 / 1000) * surface.height);
        const pixels = surface.getContext('2d')!.getImageData(x - 2, y - 2, 5, 5).data;
        for (let index = 0; index < pixels.length; index += 4)
          if (pixels[index]! < 230 && pixels[index + 1]! < 235) return true;
        return false;
      });
      expect(visibleBottomEdge).toBe(true);
      await page.screenshot({ path: testInfo.outputPath('whiteboard-mobile.png'), fullPage: true });
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true
      );
    } finally {
      await page.goto('about:blank');
      await office(['stop']);
      const status = await office(['status']);
      expect(status.service.running).toBe(false);
    }
  });
});
