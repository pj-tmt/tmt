import { randomUUID } from 'node:crypto';
import { describe, expect, it } from 'vite-plus/test';
import { assertRemoteDeviceVectors, text } from '../support/remote-device.js';
import { RemoteOwner } from '../support/remote-owner.js';
import { expectJsonResult } from './cli-assertions.js';
import { withE2EFixture } from './harness.js';
import { requestAttempts } from './request-state-oracle.js';
import { installTmuxTrace } from './tmux-trace.js';

// #1055 acceptance only. C/D-dependent bullets are added when those runtimes
// are available; this suite has no skipped or stubbed application acceptance.
describe('Remote owner-device operations (#1055)', () => {
  it('bullet 4: refuses replay and out-of-order dispatch before core or pane effects', async () => {
    assertRemoteDeviceVectors();
    await withE2EFixture(async (fixture) => {
      const identity = expectJsonResult(
        await fixture.runJsonCli<{ identity: { id: string } }>([
          'name',
          'remote-sequence-agent',
          '--save',
        ])
      );
      const owner = new RemoteOwner(fixture);
      const trace = installTmuxTrace(fixture);
      try {
        await owner.start();
        const { device, paired } = await owner.pair();
        const session = await owner.session(device, paired);
        const before = requestAttempts(fixture);
        const coreBefore = owner.coreCalls();
        trace.clear();
        const eventsBefore = fixture.events();
        const operationId = randomUUID();
        const dispatch = {
          version: 1,
          operation: 'dispatch.create',
          originator: 'anonymous',
          input: {
            operationId,
            recipientIds: [identity.identity.id],
            message: 'This out-of-order intent must never reach the agent',
            kind: 'request',
          },
        };
        const future = session.envelope('dispatch.create', dispatch, {
          id: operationId,
          sequence: '2',
        });
        expect(await session.exchange('append', future)).toMatchObject({
          error: { code: 'REMOTE_REPLAY' },
        });

        // The expected sequence remains usable: an independently verified
        // machine response and empty cursor are the positive wire control.
        const subscribe = session.envelope(
          'subscribe',
          { cursor: null, limit: 1, waitMs: 0 },
          { kind: 'control' }
        );
        const page = await session.exchange('subscribe', subscribe);
        expect(page).toMatchObject({ entries: [], reason: 'timeout', hasMore: false });
        const cursor = text(page.nextCursor);
        expect(cursor.length).toBeGreaterThan(0);
        expect(await session.exchange('subscribe', subscribe)).toMatchObject({
          error: { code: 'REMOTE_REPLAY' },
        });
        const stale = session.envelope('dispatch.create', dispatch, {
          id: operationId,
          sequence: '1',
        });
        expect(await session.exchange('append', stale)).toMatchObject({
          error: { code: 'REMOTE_REPLAY' },
        });

        const ack = session.envelope('ack', { cursor }, { kind: 'control' });
        const acknowledged = await session.exchange('ack', ack);
        expect(acknowledged).toEqual({ cursor });
        const after = await session.exchange(
          'subscribe',
          session.envelope('subscribe', { cursor, limit: 1, waitMs: 0 }, { kind: 'control' })
        );
        expect(after).toMatchObject({
          entries: [],
          nextCursor: cursor,
          reason: 'timeout',
          hasMore: false,
        });
        expect(requestAttempts(fixture)).toEqual(before);
        expect(owner.coreCalls()).toEqual(coreBefore);
        expect(fixture.events()).toEqual(eventsBefore);
        expect(trace.commands()).not.toContain('paste-buffer');
        expect(trace.commands()).not.toContain('send-keys');
      } finally {
        await owner.dispose();
      }
    });
  }, 30_000);
});
