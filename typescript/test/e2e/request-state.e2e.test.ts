import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { E2EFixture, withE2EFixture } from './harness.js';
import { requestAttempts as attempts, preambleCounters } from './request-state-oracle.js';

interface TalkOutput {
  status: string;
  requestId: string;
  response?: string;
  error?: { code: string; stage?: string };
}

function cadence(fixture: E2EFixture): number {
  const counts = Object.values(preambleCounters(fixture));
  expect(counts).toHaveLength(1);
  return counts[0];
}

describe('transactional live request bookkeeping', { concurrent: false }, () => {
  it('keeps overlapping same-pane waits independent through timeout and interruption', async () => {
    await withE2EFixture(
      async (fixture) => {
        expect((await fixture.runJsonCli(['name', 'Receiver'])).code).toBe(0);
        const legacyPath = path.join(fixture.globalDir, 'state.json');
        const legacyBytes = '{"requests":{"%0":{"id":"old"}},"future":{"keep":true}}\n';
        fs.writeFileSync(legacyPath, legacyBytes);
        const first = fixture.runCliProcess<TalkOutput>([
          '--json',
          'talk',
          'Receiver',
          'first independent wait',
          '--timeout',
          '8',
        ]);
        const firstEvent = await fixture.waitForEvent(
          (event) => event.event === 'silent' && event.message === 'first independent wait',
          5_000
        );
        const second = fixture.runCliProcess<TalkOutput>([
          '--json',
          'talk',
          fixture.pane,
          'second independent wait',
          '--timeout',
          '20',
        ]);
        const secondEvent = await fixture.waitForEvent(
          (event) => event.event === 'silent' && event.message === 'second independent wait',
          5_000
        );
        expect(firstEvent.pid).toBe(fixture.panePid);
        expect(secondEvent.pid).toBe(fixture.panePid);
        expect(firstEvent.requestId).not.toBe(secondEvent.requestId);
        await fixture.waitFor(
          () =>
            attempts(fixture).filter((row) => row.wait_active === 1 && row.wake_state === 'sent')
              .length === 2
        );
        const before = attempts(fixture);
        expect(new Set(before.map((row) => row.request_id)).size).toBe(2);
        expect(before.map((row) => row.request_id).sort()).toEqual(
          [firstEvent.requestId, secondEvent.requestId].sort()
        );
        for (const row of before) {
          expect(row).toMatchObject({
            route_kind: 'inbox',
            wake_state: 'sent',
            status: 'queued',
          });
          expect(row.recipient_identity_id).toBeTruthy();
          expect(row.recipient_identity_id).toBe(row.originator_identity_id);
        }

        const timedOut = await first.result;
        expect(timedOut).toMatchObject({
          code: 4,
          json: {
            status: 'timeout',
            requestId: firstEvent.requestId,
            error: { code: 'TIMEOUT', message: expect.stringContaining('Timed out') },
          },
        });
        expect(timedOut.stderr).toBe('');
        const timeoutOutput = JSON.parse(timedOut.stdout);
        expect(timeoutOutput).toMatchObject({
          requestId: firstEvent.requestId,
          pane: fixture.pane,
        });
        expect(timeoutOutput).not.toHaveProperty('partialResponse');
        const afterFirst = attempts(fixture);
        expect(afterFirst.find((row) => row.request_id === firstEvent.requestId)).toMatchObject({
          wait_active: 0,
          status: 'queued',
          wake_state: 'sent',
        });
        expect(afterFirst.find((row) => row.request_id === secondEvent.requestId)).toMatchObject({
          wait_active: 1,
          status: 'queued',
          wake_state: 'sent',
        });
        second.kill('SIGINT');
        expect((await second.result).code).toBe(1);
        expect(attempts(fixture)).toHaveLength(2);
        expect(
          attempts(fixture).every(
            (row) => row.wait_active === 0 && row.status === 'queued' && row.wake_state === 'sent'
          )
        ).toBe(true);
        expect(fs.readFileSync(legacyPath, 'utf8')).toBe(legacyBytes);
      },
      { mode: 'silent' }
    );
  }, 25_000);

  it('separates identical pane IDs on two private servers sharing one database', async () => {
    await withE2EFixture(
      async (firstServer) => {
        const secondServer = new E2EFixture({ globalDir: firstServer.globalDir });
        try {
          await secondServer.start({ mode: 'silent' });
          expect(firstServer.pane).toBe(secondServer.pane);
          // Unbound direct-pane routes retain endpoint fencing. Identified
          // requests instead freeze the recipient UUID in their Inbox route.
          const first = firstServer.runCliProcess([
            '--json',
            'talk',
            firstServer.pane,
            'server one wait',
            '--timeout',
            '20',
          ]);
          const firstEvent = await firstServer.waitForEvent(
            (event) => event.event === 'silent' && event.message === 'server one wait',
            5_000
          );
          const second = secondServer.runCliProcess([
            '--json',
            'talk',
            secondServer.pane,
            'server two wait',
            '--timeout',
            '20',
          ]);
          const secondEvent = await secondServer.waitForEvent(
            (event) => event.event === 'silent' && event.message === 'server two wait',
            5_000
          );
          await firstServer.waitFor(
            () => attempts(firstServer).filter((row) => row.wait_active === 1).length === 2
          );
          const rows = attempts(firstServer);
          expect(rows).toHaveLength(2);
          expect(new Set(rows.map((row) => row.server_id)).size).toBe(2);
          expect(new Set(rows.map((row) => row.socket_path)).size).toBe(2);
          expect(rows.find((row) => row.request_id === firstEvent.requestId)).toMatchObject({
            pane_pid: firstServer.panePid,
            socket_path: firstServer.socketPath,
          });
          expect(rows.find((row) => row.request_id === secondEvent.requestId)).toMatchObject({
            pane_pid: secondServer.panePid,
            socket_path: secondServer.socketPath,
          });
          first.kill('SIGINT');
          expect((await first.result).code).toBe(1);
          expect(
            attempts(firstServer).find((row) => row.request_id === secondEvent.requestId)
              ?.wait_active
          ).toBe(1);
          second.kill('SIGINT');
          expect((await second.result).code).toBe(1);
          expect(attempts(firstServer).every((row) => row.wait_active === 0)).toBe(true);
        } finally {
          await secondServer.stop();
        }
      },
      { mode: 'silent' }
    );
  }, 25_000);

  it('consumes uncertain submit cadence once and preserves the next actual payload', async () => {
    await withE2EFixture(async (fixture) => {
      expect((await fixture.runJsonCli(['name', 'Receiver'])).code).toBe(0);
      expect(
        (await fixture.runJsonCli(['preamble', 'set', 'Receiver', 'Review carefully.'])).code
      ).toBe(0);
      expect((await fixture.runJsonCli(['config', 'set', 'preambleEvery', '3'])).code).toBe(0);
      const uncertain = await fixture.runJsonCli<TalkOutput>(
        ['talk', 'RECEIVER', 'uncertain first input', '--timeout', '8'],
        // Keep the trace specific to request transport, not a separate
        // originator hint after this intentionally interrupted delivery.
        { transportFault: { stage: 'submit' }, outsideTmux: true }
      );
      expect(uncertain).toMatchObject({
        code: 1,
        json: { error: { code: 'DELIVERY_UNCERTAIN', stage: 'submit' } },
      });
      const firstEvent = await fixture.waitForEvent(
        (event) =>
          event.event === 'submitted' &&
          event.message === '[SYSTEM: Review carefully.]\nuncertain first input'
      );
      expect(firstEvent.pid).toBe(fixture.panePid);
      expect(attempts(fixture)).toHaveLength(1);
      expect(attempts(fixture)[0]).toMatchObject({
        status: 'queued',
        route_kind: 'inbox',
        wake_state: 'uncertain',
        wait_active: 0,
        inject_preamble: 1,
        request_id: firstEvent.requestId,
      });
      expect(cadence(fixture)).toBe(1);
      const next = await fixture.runJsonCli<TalkOutput>([
        'talk',
        fixture.pane,
        'second actual input',
        '--timeout',
        '8',
      ]);
      expect(next).toMatchObject({ code: 0, json: { status: 'completed' } });
      const nextEvent = await fixture.waitForEvent(
        (event) =>
          event.event === 'submitted' &&
          event.requestId === next.json?.requestId &&
          event.pid === fixture.panePid
      );
      expect(nextEvent.message).toBe('second actual input');
      expect(cadence(fixture)).toBe(2);
      expect(attempts(fixture).find((row) => row.request_id === nextEvent.requestId)).toMatchObject(
        {
          status: 'queued',
          route_kind: 'inbox',
          wake_state: 'sent',
          wait_active: 0,
          inject_preamble: 0,
        }
      );
      expect(fixture.events().filter((event) => event.event === 'request')).toHaveLength(2);
      expect(
        fixture.transportTrace().filter((line) => line.startsWith('submit.before'))
      ).toHaveLength(1);
    });
  }, 20_000);
});
