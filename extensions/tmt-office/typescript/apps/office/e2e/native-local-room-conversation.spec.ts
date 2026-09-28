import { writeFileSync } from 'node:fs';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { officeWorldFixture } from '../../../../../../typescript/test/support/office-world.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import { openOfficeDirectory } from './office-navigation.js';
import type { DispatchReceipt } from '../src/local/dispatch-contract.js';
import type { WorldDocument } from '../src/world-map/world-contract.js';

test('meeting private messages keep recipient, history and recovery scoped to one member and room', async ({
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
    const room = async (args: string[]) => (await cli(['room', ...args])).room;
    const alice = (await cli(['identity', 'create', 'Alice'])).identity;
    await cli(['identity', 'create', 'Bob']);
    const design = await room(['create', 'Design']);
    const planning = await room(['create', 'Planning']);
    for (const meeting of [design, planning])
      for (const name of ['Alice', 'Bob']) await room(['join', meeting.id, '--identity', name]);

    const initial = await office(['layout', 'show']);
    const base = officeWorldFixture().layout;
    const meetings = [design, planning].map((meeting, index) => ({
      id: `70000000-0000-4000-8000-00000000000${index + 1}`,
      name: meeting.name as string,
      binding: { type: 'meeting' as const, roomId: meeting.id as string },
    }));
    const layout: WorldDocument = {
      ...base,
      objects: [],
      map: {
        ...base.map,
        areas: [...base.map.areas, ...meetings],
        floor: [
          ...base.map.floor,
          ...meetings.flatMap((area, index) =>
            Array.from({ length: 36 }, (_, y) => ({
              y,
              start: (index + 1) * 36,
              end: (index + 2) * 36,
              areaId: area.id,
            }))
          ),
        ],
        doors: meetings.map((_, index) => ({ x: (index + 1) * 36, y: 16, axis: 'vertical' })),
      },
    };
    const file = path.join(sandbox.root, 'conversation-world.json');
    writeFileSync(file, JSON.stringify(layout));
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
    const panel = page.getByRole('dialog', { name: 'Agent conversation', exact: true });
    const message = panel.getByRole('textbox', { name: 'Message', exact: true });
    const open = async (name: string) => {
      await openOfficeDirectory(page);
      await page.getByRole('button', { name: new RegExp(`^${name} · meeting`) }).click();
      await page
        .getByRole('region', { name: 'Area roster' })
        .getByRole('button', { name: 'Alice · Offline', exact: true })
        .click();
      if (
        !(await panel.getByRole('region', { name: 'Switch conversation confirmation' }).isVisible())
      )
        await panel.getByRole('tab', { name: 'Chat', exact: true }).click();
    };
    const close = () => panel.getByRole('button', { name: 'Close agent conversation' }).click();
    const send = async (text: string) => {
      await message.fill(text);
      const response = page.waitForResponse(
        (response) => new URL(response.url()).pathname === '/api/v1/local/dispatch'
      );
      await panel.getByRole('button', { name: 'Send', exact: true }).click();
      const accepted = await response;
      expect(accepted.status()).toBe(200);
      expect(accepted.request().postDataJSON()).toMatchObject({
        recipientIds: [alice.id],
        room: { kind: 'direct', roomId: design.id },
        message: text,
      });
      return (await accepted.json()) as DispatchReceipt;
    };
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    try {
      await page.setViewportSize({ width: 1440, height: 1000 });
      await page.goto(started.url);
      await open('Design');
      await expect(
        panel.getByText('Room: Design. Only Alice receives this message.')
      ).toBeVisible();
      const receipt = await send('Review privately in Design.');
      expect(receipt.items).toHaveLength(1);
      const requestId = receipt.items[0]!.requestId;
      const exchange = (await cli(['x', 'show', requestId, '--incoming', '--identity', 'Alice']))
        .exchange;
      expect(exchange.roomId).toBe(design.id);
      await cli([
        'reply',
        requestId,
        '--receipt',
        exchange.reply.receipt,
        '--message',
        'Design reviewed.',
      ]);
      await expect(panel.locator('.conversation-reply')).toHaveText('Design reviewed.');

      // Even the same identity needs confirmation before switching a dirty room draft.
      await message.fill('Do not silently move this draft to Planning.');
      await close();
      await open('Planning');
      await expect(
        panel.getByRole('heading', { name: 'Open conversation with Alice in Planning?' })
      ).toBeVisible();
      await panel.getByRole('button', { name: 'Keep this conversation' }).click();
      await expect(message).toHaveValue('Do not silently move this draft to Planning.');
      await close();
      await open('Planning');
      await panel.getByRole('button', { name: 'Switch conversation', exact: true }).click();
      await expect(
        panel.getByText('Room: Planning. Only Alice receives this message.')
      ).toBeVisible();
      await expect(panel.getByText('Start a conversation with Alice.')).toBeVisible();
      await expect(panel.locator('.chat-outgoing pre')).toHaveCount(0);
      await expect(message).toHaveValue('');
      await close();
      await open('Design');
      await expect(panel.locator('.conversation-reply')).toHaveText('Design reviewed.');

      // The host commits the exact request; only its HTTP response is lost.
      await page.route(
        '**/api/v1/local/dispatch',
        async (route) => {
          expect((await route.fetch()).status()).toBe(200);
          await route.abort('failed');
        },
        { times: 1 }
      );
      await message.fill('Recover this Design request once.');
      await panel.getByRole('button', { name: 'Send', exact: true }).click();
      await expect(panel.getByRole('button', { name: 'Retry', exact: true })).toBeEnabled();
      await page.goto('about:blank');
      await page.goto(started.url);
      await open('Planning');
      await expect(panel.getByText('Start a conversation with Alice.')).toBeVisible();
      await close();
      await open('Design');
      await expect(message).toHaveValue('');
      await expect(panel.locator('.chat-outgoing pre')).toHaveText([
        'Review privately in Design.',
        'Recover this Design request once.',
      ]);
      expect(
        await page.evaluate(() =>
          Object.keys(sessionStorage).filter((key) => key.startsWith('tmt.office.pending-request.'))
        )
      ).toEqual([]);
      const incoming = await cli([
        'x',
        'listen',
        '--identity',
        'Alice',
        '--room',
        design.id,
        '--timeout',
        '1s',
        '--debounce',
        '1ms',
      ]);
      expect(incoming.items).toHaveLength(2);
      expect(
        new Set(incoming.items.map((item: { requestId: string }) => item.requestId)).size
      ).toBe(2);
      expect(
        (await cli(['x', 'listen', '--identity', 'Bob', '--timeout', '1ms', '--debounce', '1ms']))
          .items
      ).toEqual([]);
      await page.screenshot({ path: testInfo.outputPath('direct-room-conversation.png') });
      expect(errors).toEqual([]);
    } finally {
      await page.goto('about:blank');
      await office(['stop']);
      expect((await office(['status'])).service.running).toBe(false);
    }
  });
});
