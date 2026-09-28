import { randomUUID } from 'node:crypto';
import { expect, test } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import { openOfficeObjects } from './office-navigation.js';
import { installationWorldPoint } from './native-world-geometry.js';
import type { DispatchInput } from '../src/local/dispatch-contract.js';

test('Office announcements reuse the real inbox without offering a reply or changing request semantics', async ({
  request,
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
    await cli(['identity', 'create', 'Bob']);
    const started = await office(['start', '--port', String(await unusedLoopbackPort())]);
    try {
      const address = new URL(started.url);
      const token = new URLSearchParams(address.hash.slice(1)).get('token')!;
      const post = (data: unknown) =>
        request.post(`${address.origin}/api/v1/local/dispatch`, {
          headers: { Authorization: `Bearer ${token}`, Origin: address.origin },
          data,
        });
      await page.setViewportSize({ width: 1440, height: 1000 });
      await page.goto(started.url);
      await expect(
        page.getByRole('button', { name: 'Compose announcement', exact: true })
      ).toHaveCount(0);
      await expect(page.getByRole('complementary', { name: 'Layout tools' })).toBeVisible();
      await expect(page.getByRole('button', { name: 'Edit layout', exact: true })).toHaveCount(0);
      await expect(page.getByRole('button', { name: 'Save layout', exact: true })).toHaveCount(0);
      await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
      const canvas = page.locator('.office-canvas canvas');
      const bounds = (await canvas.boundingBox())!;
      // Radio footprint [76,51,8,8] projects upright to [76,43.625,8,8].
      const station = installationWorldPoint(bounds, 96, 61.625);
      await page.mouse.move(station.x, station.y);
      await expect(canvas).toHaveCSS('cursor', 'grab');
      const action = installationWorldPoint(bounds, 96, 56.5);
      await page.mouse.move(action.x, action.y);
      await expect(canvas).toHaveCSS('cursor', 'pointer');
      await page.screenshot({ path: testInfo.outputPath('broadcaster-lobby-desktop.png') });
      await page.mouse.click(action.x, action.y);
      const panel = page.getByRole('dialog', { name: 'Broadcast station', exact: true });
      const review = panel.getByRole('button', { name: 'Review announcement', exact: true });
      await expect(review).toBeDisabled();
      const message = '  Office update\nA new whiteboard is ready.';
      await panel.getByRole('textbox', { name: 'Message', exact: true }).fill(message);
      await expect(review).toBeDisabled();
      await panel.getByRole('checkbox', { name: 'Alice · offline', exact: true }).check();
      await expect(
        panel.getByRole('checkbox', { name: 'Bob · offline', exact: true })
      ).not.toBeChecked();
      await panel.getByRole('button', { name: 'Close broadcast station' }).click();
      await openOfficeObjects(page);
      await page.getByRole('button', { name: 'Compose announcement', exact: true }).click();
      await expect(panel.getByRole('textbox', { name: 'Message', exact: true })).toHaveValue(
        message
      );
      await review.click();
      await expect(
        panel.getByRole('list', { name: 'Confirmed recipients' }).getByRole('listitem')
      ).toHaveText(['Alice']);
      await expect(panel.getByRole('textbox', { name: 'Exact announcement' })).toHaveValue(message);
      const beforeSend = await cli([
        'x',
        'listen',
        '--identity',
        'Alice',
        '--timeout',
        '1s',
        '--debounce',
        '1ms',
      ]);
      expect(beforeSend.items).toEqual([]);
      await page.screenshot({ path: testInfo.outputPath('broadcaster-review-desktop.png') });
      await page.setViewportSize({ width: 390, height: 844 });
      const panelBounds = await panel.evaluate((element) => ({
        width: element.clientWidth,
        content: element.scrollWidth,
        right: element.getBoundingClientRect().right,
      }));
      expect(panelBounds.content).toBeLessThanOrEqual(panelBounds.width);
      expect(panelBounds.right).toBeLessThanOrEqual(390);
      await page.screenshot({ path: testInfo.outputPath('broadcaster-review-narrow.png') });
      const sending = page.waitForResponse(
        (response) =>
          response.request().method() === 'POST' &&
          new URL(response.url()).pathname === '/api/v1/local/dispatch'
      );
      await panel.getByRole('button', { name: 'Send announcement', exact: true }).click();
      const sent = await sending;
      const input = sent.request().postDataJSON() as DispatchInput;
      expect(input).toMatchObject({ kind: 'announcement', recipientIds: [alice.id], message });
      expect(sent.status()).toBe(200);
      const receipt = await sent.json();
      const requestId = receipt.items[0].requestId;
      expect(receipt.items).toEqual([{ recipientId: alice.id, requestId, acceptance: 'queued' }]);
      await expect(panel.getByRole('status')).toContainText('Announcement recorded.');
      await expect(
        panel.getByText('No reply is requested. Recipients acknowledge after reading.')
      ).toBeVisible();
      await expect(panel.getByText(/tmt result/)).toHaveCount(0);
      expect(
        (await cli(['x', 'listen', '--identity', 'Bob', '--timeout', '1s', '--debounce', '1ms']))
          .items
      ).toEqual([]);
      const replay = await post(input);
      expect(replay.status()).toBe(200);
      expect(await replay.json()).toEqual(receipt);
      expect((await post({ ...input, kind: 'request' })).status()).toBe(409);
      const listen = () =>
        cli(['x', 'listen', '--identity', 'Alice', '--timeout', '1s', '--debounce', '1ms']);
      const incoming = await listen();
      expect(incoming.items).toEqual([
        expect.objectContaining({
          requestId,
          kind: 'announcement',
          finalStatus: 'not_required',
          settled: false,
          inspectCommand: `tmt x show ${requestId} --incoming --identity alice`,
          ackCommand: `tmt x ack ${requestId} --incoming --revision 1 --identity alice`,
        }),
      ]);
      const show = () => cli(['x', 'show', requestId, '--incoming', '--identity', 'Alice']);
      const detail = (await show()).exchange;
      expect(detail.prompt.message).toBe(input.message);
      expect(detail.final.status).toBe('not_required');
      expect(detail).not.toHaveProperty('reply');
      expect(await cli(['result', requestId])).toEqual({ status: 'not_required', requestId });
      // Reading both projections is not an acknowledgment and does not enqueue again.
      expect((await listen()).items).toEqual(incoming.items);
      await cli(['x', 'ack', requestId, '--incoming', '--identity', 'Alice', '--revision', '1']);
      expect((await show()).exchange).toMatchObject({ acknowledged: true, settled: true });
      expect((await listen()).items).toEqual([]);

      // Omitted kind is still an ordinary replyable request through the same endpoint.
      const ordinary = await post({
        operationId: randomUUID(),
        recipientIds: [alice.id],
        message: 'Please review.',
      });
      expect(ordinary.status()).toBe(200);
      const ordinaryId = (await ordinary.json()).items[0].requestId;
      const ordinaryDetail = (
        await cli(['x', 'show', ordinaryId, '--incoming', '--identity', 'Alice'])
      ).exchange;
      expect(ordinaryDetail.final.status).toBe('not_submitted');
      expect(ordinaryDetail.reply.receipt).toMatch(/^v2_/);
      await cli([
        'reply',
        ordinaryId,
        '--receipt',
        ordinaryDetail.reply.receipt,
        '--message',
        'Reviewed.',
      ]);
      expect(await cli(['result', ordinaryId])).toMatchObject({
        status: 'completed',
        response: 'Reviewed.',
      });
    } finally {
      await office(['stop']);
      expect((await office(['status'])).service.running).toBe(false);
    }
  });
});
