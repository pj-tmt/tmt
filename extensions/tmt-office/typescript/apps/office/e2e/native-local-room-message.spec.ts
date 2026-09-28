import { writeFileSync } from 'node:fs';
import path from 'node:path';
import Database from '../../../../../../typescript/test/support/sqlite-oracle.js';
import { expect, test } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { officeWorldFixture } from '../../../../../../typescript/test/support/office-world.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import { openOfficeDirectory } from './office-navigation.js';
import type { DispatchInput, DispatchReceipt } from '../src/local/dispatch-contract.js';

function storedRequests(databasePath: string) {
  const database = new Database(databasePath, { readonly: true });
  try {
    return database
      .prepare(
        'SELECT request_id AS requestId, recipient_identity_id AS recipientId, room_id AS roomId FROM request_attempts ORDER BY recipient_identity_id'
      )
      .all();
  } finally {
    database.close();
  }
}

test('Message room reviews the real roster and recovers committed fan-out without resending', async ({
  page,
}, testInfo) => {
  page.setDefaultTimeout(10_000);
  await withSandbox(async (sandbox) => {
    const prefix = await installNativeOffice(sandbox);
    const cli = async (args: string[]) => {
      const result = await runCli(sandbox, [...args, '--json'], { deadlineMs: 30_000 });
      expect(result.status, result.stdout + result.stderr).toBe(0);
      return JSON.parse(result.stdout);
    };
    const office = (args: string[]) => cli(['office', '--prefix', prefix, ...args]);
    const alice = (await cli(['identity', 'create', 'Alice'])).identity;
    const bob = (await cli(['identity', 'create', 'Bob'])).identity;
    await cli(['identity', 'create', 'Outsider']);
    const room = (await cli(['room', 'create', 'Design'])).room;
    await cli(['room', 'join', room.id, '--identity', 'Alice']);
    const initial = await office(['layout', 'show']);
    const base = officeWorldFixture().layout;
    const area = {
      id: '70000000-0000-4000-8000-000000000001',
      name: 'Design',
      binding: { type: 'meeting', roomId: room.id },
    };
    const file = path.join(sandbox.root, 'room-message-world.json');
    writeFileSync(
      file,
      JSON.stringify({
        ...base,
        objects: [],
        map: {
          ...base.map,
          areas: [...base.map.areas, area],
          floor: [
            ...base.map.floor,
            ...Array.from({ length: 36 }, (_, y) => ({ y, start: 36, end: 72, areaId: area.id })),
          ],
          doors: [{ x: 36, y: 16, axis: 'vertical' }],
        },
      })
    );
    await office([
      'layout',
      'apply',
      '--file',
      file,
      '--if-revision',
      '0',
      '--legacy-basis',
      initial.legacyBasis,
    ]);
    const started = await office(['start', '--port', String(await unusedLoopbackPort())]);
    const panel = page.getByRole('dialog', { name: 'Message room', exact: true });
    const open = async () => {
      await openOfficeDirectory(page);
      await page.getByRole('button', { name: /^Design · meeting/ }).click();
      await page.getByRole('button', { name: 'Message room', exact: true }).click();
    };
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    page.on('dialog', (dialog) => void dialog.accept());
    const sentInputs: DispatchInput[] = [];
    page.on('request', (request) => {
      if (
        new URL(request.url()).pathname === '/api/v1/local/dispatch' &&
        request.method() === 'POST'
      )
        sentInputs.push(request.postDataJSON());
    });
    try {
      await page.setViewportSize({ width: 1440, height: 1000 });
      await page.goto(started.url);
      await open();
      const message = panel.getByRole('textbox', { name: 'Message', exact: true });
      await message.fill('Review the room plan together.');
      await expect(panel.getByRole('button', { name: 'Review request' })).toBeDisabled();
      await expect(panel.getByRole('combobox', { name: 'Meeting room' })).toBeDisabled();
      await panel.getByRole('button', { name: 'Close message room' }).click();
      await open();
      await expect(message).toHaveValue('Review the room plan together.');
      await panel.getByRole('button', { name: 'Use this roster' }).click();
      await panel.getByRole('button', { name: 'Review request' }).click();
      await expect(panel.getByRole('list', { name: 'Confirmed recipients' })).toHaveText('Alice');
      expect(storedRequests(sandbox.database)).toEqual([]);
      const changed = (await cli(['room', 'join', room.id, '--identity', 'Bob'])).room;
      const staleResponse = page.waitForResponse(
        (response) => new URL(response.url()).pathname === '/api/v1/local/dispatch'
      );
      await panel.getByRole('button', { name: 'Send request', exact: true }).click();
      expect((await staleResponse).status()).toBe(409);
      await expect(panel.getByRole('alert')).toContainText('The room changed');
      expect(storedRequests(sandbox.database)).toEqual([]);
      await expect(panel.getByRole('button', { name: 'Review request' })).toBeDisabled();
      await panel.getByRole('button', { name: 'Refresh rooms' }).click();
      await expect(
        panel.getByRole('list', { name: 'Current room roster' }).getByRole('listitem')
      ).toHaveCount(2);
      await panel.getByRole('button', { name: 'Use this roster' }).click();
      await panel.getByRole('button', { name: 'Review request' }).click();
      await expect(
        panel.getByRole('list', { name: 'Confirmed recipients' }).getByRole('listitem')
      ).toHaveText(
        [alice, bob].sort((a, b) => a.id.localeCompare(b.id)).map((identity) => identity.name)
      );
      expect((await panel.boundingBox())!.width).toBeLessThanOrEqual(620);
      await page.screenshot({ path: testInfo.outputPath('room-request-review-desktop.png') });
      await page.setViewportSize({ width: 430, height: 932 });
      expect(await panel.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(
        true
      );
      await page.screenshot({ path: testInfo.outputPath('room-request-review-narrow.png') });
      let receipt: DispatchReceipt | undefined;
      // Commit through the real host, then lose only the HTTP response.
      await page.route(
        '**/api/v1/local/dispatch',
        async (route) => {
          const response = await route.fetch();
          expect(response.status()).toBe(200);
          receipt = await response.json();
          await route.abort('failed');
        },
        { times: 1 }
      );
      await panel.getByRole('button', { name: 'Send request', exact: true }).click();
      await expect(panel.getByRole('button', { name: 'Retry send' })).toBeEnabled();
      expect(receipt?.items).toHaveLength(2);
      expect(sentInputs).toHaveLength(2);
      expect(sentInputs[1]).toMatchObject({
        recipientIds: [alice.id, bob.id].sort(),
        room: { kind: 'roster', roomId: room.id, revision: changed.revision },
      });
      expect(sentInputs[1]!.operationId).not.toBe(sentInputs[0]!.operationId);
      const expectedRows = receipt!.items.map((item) => ({
        requestId: item.requestId,
        recipientId: item.recipientId,
        roomId: room.id,
      }));
      expect(storedRequests(sandbox.database)).toEqual(expectedRows);
      // Recovery must not replace the frozen audience with the now-smaller roster.
      await cli(['room', 'leave', room.id, '--identity', 'Bob']);
      await page.goto('about:blank');
      await page.goto(started.url);
      await open();
      await expect(panel.getByText('Request recorded.', { exact: false })).toBeVisible();
      expect(sentInputs).toHaveLength(2);
      expect(storedRequests(sandbox.database)).toEqual(expectedRows);
      expect(
        await page.evaluate(() =>
          Object.keys(sessionStorage).filter((key) =>
            key.startsWith('tmt.office.pending-room-request.')
          )
        )
      ).toEqual([]);
      for (const identity of [alice, bob]) {
        const item = receipt!.items.find((item) => item.recipientId === identity.id)!;
        const incoming = await cli([
          'x',
          'listen',
          '--identity',
          identity.name,
          '--room',
          room.id,
          '--timeout',
          '1s',
          '--debounce',
          '1ms',
        ]);
        expect(incoming.items.map((entry: { requestId: string }) => entry.requestId)).toEqual([
          item.requestId,
        ]);
        const exchange = (
          await cli(['x', 'show', item.requestId, '--incoming', '--identity', identity.name])
        ).exchange;
        expect(exchange.prompt.message).toBe('Review the room plan together.');
        const response = `${identity.name} reviewed the plan.`;
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
      expect(
        (
          await cli([
            'x',
            'listen',
            '--identity',
            'Outsider',
            '--timeout',
            '1ms',
            '--debounce',
            '1ms',
          ])
        ).items
      ).toEqual([]);
      expect(errors).toEqual([]);
    } finally {
      await page.goto('about:blank');
      await office(['stop']);
    }
  });
});
