import Database from '../../../../../../typescript/test/support/sqlite-oracle.js';
import { copyFile, readFile, readdir } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import { openOfficeObjects } from './office-navigation.js';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';

function snapshotState(databasePath: string) {
  const database = new Database(databasePath, { readonly: true });
  try {
    return {
      captures: database
        .prepare(
          'SELECT snapshot_id,document_revision,scene,selected_element_ids,annotation FROM office_whiteboard_snapshots'
        )
        .all() as {
        snapshot_id: string;
        document_revision: number;
        scene: string;
        selected_element_ids: string;
        annotation: string;
      }[],
      images: database
        .prepare('SELECT snapshot_id,png FROM office_whiteboard_snapshot_images')
        .all() as { snapshot_id: string; png: Buffer }[],
      requests: database.prepare('SELECT count(*) FROM request_attempts').pluck().get(),
    };
  } finally {
    database.close();
  }
}

test('a browser-drawn snapshot retains annotated PNG pixels through later edits and service restart', async ({
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
      expect(result.stderr).toBe('');
      return JSON.parse(result.stdout);
    };
    let started = await office(['start', '--port', String(await unusedLoopbackPort())]);
    try {
      await page.setViewportSize({ width: 1440, height: 1000 });
      await page.goto(started.url);
      await openOfficeObjects(page);
      await page.getByRole('button', { name: 'Open whiteboard', exact: true }).click();
      const panel = page.getByRole('dialog', { name: 'Whiteboard', exact: true });
      await expect(panel.getByText('New board · not saved')).toBeVisible();
      await panel.getByRole('button', { name: 'Review snapshot', exact: true }).click();
      await expect(
        panel.getByRole('button', { name: 'Create snapshot', exact: true })
      ).toBeDisabled();
      expect(snapshotState(sandbox.database)).toEqual({ captures: [], images: [], requests: 0 });
      await panel.getByRole('button', { name: 'Back to drawing', exact: true }).click();
      await panel.getByRole('button', { name: 'Note', exact: true }).click();
      await panel.getByLabel('Whiteboard drawing surface').click({ position: { x: 85, y: 90 } });
      await panel.getByRole('textbox', { name: 'Text', exact: true }).fill('API routing sketch');
      await panel.getByRole('button', { name: 'Apply text', exact: true }).click();
      await panel.getByRole('button', { name: 'Save', exact: true }).click();
      await expect(panel.getByText('Saved · revision 1')).toBeVisible();
      await panel.getByRole('button', { name: 'Review snapshot', exact: true }).click();
      await panel
        .getByRole('textbox', { name: 'Annotation', exact: true })
        .fill('Please review the selected API note.');
      await panel
        .getByRole('checkbox', { name: '1. note — API routing sketch', exact: true })
        .check();
      const capturing = page.waitForResponse(
        (response) =>
          response.request().method() === 'POST' &&
          new URL(response.url()).pathname.endsWith('/whiteboards/lobby/snapshots')
      );
      const uploading = page.waitForResponse(
        (response) =>
          response.request().method() === 'PUT' &&
          new URL(response.url()).pathname.endsWith('/image')
      );
      await panel.getByRole('button', { name: 'Create snapshot', exact: true }).click();
      const capturedResponse = await capturing;
      expect(capturedResponse.status()).toBe(200);
      const captured = await capturedResponse.json();
      const uploadedResponse = await uploading;
      expect(uploadedResponse.status()).toBe(200);
      const uploaded = await uploadedResponse.body();
      const image = panel.getByRole('img', { name: 'Saved whiteboard snapshot', exact: true });
      await expect(image).toBeVisible();
      await expect(panel.getByText('Snapshot ready', { exact: true })).toBeVisible();
      await expect(image).toHaveJSProperty('naturalWidth', 1600);
      await expect(image).toHaveJSProperty('naturalHeight', 1000);
      const stored = snapshotState(sandbox.database);
      expect(stored.captures).toHaveLength(1);
      expect(stored.images).toHaveLength(1);
      expect(stored.requests).toBe(0);
      expect(stored.captures[0]).toMatchObject({
        snapshot_id: captured.id,
        document_revision: 1,
        annotation: 'Please review the selected API note.',
      });
      expect(JSON.parse(stored.captures[0]!.scene)).toEqual(captured.scene);
      expect(captured.scene.elements).toEqual([
        expect.objectContaining({ kind: 'note', text: 'API routing sketch' }),
      ]);
      expect(JSON.parse(stored.captures[0]!.selected_element_ids)).toEqual([
        captured.scene.elements[0].id,
      ]);
      expect(stored.images[0]!.png.equals(uploaded)).toBe(true);
      expect([...uploaded.subarray(0, 8)]).toEqual([137, 80, 78, 71, 13, 10, 26, 10]);
      const pixels = await image.evaluate((element) => {
        const canvas = document.createElement('canvas');
        canvas.width = 1600;
        canvas.height = 1000;
        const context = canvas.getContext('2d')!;
        context.drawImage(element as HTMLImageElement, 0, 0);
        const bytes = context.getImageData(0, 0, 1600, 1000).data;
        let highlighted = 0;
        for (let i = 0; i < bytes.length; i += 4)
          if (
            bytes[i] === 0 &&
            bytes[i + 1] === 140 &&
            bytes[i + 2] === 145 &&
            bytes[i + 3] === 255
          )
            highlighted++;
        return { corner: [...context.getImageData(1590, 990, 1, 1).data], highlighted };
      });
      expect(pixels.corner).toEqual([255, 247, 231, 255]);
      expect(pixels.highlighted).toBeGreaterThan(20);
      const previewUrl = await image.getAttribute('src');
      expect(previewUrl).toMatch(/^blob:/);
      const reference = `tmt:whiteboard:snapshot:${captured.id}`;
      await expect(panel.getByRole('textbox', { name: 'Local snapshot reference' })).toHaveValue(
        reference
      );
      await page.context().grantPermissions(['clipboard-read', 'clipboard-write']);
      await panel.getByRole('button', { name: 'Copy reference', exact: true }).click();
      expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(reference);
      expect(snapshotState(sandbox.database)).toEqual(stored);
      await page.screenshot({ path: testInfo.outputPath('whiteboard-snapshot-desktop.png') });

      await panel.getByRole('button', { name: 'Back to drawing', exact: true }).click();
      await panel.getByRole('textbox', { name: 'Text', exact: true }).fill('New live drawing');
      await panel.getByRole('button', { name: 'Apply text', exact: true }).click();
      await panel.getByRole('button', { name: 'Save', exact: true }).click();
      await expect(panel.getByText('Saved · revision 2')).toBeVisible();
      await panel.getByRole('button', { name: 'Review snapshot', exact: true }).click();
      await expect(image).toHaveAttribute('src', previewUrl!);
      await expect(panel.getByText('Revision 1 · 1 highlighted · Image saved')).toBeVisible();
      expect(snapshotState(sandbox.database)).toEqual(stored);
      await panel.getByRole('button', { name: 'Close whiteboard', exact: true }).click();
      await page.getByRole('button', { name: 'Open whiteboard', exact: true }).click();
      await expect(image).toHaveAttribute('src', previewUrl!);
      await page.setViewportSize({ width: 390, height: 844 });
      await expect(image).toBeVisible();
      const viewport = panel.getByRole('region', { name: 'Snapshot image viewport', exact: true });
      await expect(panel.getByRole('button', { name: 'Fit board', exact: true })).toHaveAttribute(
        'aria-pressed',
        'true'
      );
      const fittedWidth = await image.evaluate((element) => element.getBoundingClientRect().width);
      expect(fittedWidth).toBeLessThan(390);
      await panel.getByRole('button', { name: 'Actual size', exact: true }).click();
      expect(await image.evaluate((element) => element.getBoundingClientRect().width)).toBe(1600);
      await viewport.focus();
      await page.keyboard.press('ArrowRight');
      await expect
        .poll(() => viewport.evaluate((element) => element.scrollLeft))
        .toBeGreaterThan(0);
      await viewport.evaluate((element) =>
        element.scrollTo({ left: 150, top: 150, behavior: 'instant' })
      );
      await page.screenshot({ path: testInfo.outputPath('whiteboard-snapshot-mobile-detail.png') });
      expect(await panel.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(
        true
      );
      await expect(image).toHaveAttribute('src', previewUrl!);
      expect(snapshotState(sandbox.database)).toEqual(stored);
      await panel.getByRole('button', { name: 'Fit board', exact: true }).click();
      expect(await image.evaluate((element) => element.getBoundingClientRect().width)).toBe(
        fittedWidth
      );
      await page.screenshot({
        path: testInfo.outputPath('whiteboard-snapshot-mobile.png'),
        fullPage: true,
      });
      expect(await panel.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(
        true
      );
      const canLoadPreview = () =>
        page.evaluate(
          (url) =>
            new Promise<boolean>((resolve) => {
              const image = new Image();
              image.onload = () => resolve(true);
              image.onerror = () => resolve(false);
              image.src = url;
            }),
          previewUrl!
        );
      expect(await canLoadPreview()).toBe(true);
      await panel.getByRole('button', { name: 'Start new preview', exact: true }).click();
      await expect(image).toHaveCount(0);
      expect(await canLoadPreview()).toBe(false);
      expect(snapshotState(sandbox.database)).toEqual(stored);
      await page.goto('about:blank');
      await office(['stop']);
      expect((await office(['status'])).service.running).toBe(false);
      // The same retained snapshot is readable natively with no browser service or token.
      const snapshotCommand = ['whiteboard', 'snapshot'];
      expect(await office([...snapshotCommand, 'show', reference])).toEqual(captured);
      expect(await office([...snapshotCommand, 'show', captured.id])).toEqual(captured);
      const output = path.join(sandbox.cwd, 'review.png');
      const exported = await office([...snapshotCommand, 'export', reference, '--output', output]);
      expect(exported).toEqual({ path: output, bytes: uploaded.length, snapshotId: captured.id });
      expect((await readFile(output)).equals(uploaded)).toBe(true);
      await copyFile(output, testInfo.outputPath('whiteboard-agent-export.png'));
      const beforeFailure = await readdir(sandbox.cwd);
      for (const [id, expected] of [
        [reference, 'OUTPUT_EXISTS'],
        ['77777777-7777-4777-8777-777777777777', 'WHITEBOARD_NOT_FOUND'],
      ]) {
        const failed = await runCli(sandbox, [
          'office',
          '--prefix',
          prefix,
          ...snapshotCommand,
          'export',
          id!,
          '--output',
          output,
          '--json',
        ]);
        expect(failed.status).toBe(1);
        expect(JSON.parse(failed.stdout).error.code).toBe(expected);
        expect((await readFile(output)).equals(uploaded)).toBe(true);
        expect(await readdir(sandbox.cwd)).toEqual(beforeFailure);
      }
      expect(snapshotState(sandbox.database)).toEqual(stored);
      expect((await office(['status'])).service.running).toBe(false);
      started = await office(['start', '--port', String(await unusedLoopbackPort())]);
      const address = new URL(started.url);
      const token = new URLSearchParams(address.hash.slice(1)).get('token')!;
      const endpoint = `${address.origin}/api/v1/local/whiteboard-snapshots/${captured.id}`;
      const headers = { Authorization: `Bearer ${token}` };
      const read = await page.request.get(endpoint, { headers });
      expect(read.status()).toBe(200);
      expect(await read.json()).toEqual(captured);
      const readImage = await page.request.get(`${endpoint}/image`, { headers });
      expect(readImage.status()).toBe(200);
      expect((await readImage.body()).equals(uploaded)).toBe(true);
      expect(snapshotState(sandbox.database)).toEqual(stored);
    } finally {
      await page.goto('about:blank');
      await office(['stop']);
      expect((await office(['status'])).service.running).toBe(false);
    }
  });
});
