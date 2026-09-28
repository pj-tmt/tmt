import Database from '../../../../../../typescript/test/support/sqlite-oracle.js';
import { copyFile, readFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import type { DispatchInput, DispatchReceipt } from '../src/local/dispatch-contract.js';
import { openOfficeObjects } from './office-navigation.js';

function requestCounts(databasePath: string) {
  const database = new Database(databasePath, { readonly: true });
  try {
    return {
      attempts: database.prepare('SELECT count(*) FROM request_attempts').pluck().get(),
      operations: database.prepare('SELECT count(*) FROM office_dispatch_operations').pluck().get(),
    };
  } finally {
    database.close();
  }
}

test('a reviewed whiteboard request reaches the exact inbox, survives replay, and links a reply to its saved image', async ({
  page,
}, testInfo) => {
  await withSandbox(async (sandbox) => {
    const prefix = await installNativeOffice(sandbox);
    const cli = async (args: string[]) => {
      const result = await runCli(sandbox, [...args, '--json'], { deadlineMs: 30_000 });
      expect(result.status, result.stdout).toBe(0);
      expect(result.stderr).toBe('');
      return JSON.parse(result.stdout);
    };
    const office = (args: string[]) => cli(['office', '--prefix', prefix, ...args]);
    const alice = (await cli(['identity', 'create', 'Alice'])).identity;
    const bob = (await cli(['identity', 'create', 'Bob'])).identity;
    const started = await office(['start', '--port', String(await unusedLoopbackPort())]);
    const renderErrors: string[] = [];
    page.on('pageerror', (error) => renderErrors.push(error.message));
    try {
      await page.setViewportSize({ width: 1440, height: 1000 });
      await page.goto(started.url);
      await openOfficeObjects(page);
      await page.getByRole('button', { name: 'Open whiteboard', exact: true }).click();
      const panel = page.getByRole('dialog', { name: 'Whiteboard', exact: true });
      await panel.getByRole('button', { name: 'Note', exact: true }).click();
      await panel.getByLabel('Whiteboard drawing surface').click({ position: { x: 120, y: 105 } });
      await panel
        .getByRole('textbox', { name: 'Text', exact: true })
        .fill('Browser → inbox → review');
      await panel.getByRole('button', { name: 'Apply text', exact: true }).click();
      await panel.getByRole('button', { name: 'Save', exact: true }).click();
      await expect(panel.getByText('Saved · revision 1')).toBeVisible();
      await panel.getByRole('button', { name: 'Review snapshot', exact: true }).click();
      await panel
        .getByRole('textbox', { name: 'Annotation', exact: true })
        .fill('Check the delivery boundary.');
      await panel.getByRole('checkbox', { name: /1\. note/ }).check();
      const uploading = page.waitForResponse(
        (response) =>
          response.request().method() === 'PUT' &&
          new URL(response.url()).pathname.endsWith('/image')
      );
      await panel.getByRole('button', { name: 'Create snapshot', exact: true }).click();
      const imageResponse = await uploading;
      expect(imageResponse.status()).toBe(200);
      const png = await imageResponse.body();
      await expect(panel.getByRole('img', { name: 'Saved whiteboard snapshot' })).toBeVisible();
      const reference = await panel
        .getByRole('textbox', { name: 'Local snapshot reference' })
        .inputValue();
      await panel.getByRole('checkbox', { name: 'Alice · offline', exact: true }).check();
      await panel.getByRole('checkbox', { name: 'Bob · offline', exact: true }).check();
      await panel
        .getByRole('textbox', { name: 'Question', exact: true })
        .fill('Which boundary owns retries?');
      await expect(panel.getByRole('button', { name: 'Start new preview' })).toBeDisabled();
      expect(requestCounts(sandbox.database)).toEqual({ attempts: 0, operations: 0 });
      await panel.getByRole('button', { name: 'Close whiteboard', exact: true }).click();
      await page.getByRole('button', { name: 'Open whiteboard', exact: true }).click();
      await expect(panel.getByRole('textbox', { name: 'Question', exact: true })).toHaveValue(
        'Which boundary owns retries?'
      );
      await panel.getByRole('button', { name: 'Review request', exact: true }).click();
      const message = await panel.getByRole('textbox', { name: 'Exact request' }).inputValue();
      await expect(panel.getByRole('textbox', { name: 'Local snapshot reference' })).toHaveCount(1);
      expect(message).toContain(reference);
      await expect(
        panel.getByRole('list', { name: 'Confirmed recipients' }).getByRole('listitem')
      ).toHaveText(
        ['Alice', 'Bob'].sort((a, b) => {
          const ids: Record<string, string> = { Alice: alice.id, Bob: bob.id };
          return ids[a]!.localeCompare(ids[b]!);
        })
      );
      // A stale recipient must not silently retarget a same-name replacement.
      await cli(['rm', 'Bob', '--force']);
      const replacement = (await cli(['identity', 'create', 'Bob'])).identity;
      expect(replacement.id).not.toBe(bob.id);
      expect(requestCounts(sandbox.database)).toEqual({ attempts: 0, operations: 0 });
      await panel
        .getByRole('button', { name: 'Send request', exact: true })
        .scrollIntoViewIfNeeded();
      await expect(panel.getByRole('img', { name: 'Saved whiteboard snapshot' })).toBeInViewport();
      await page.screenshot({ path: testInfo.outputPath('whiteboard-send-review-desktop.png') });
      const sending = page.waitForResponse(
        (response) =>
          response.request().method() === 'POST' &&
          new URL(response.url()).pathname === '/api/v1/local/dispatch'
      );
      await panel.getByRole('button', { name: 'Send request', exact: true }).click();
      const sent = await sending;
      expect(sent.status()).toBe(200);
      const input: DispatchInput = sent.request().postDataJSON();
      const receipt: DispatchReceipt = await sent.json();
      expect(input.message).toBe(message);
      expect(input.recipientIds).toEqual([alice.id, bob.id].sort());
      expect(receipt.items).toEqual(
        input.recipientIds.map((recipientId) => ({
          recipientId,
          requestId: expect.stringMatching(/^req_/),
          acceptance: recipientId === alice.id ? 'queued' : 'recipientUnavailable',
        }))
      );
      await expect(panel.getByText('Request recorded.', { exact: false })).toBeVisible();
      await expect(panel.getByText('Unavailable · not queued', { exact: false })).toBeVisible();
      await expect(panel.getByRole('textbox', { name: 'Local snapshot reference' })).toHaveCount(1);
      expect(requestCounts(sandbox.database)).toEqual({ attempts: 2, operations: 1 });
      const address = new URL(started.url);
      const token = new URLSearchParams(address.hash.slice(1)).get('token')!;
      // Replay the exact browser envelope against the real host; no route interception.
      const replay = await page.request.post(`${address.origin}/api/v1/local/dispatch`, {
        headers: { Authorization: `Bearer ${token}`, Origin: address.origin },
        data: input,
      });
      expect(replay.status()).toBe(200);
      expect(await replay.json()).toEqual(receipt);
      expect(requestCounts(sandbox.database)).toEqual({ attempts: 2, operations: 1 });
      const aliceRequest = receipt.items.find((item) => item.recipientId === alice.id)!.requestId;
      const incoming = await cli([
        'x',
        'listen',
        '--identity',
        'Alice',
        '--timeout',
        '1s',
        '--debounce',
        '1ms',
      ]);
      expect(incoming).toMatchObject({
        reason: 'messages',
        identityId: alice.id,
        items: [
          { requestId: aliceRequest, kind: 'request', direction: 'incoming', delivery: 'queued' },
        ],
      });
      const detail = (await cli(['x', 'show', aliceRequest, '--incoming', '--identity', 'Alice']))
        .exchange;
      expect(detail.prompt.message).toBe(message);
      expect(detail.reply.receipt).toMatch(/^v2_/);
      const untouched = await cli([
        'x',
        'listen',
        '--identity',
        'Bob',
        '--timeout',
        '1ms',
        '--debounce',
        '1ms',
      ]);
      expect(untouched.items).toEqual([]);
      // The receiver discovers its locator from the actual inbox, not sender-side state.
      // Parse only the typed reference; never execute arbitrary inbox text as a command.
      const receivedReference = detail.prompt.message.match(
        /^Whiteboard snapshot: (tmt:whiteboard:snapshot:[0-9a-f-]+)$/m
      )?.[1];
      expect(receivedReference).toBe(reference);
      expect(detail.prompt.message).toContain(
        'If you cannot view images, review the structured content and say the image was not inspected.'
      );
      const captured = await office(['whiteboard', 'snapshot', 'show', receivedReference!]);
      expect(captured.annotation).toBe('Check the delivery boundary.');
      expect(captured.scene.elements).toEqual([
        expect.objectContaining({ text: 'Browser → inbox → review' }),
      ]);
      const output = path.join(sandbox.cwd, 'agent-review.png');
      await office(['whiteboard', 'snapshot', 'export', receivedReference!, '--output', output]);
      const receivedImage = await readFile(output);
      expect(receivedImage.equals(png)).toBe(true);
      // Decode exported bytes independently of the editor preview. This proves image
      // access/renderability, not a real model's ability to understand the drawing.
      const decoded = await page.evaluate(
        async (bytes) => {
          const image = await createImageBitmap(
            new Blob([new Uint8Array(bytes)], { type: 'image/png' })
          );
          try {
            const canvas = document.createElement('canvas');
            canvas.width = image.width;
            canvas.height = image.height;
            const context = canvas.getContext('2d')!;
            context.drawImage(image, 0, 0);
            const pixels = context.getImageData(0, 0, image.width, image.height).data;
            let highlighted = 0;
            for (let i = 0; i < pixels.length; i += 4)
              if (
                pixels[i] === 0 &&
                pixels[i + 1] === 140 &&
                pixels[i + 2] === 145 &&
                pixels[i + 3] === 255
              )
                highlighted++;
            return { width: image.width, height: image.height, highlighted };
          } finally {
            image.close();
          }
        },
        [...receivedImage]
      );
      expect(decoded).toMatchObject({ width: 1600, height: 1000 });
      expect(decoded.highlighted).toBeGreaterThan(20);
      await copyFile(output, testInfo.outputPath('whiteboard-send-agent-image.png'));
      const response = `The host owns request retries. Reviewed ${receivedReference}.`;
      await cli(['reply', aliceRequest, '--receipt', detail.reply.receipt, '--message', response]);
      expect(await cli(['result', aliceRequest])).toMatchObject({ status: 'completed', response });
      expect(requestCounts(sandbox.database)).toEqual({ attempts: 2, operations: 1 });
      await page.setViewportSize({ width: 430, height: 932 });
      await panel.getByRole('button', { name: 'Compose another request' }).scrollIntoViewIfNeeded();
      await page.screenshot({
        path: testInfo.outputPath('whiteboard-send-receipt-mobile.png'),
        fullPage: true,
      });
      expect(await panel.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(
        true
      );
      // The same composer can adopt a stored room, but never expands it after preview.
      await page.setViewportSize({ width: 1440, height: 1000 });
      await panel.getByRole('button', { name: 'Compose another request' }).click();
      await panel.getByRole('button', { name: 'Refresh agents' }).click();
      await panel.getByRole('radio', { name: 'Meeting room', exact: true }).check();
      await panel.getByRole('button', { name: 'Create room', exact: true }).click();
      const editor = panel.getByRole('form', { name: 'Meeting room editor' });
      await editor.getByRole('textbox', { name: 'Room name' }).fill('Design review');
      await editor.getByRole('checkbox', { name: 'Alice · offline', exact: true }).check();
      await expect(panel.getByRole('button', { name: 'Start new preview' })).toBeDisabled();
      await panel.getByRole('button', { name: 'Close whiteboard', exact: true }).click();
      await page.getByRole('button', { name: 'Open whiteboard', exact: true }).click();
      await expect(editor.getByRole('textbox', { name: 'Room name' })).toHaveValue('Design review');
      await page.setViewportSize({ width: 430, height: 932 });
      await editor.getByRole('button', { name: 'Save room', exact: true }).scrollIntoViewIfNeeded();
      await page.screenshot({ path: testInfo.outputPath('whiteboard-room-editor-mobile.png') });
      expect(await panel.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(
        true
      );
      await page.setViewportSize({ width: 1440, height: 1000 });
      const savingRoom = page.waitForResponse(
        (response) =>
          response.request().method() === 'PUT' &&
          new URL(response.url()).pathname.startsWith('/api/v1/local/rooms/')
      );
      await editor.getByRole('button', { name: 'Save room', exact: true }).click();
      const savedRoom = await savingRoom;
      expect(savedRoom.status()).toBe(200);
      const meeting = await savedRoom.json();
      expect(meeting).toMatchObject({ name: 'Design review', revision: 1, memberIds: [alice.id] });
      expect(requestCounts(sandbox.database)).toEqual({ attempts: 2, operations: 1 });
      await panel.getByRole('button', { name: 'Use this roster' }).click();
      // Valid long room labels must not widen the compact inspector on narrow screens.
      const longName = 'R'.repeat(80);
      const renamedRoom = await page.request.put(
        `${address.origin}/api/v1/local/rooms/${meeting.id}`,
        {
          headers: { Authorization: `Bearer ${token}`, Origin: address.origin },
          data: { expectedRevision: 1, name: longName, memberIds: [alice.id] },
        }
      );
      expect(renamedRoom.status()).toBe(200);
      await panel.getByRole('button', { name: 'Refresh rooms' }).click();
      await expect(
        panel
          .getByRole('combobox', { name: 'Meeting room' })
          .getByRole('option', { name: longName })
      ).toHaveCount(1);
      await panel.getByRole('button', { name: 'Use this roster' }).click();
      await page.setViewportSize({ width: 430, height: 932 });
      await panel.getByRole('combobox', { name: 'Meeting room' }).scrollIntoViewIfNeeded();
      expect(await panel.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(
        true
      );
      await page.screenshot({ path: testInfo.outputPath('whiteboard-room-long-name-mobile.png') });
      await page.setViewportSize({ width: 1440, height: 1000 });
      await panel
        .getByRole('textbox', { name: 'Question', exact: true })
        .fill('Review this together in the design room.');
      await panel.getByRole('button', { name: 'Review request', exact: true }).click();
      const changedRoom = await page.request.put(
        `${address.origin}/api/v1/local/rooms/${meeting.id}`,
        {
          headers: { Authorization: `Bearer ${token}`, Origin: address.origin },
          data: { expectedRevision: 2, name: meeting.name, memberIds: [alice.id, replacement.id] },
        }
      );
      expect(changedRoom.status()).toBe(200);
      const rejecting = page.waitForResponse(
        (response) =>
          response.request().method() === 'POST' &&
          new URL(response.url()).pathname === '/api/v1/local/dispatch'
      );
      await panel.getByRole('button', { name: 'Send request', exact: true }).click();
      const staleRoom = await rejecting;
      expect(staleRoom.status()).toBe(409);
      expect(await staleRoom.json()).toEqual({ error: 'ROOM_ROSTER_CHANGED' });
      expect(requestCounts(sandbox.database)).toEqual({ attempts: 2, operations: 1 });
      await expect(panel.getByRole('alert')).toContainText('The room changed');
      await expect(panel.getByRole('textbox', { name: 'Question', exact: true })).toHaveValue(
        'Review this together in the design room.'
      );
      await expect(
        panel.getByRole('button', { name: 'Review request', exact: true })
      ).toBeDisabled();
      await panel.getByRole('button', { name: 'Refresh rooms' }).click();
      await panel.getByRole('combobox', { name: 'Meeting room' }).selectOption(meeting.id);
      await expect(
        panel.getByRole('list', { name: 'Current room roster' }).getByRole('listitem')
      ).toHaveCount(2);
      await panel.getByRole('button', { name: 'Use this roster' }).click();
      await panel.getByRole('button', { name: 'Review request', exact: true }).click();
      await expect(panel.getByText('Design review · revision 3', { exact: true })).toBeVisible();
      await panel
        .getByRole('button', { name: 'Send request', exact: true })
        .scrollIntoViewIfNeeded();
      await page.screenshot({ path: testInfo.outputPath('whiteboard-room-review-desktop.png') });
      const sendingRoom = page.waitForResponse(
        (response) =>
          response.request().method() === 'POST' &&
          new URL(response.url()).pathname === '/api/v1/local/dispatch'
      );
      await panel.getByRole('button', { name: 'Send request', exact: true }).click();
      const sentRoom = await sendingRoom;
      expect(sentRoom.status()).toBe(200);
      const roomInput: DispatchInput = sentRoom.request().postDataJSON();
      expect(roomInput).toMatchObject({
        recipientIds: [alice.id, replacement.id].sort(),
        room: { kind: 'roster', roomId: meeting.id, revision: 3 },
      });
      const roomReceipt: DispatchReceipt = await sentRoom.json();
      expect(roomReceipt.items.map((item) => item.acceptance)).toEqual(['queued', 'queued']);
      const emptied = await page.request.put(`${address.origin}/api/v1/local/rooms/${meeting.id}`, {
        headers: { Authorization: `Bearer ${token}`, Origin: address.origin },
        data: { expectedRevision: 3, name: meeting.name, memberIds: [] },
      });
      expect(emptied.status()).toBe(200);
      const roomReplay = await page.request.post(`${address.origin}/api/v1/local/dispatch`, {
        headers: { Authorization: `Bearer ${token}`, Origin: address.origin },
        data: roomInput,
      });
      expect(roomReplay.status()).toBe(200);
      expect(await roomReplay.json()).toEqual(roomReceipt);
      expect(requestCounts(sandbox.database)).toEqual({ attempts: 4, operations: 2 });
      const bobRequest = roomReceipt.items.find(
        (item) => item.recipientId === replacement.id
      )!.requestId;
      const bobIncoming = await cli([
        'x',
        'listen',
        '--identity',
        'Bob',
        '--timeout',
        '1s',
        '--debounce',
        '1ms',
      ]);
      expect(bobIncoming.items).toEqual([
        expect.objectContaining({
          requestId: bobRequest,
          direction: 'incoming',
          delivery: 'queued',
        }),
      ]);
      expect(
        (await cli(['x', 'show', bobRequest, '--incoming', '--identity', 'Bob'])).exchange.prompt
          .message
      ).toBe(roomInput.message);
      // Clearing the roster after dispatch cannot revoke an accepted request or
      // merge separate recipients' replies into one room-wide result.
      for (const item of roomReceipt.items) {
        const name = item.recipientId === alice.id ? 'Alice' : 'Bob';
        const exchange = (
          await cli(['x', 'show', item.requestId, '--incoming', '--identity', name])
        ).exchange;
        expect(exchange.prompt.message).toBe(roomInput.message);
        expect(exchange.prompt.message).toContain(reference);
        const response = `${name} reviewed the room snapshot ${reference}.`;
        await cli([
          'reply',
          item.requestId,
          '--receipt',
          exchange.reply.receipt,
          '--message',
          response,
        ]);
        expect(await cli(['result', item.requestId])).toMatchObject({
          status: 'completed',
          response,
        });
      }
      expect(await cli(['result', aliceRequest])).toMatchObject({ status: 'completed', response });
      expect(requestCounts(sandbox.database)).toEqual({ attempts: 4, operations: 2 });
      expect(renderErrors).toEqual([]);
    } finally {
      await page.goto('about:blank');
      await office(['stop']);
      expect((await office(['status'])).service.running).toBe(false);
    }
  });
});
