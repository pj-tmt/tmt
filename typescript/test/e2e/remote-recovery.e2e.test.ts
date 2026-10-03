import { randomUUID } from 'node:crypto';
import { describe, expect, it } from 'vite-plus/test';
import { object, text } from '../support/remote-device.js';
import { dispatchIntent, RemoteOwner } from '../support/remote-owner.js';
import { expectJsonResult } from './cli-assertions.js';
import { withE2EFixture } from './harness.js';
import { requestAttempts } from './request-state-oracle.js';
import { installTmuxTrace } from './tmux-trace.js';

describe('Remote same-ID crash recovery (#1055 bullet 5)', () => {
  for (const phase of ['before', 'after'] as const) {
    it(`recovers after serve dies ${phase} actual core acceptance, with one delivery and joined cleanup`, async () => {
      await withE2EFixture(
        async (fixture) => {
          const identity = expectJsonResult(
            await fixture.runJsonCli<{ identity: { id: string } }>([
              'name',
              `remote-recovery-${phase}`,
              '--save',
            ])
          );
          const owner = new RemoteOwner(fixture);
          const trace = installTmuxTrace(fixture);
          try {
            await owner.start();
            const { device, paired } = await owner.pair();
            const session = await owner.session(device, paired);
            const operationId = randomUUID();
            const message = `Original intent across ${phase}-acceptance crash`;
            const intent = dispatchIntent(operationId, identity.identity.id, message);
            const initial = requestAttempts(fixture);
            const initialEvents = fixture.events();
            owner.armCrash(operationId, phase);
            trace.clear();
            // Observe rejection immediately, and join it after killing serve. No
            // SDK retry or fabricated core output is involved in the crash.
            const inFlight = session.append('dispatch.create', intent, operationId).then(
              (value) => ({ value, error: undefined }),
              (error) => ({ value: undefined, error })
            );
            expect(await owner.crashBarrier()).toMatchObject({ phase, operationId });
            let acceptedRequest: string | undefined;
            if (phase === 'before') {
              expect(requestAttempts(fixture)).toEqual(initial);
              expect(fixture.events()).toEqual(initialEvents);
              expect(
                owner.coreCalls().filter((call) => call.operation === 'dispatch.create')
              ).toEqual([expect.objectContaining({ operationId, corePid: null })]);
            } else {
              const receipt = owner.barrierCoreOutput();
              expect(receipt.operationId).toBe(operationId);
              const items = receipt.items as unknown[];
              expect(items).toHaveLength(1);
              acceptedRequest = text(object(items[0]).requestId);
              await fixture.waitForEvent(
                (e) => e.event === 'silent' && e.requestId === acceptedRequest
              );
              expect(
                requestAttempts(fixture).filter((row) => row.request_id === acceptedRequest)
              ).toEqual([
                expect.objectContaining({
                  recipient_identity_id: identity.identity.id,
                  message_text: `[remote: ${device.name}]\n${message}`,
                }),
              ]);
            }
            await owner.crash();
            expect((await inFlight).error).toBeDefined();
            await owner.assertRestartBlocked();
            await owner.finishCrash();
            await owner.start();
            const recovered = await owner.session(device, paired);
            // Observation never retries a send. The before-acceptance case needs
            // the caller's explicit identical dispatch using its ORIGINAL ID.
            const observed = await recovered.append('operation.show', { operationId });
            if (phase === 'before') {
              expect(observed).toEqual({ state: 'uncertain', operationId });
              expect(requestAttempts(fixture)).toEqual(initial);
              expect(fixture.events()).toEqual(initialEvents);
            } else {
              expect(observed).toEqual({
                state: 'accepted',
                operationId,
                requestId: acceptedRequest,
              });
            }
            const sent = await recovered.append('dispatch.create', intent, operationId);
            const requestId = text(sent.requestId);
            expect(sent).toEqual({ state: 'accepted', operationId, requestId });
            if (acceptedRequest !== undefined) expect(requestId).toBe(acceptedRequest);
            await fixture.waitForEvent((e) => e.event === 'silent' && e.requestId === requestId);
            const settled = requestAttempts(fixture);
            const events = fixture.events();
            expect(await recovered.append('dispatch.create', intent, operationId)).toEqual(sent);
            expect(await recovered.append('operation.show', { operationId })).toEqual(sent);
            expect(
              owner
                .coreCalls()
                .filter((call) => call.operation === 'dispatch.create' && call.corePid !== null)
            ).toHaveLength(1);
            expect(settled).toHaveLength(initial.length + 1);
            expect(settled.filter((row) => row.request_id === requestId)).toEqual([
              expect.objectContaining({
                recipient_identity_id: identity.identity.id,
                message_text: `[remote: ${device.name}]\n${message}`,
              }),
            ]);
            expect(
              fixture.events().filter((e) => e.event === 'request' && e.requestId === requestId)
            ).toHaveLength(1);
            expect(trace.commands().filter((command) => command === 'paste-buffer')).toHaveLength(
              1
            );
            expect(trace.commands().filter((command) => command === 'send-keys')).toHaveLength(1);
            expect(requestAttempts(fixture)).toEqual(settled);
            expect(fixture.events()).toEqual(events);
            await owner.stop();
            await owner.assertListenerClosed();
          } finally {
            await owner.dispose();
          }
        },
        { mode: 'silent' }
      );
    }, 45_000);
  }
});
