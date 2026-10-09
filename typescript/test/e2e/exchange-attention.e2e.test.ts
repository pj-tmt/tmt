import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import { expectJsonResult } from './cli-assertions.js';
import { withE2EFixture, type E2EFixture } from './harness.js';
import { requestAttempts } from './request-state-oracle.js';

interface PublicIdentity {
  readonly id: string;
  readonly name: string;
  readonly canonicalName: string;
}

interface TalkResult {
  readonly status?: string;
  readonly requestId?: string;
  readonly error?: { readonly code?: string };
}

function malformedConfig(fixture: E2EFixture): { global: string; local: string } {
  const global = '{ malformed exchange config';
  const local = '{ malformed local exchange config';
  fs.writeFileSync(path.join(fixture.globalDir, 'config.json'), global);
  fs.writeFileSync(path.join(fixture.workspace, 'tmt.json'), local);
  return { global, local };
}

async function calibrateOfflineTmuxGuard(fixture: E2EFixture): Promise<void> {
  const calibration = await fixture.runJsonCli(['check', fixture.pane], { withoutTmux: true });
  expect(calibration.code).toBe(3);
  expect(calibration.json).toMatchObject({ error: { code: 'PANE_NOT_FOUND' } });
  expect(fs.existsSync(fixture.forbiddenTmuxLogPath)).toBe(true);
  fs.rmSync(fixture.forbiddenTmuxLogPath, { force: true });
}

function processGroupIsRunning(pid: number): boolean {
  try {
    process.kill(-pid, 0);
    return true;
  } catch (error) {
    if (error instanceof Error && 'code' in error && error.code === 'ESRCH') return false;
    throw error;
  }
}

function processWaitsInInterruptPoll(pid: number): boolean {
  try {
    return /poll/i.test(fs.readFileSync(`/proc/${pid}/wchan`, 'utf8').trim());
  } catch (error) {
    if (error instanceof Error && 'code' in error && error.code === 'ENOENT') return false;
    throw error;
  }
}

function processHasOpenFile(pid: number, file: string): boolean {
  try {
    return fs
      .readdirSync(`/proc/${pid}/fd`)
      .some((descriptor) => fs.readlinkSync(`/proc/${pid}/fd/${descriptor}`) === file);
  } catch (error) {
    if (error instanceof Error && 'code' in error && error.code === 'ENOENT') return false;
    throw error;
  }
}

function listenerWaitIsReady(fixture: E2EFixture, pid: number): boolean {
  if (!processGroupIsRunning(pid)) {
    throw new Error('Listener exited before reaching its interrupt wait.');
  }
  return (
    processHasOpenFile(pid, path.join(fixture.globalDir, 'tmux-team.db')) &&
    processWaitsInInterruptPoll(pid)
  );
}

describe('Exchange attention through the real Docker/tmux fixture', { concurrent: false }, () => {
  it('ackall works before any list, late finals reopen attention, and explicit/implicit identity access survives rebind', async () => {
    await withE2EFixture(
      async (fixture) => {
        const created = expectJsonResult(
          await fixture.runJsonCli<{ identity: PublicIdentity; created: boolean }>(
            ['identity', 'create', 'Alice'],
            { withoutTmux: true }
          )
        );
        expect(created.created).toBe(true);
        await calibrateOfflineTmuxGuard(fixture);

        const detached = expectJsonResult(
          await fixture.runJsonCli<TalkResult>([
            'talk',
            fixture.pane,
            'late attention final',
            '--identity',
            'alice',
            '--no-preamble',
            '--detach',
          ])
        );
        expect(detached).toMatchObject({
          status: 'sent',
          requestId: expect.stringMatching(/^req_/),
        });
        const requestId = detached.requestId!;
        await fixture.waitForEvent(
          (event) => event.event === 'request' && event.requestId === requestId
        );

        // The acknowledgement is intentionally the first exchange read: ackall must use only
        // the identity watermark and cannot depend on a preceding list or body lookup.
        const acknowledged = expectJsonResult(
          await fixture.runJsonCli<{ identity: PublicIdentity; acknowledgedThrough: number }>(
            ['x', 'ackall', '--identity', 'alice'],
            { withoutTmux: true }
          )
        );
        expect(acknowledged.identity).toEqual(created.identity);
        expect(acknowledged.acknowledgedThrough).toBeGreaterThan(0);

        const configs = malformedConfig(fixture);
        fixture.releaseReplyGate(requestId);
        const submitted = await fixture.waitForEvent(
          (event) => event.event === 'submitted' && event.requestId === requestId,
          5_000
        );
        expect(submitted.body).toBe('mock-agent response: late attention final');

        const reopened = expectJsonResult<{
          identity: PublicIdentity;
          items: Array<Record<string, unknown>>;
          nextAfter: number | null;
        }>(await fixture.runJsonCli(['x', 'list', '--identity', 'alice'], { withoutTmux: true }));
        expect(reopened.identity).toEqual(created.identity);
        expect(reopened.items).toEqual([
          expect.objectContaining({
            requestId,
            acknowledged: false,
            final: {
              status: 'retained',
              bodyBytes: Buffer.byteLength(submitted.body!),
              submittedAtMs: expect.any(Number),
              expiresAtMs: expect.any(Number),
            },
          }),
        ]);
        expect(JSON.stringify(reopened)).not.toContain('attemptId');
        expect(JSON.stringify(reopened)).not.toContain('socketPath');

        const exact = expectJsonResult<{
          identity: PublicIdentity;
          exchange: { final: { status: string; response?: string; bodyBytes?: number } };
        }>(
          await fixture.runJsonCli(['x', 'show', requestId, '--identity', 'alice'], {
            withoutTmux: true,
          })
        );
        expect(exact.identity).toEqual(created.identity);
        expect(exact.exchange.final).toEqual({
          status: 'retained',
          response: submitted.body,
          bodyBytes: Buffer.byteLength(submitted.body!),
          submittedAtMs: expect.any(Number),
          expiresAtMs: expect.any(Number),
        });
        expect(fs.existsSync(fixture.forbiddenTmuxLogPath)).toBe(false);

        // Bind the same durable identity and prove omitted --identity uses verified caller
        // evidence. The explicit offline path above never touched tmux or parsed config.
        const bindWithValidSettings = async (name: string, pane: string): Promise<void> => {
          // Binding now validates badge configuration. Repair only for that operation,
          // then restore malformed settings before each implicit exchange read below.
          expect(fs.readFileSync(path.join(fixture.globalDir, 'config.json'), 'utf8')).toBe(
            configs.global
          );
          expect(fs.readFileSync(path.join(fixture.workspace, 'tmt.json'), 'utf8')).toBe(
            configs.local
          );
          fs.writeFileSync(path.join(fixture.globalDir, 'config.json'), '{}');
          fs.writeFileSync(path.join(fixture.workspace, 'tmt.json'), '{}');
          expect(expectJsonResult(await fixture.runJsonCli(['name', name]))).toEqual({
            bound: true,
            id: created.identity.id,
            lifetime: 'saved',
            name: 'Alice',
            pane,
          });
          expect(malformedConfig(fixture)).toEqual(configs);
        };
        await bindWithValidSettings('Alice', fixture.pane);
        const implicit = expectJsonResult<{ items: Array<Record<string, unknown>> }>(
          await fixture.runJsonCli(['x', 'list'])
        );
        expect(implicit.items.some((item) => item.requestId === requestId)).toBe(true);

        const restarted = await fixture.restartServer();
        await bindWithValidSettings('alice', restarted.pane);
        const afterRebind = expectJsonResult<{ identity: PublicIdentity }>(
          await fixture.runJsonCli(['x', 'show', requestId])
        );
        expect(afterRebind.identity).toEqual(created.identity);

        expect(fs.readFileSync(path.join(fixture.globalDir, 'config.json'), 'utf8')).toBe(
          configs.global
        );
        expect(fs.readFileSync(path.join(fixture.workspace, 'tmt.json'), 'utf8')).toBe(
          configs.local
        );
      },
      { replyGate: true }
    );
  }, 45_000);
  it('recovers a timed-out explicit exchange and shows the exact virtualized final body without reading the terminal', async () => {
    await withE2EFixture(
      async (fixture) => {
        const created = expectJsonResult(
          await fixture.runJsonCli<{ identity: PublicIdentity; created: boolean }>(
            ['identity', 'create', 'VirtualizedOwner'],
            { withoutTmux: true }
          )
        );
        expect(created.created).toBe(true);
        await calibrateOfflineTmuxGuard(fixture);
        const token = 'exchange-virtualized-final-🙂-\u65e5\u672c\u8a9e';
        const expectedBody = [
          `VIRTUALIZED-BEGIN:${token}`,
          ...Array.from(
            { length: 200 },
            (_, index) => `VIRTUALIZED-LINE-${String(index + 1).padStart(3, '0')}:${token}`
          ),
          `VIRTUALIZED-END:${token}`,
        ].join('\n');

        const timedOut = await fixture.runJsonCli<TalkResult>([
          'talk',
          fixture.pane,
          token,
          '--identity',
          'virtualizedowner',
          '--no-preamble',
          '--timeout',
          '1',
        ]);
        expect(timedOut.code).toBe(4);
        expect(timedOut.json).toMatchObject({
          status: 'timeout',
          requestId: expect.stringMatching(/^req_[0-9a-f-]+$/),
          error: { code: 'TIMEOUT' },
        });
        const requestId = timedOut.json?.requestId;
        expect(requestId).toBeDefined();
        expect(
          fixture
            .events()
            .some((event) => event.event === 'submitted' && event.requestId === requestId)
        ).toBe(false);
        fixture.releaseReplyGate(requestId);
        const submitted = await fixture.waitForEvent(
          (event) => event.event === 'submitted' && event.requestId === requestId,
          5_000
        );
        expect(submitted.body).toBe(expectedBody);

        const listed = expectJsonResult<{
          items: Array<{
            requestId: string;
            revision: number;
            acknowledged: boolean;
            final: Record<string, unknown>;
          }>;
        }>(
          await fixture.runJsonCli(['x', 'list', '--identity', 'virtualizedowner'], {
            withoutTmux: true,
          })
        );
        const item = listed.items.find((candidate) => candidate.requestId === requestId);
        expect(item).toMatchObject({
          requestId,
          acknowledged: false,
          final: { status: 'retained', bodyBytes: Buffer.byteLength(expectedBody) },
        });
        expect(item?.revision).toBeGreaterThan(1);

        const acked = expectJsonResult<{ changed: boolean; revision: number }>(
          await fixture.runJsonCli(
            [
              'x',
              'ack',
              requestId!,
              '--identity',
              'virtualizedowner',
              '--revision',
              String(item!.revision),
            ],
            { withoutTmux: true }
          )
        );
        expect(acked).toMatchObject({ changed: true, revision: item!.revision });

        const shown = expectJsonResult<{
          exchange: {
            acknowledged: boolean;
            settled: boolean;
            final: { status: string; response?: string; bodyBytes?: number };
          };
        }>(
          await fixture.runJsonCli(['x', 'show', requestId!, '--identity', 'virtualizedowner'], {
            withoutTmux: true,
          })
        );
        expect(shown.exchange.acknowledged).toBe(true);
        expect(shown.exchange.settled).toBe(true);
        expect(shown.exchange.final).toEqual({
          status: 'retained',
          response: expectedBody,
          bodyBytes: Buffer.byteLength(expectedBody),
          submittedAtMs: expect.any(Number),
          expiresAtMs: expect.any(Number),
        });
        expect(fs.existsSync(fixture.forbiddenTmuxLogPath)).toBe(false);
        expect(fixture.capture()).not.toContain(`VIRTUALIZED-LINE-100:${token}`);
      },
      { mode: 'virtualized', replyGate: true }
    );
  }, 30_000);

  it('queues, listens, inspects, replies and retrieves an inbox request without tmux or Office', async () => {
    await withE2EFixture(async (fixture) => {
      const sender = expectJsonResult<{ identity: PublicIdentity }>(
        await fixture.runJsonCli(['identity', 'create', 'Inbox Sender'], { withoutTmux: true })
      ).identity;
      const receiver = expectJsonResult<{ identity: PublicIdentity }>(
        await fixture.runJsonCli(['identity', 'create', 'Inbox Receiver'], { withoutTmux: true })
      ).identity;
      await calibrateOfflineTmuxGuard(fixture);

      const queued = expectJsonResult<{
        status: string;
        requestId: string;
        recipientIdentityId: string;
      }>(
        await fixture.runJsonCli(
          [
            'talk',
            receiver.canonicalName,
            'docker inbox review',
            '--inbox',
            '--identity',
            sender.canonicalName,
            '--detach',
          ],
          { withoutTmux: true }
        )
      );
      expect(queued).toMatchObject({
        status: 'queued',
        recipientIdentityId: receiver.id,
      });
      expect(queued).not.toHaveProperty('pane');

      const incoming = expectJsonResult<{
        reason: string;
        identityId: string;
        items: Array<{
          requestId: string;
          revision: number;
          kind: string;
          delivery: string;
          inspectCommand: string;
        }>;
      }>(
        await fixture.runJsonCli(
          [
            'x',
            'listen',
            '--identity',
            receiver.canonicalName,
            '--timeout',
            '1s',
            '--debounce',
            '1ms',
          ],
          { withoutTmux: true }
        )
      );
      expect(incoming).toMatchObject({
        reason: 'messages',
        identityId: receiver.id,
        items: [
          {
            requestId: queued.requestId,
            revision: 1,
            kind: 'request',
            delivery: 'queued',
            inspectCommand: `tmt x show ${queued.requestId} --incoming --identity 'inbox receiver'`,
          },
        ],
      });
      expect(JSON.stringify(incoming)).not.toContain('docker inbox review');
      expect(JSON.stringify(incoming)).not.toContain('receipt');

      const detail = expectJsonResult<{
        exchange: {
          prompt: { message: string };
          reply: { receipt: string; command: string };
        };
      }>(
        await fixture.runJsonCli(
          ['x', 'show', queued.requestId, '--incoming', '--identity', receiver.canonicalName],
          { withoutTmux: true }
        )
      );
      expect(detail.exchange.prompt.message).toBe('docker inbox review');
      expect(detail.exchange.reply.receipt).toMatch(/^v2_/);

      expectJsonResult(
        await fixture.runJsonCli(
          [
            'reply',
            queued.requestId,
            '--receipt',
            detail.exchange.reply.receipt,
            '--message',
            'docker review complete',
          ],
          { withoutTmux: true }
        )
      );
      expect(
        expectJsonResult<{ status: string; response: string }>(
          await fixture.runJsonCli(['result', queued.requestId], { withoutTmux: true })
        )
      ).toMatchObject({ status: 'completed', response: 'docker review complete' });

      const resultNotice = expectJsonResult<{
        reason: string;
        identityId: string;
        items: Array<{ requestId: string; kind: string; revision: number }>;
      }>(
        await fixture.runJsonCli(
          [
            'x',
            'listen',
            '--identity',
            sender.canonicalName,
            '--timeout',
            '1s',
            '--debounce',
            '1ms',
          ],
          { withoutTmux: true }
        )
      );
      expect(resultNotice).toMatchObject({
        reason: 'messages',
        identityId: sender.id,
        items: [{ requestId: queued.requestId, kind: 'response', revision: 2 }],
      });
      expect(JSON.stringify(resultNotice)).not.toContain('docker review complete');
      expect(fs.existsSync(fixture.forbiddenTmuxLogPath)).toBe(false);
      expect(fs.existsSync(path.join(fixture.globalDir, 'office', 'service.json'))).toBe(false);
    });
  }, 30_000);

  it('wakes an already-running empty-inbox listener for exactly the separately queued request', async () => {
    await withE2EFixture(async (fixture) => {
      const sender = expectJsonResult<{ identity: PublicIdentity }>(
        await fixture.runJsonCli(['identity', 'create', 'Wake Sender'], { withoutTmux: true })
      ).identity;
      const receiver = expectJsonResult<{ identity: PublicIdentity }>(
        await fixture.runJsonCli(['identity', 'create', 'Wake Receiver'], { withoutTmux: true })
      ).identity;
      const listener = fixture.runCliProcess<{
        reason: string;
        identityId: string;
        items: Array<{ requestId: string; revision: number; kind: string }>;
        nextAfter: number | null;
      }>(
        [
          '--json',
          'x',
          'listen',
          '--identity',
          receiver.canonicalName,
          '--timeout',
          '5s',
          '--debounce',
          '1ms',
        ],
        { withoutTmux: true }
      );
      await fixture.waitFor(
        () => listenerWaitIsReady(fixture, listener.pid),
        2_000,
        'empty-inbox listener interrupt wait readiness'
      );

      const queued = expectJsonResult<{ requestId: string }>(
        await fixture.runJsonCli(
          [
            'talk',
            receiver.canonicalName,
            'wake this exact listener',
            '--inbox',
            '--identity',
            sender.canonicalName,
            '--detach',
          ],
          { withoutTmux: true }
        )
      );
      const result = await listener.result;
      expect(result.code).toBe(0);
      expect(result.stderr).toBe('');
      expect(result.json).toEqual({
        reason: 'messages',
        identityId: receiver.id,
        items: [
          expect.objectContaining({ requestId: queued.requestId, revision: 1, kind: 'request' }),
        ],
        nextAfter: null,
      });
      await fixture.waitFor(
        () => !processGroupIsRunning(listener.pid),
        2_000,
        'completed listener process cleanup'
      );
    });
  }, 15_000);

  it('keeps a published inbox request replyable after its sender interrupts the wait', async () => {
    await withE2EFixture(async (fixture) => {
      await calibrateOfflineTmuxGuard(fixture);
      const sender = expectJsonResult<{ identity: PublicIdentity }>(
        await fixture.runJsonCli(['identity', 'create', 'Waiting Sender'], { withoutTmux: true })
      ).identity;
      const receiver = expectJsonResult<{ identity: PublicIdentity }>(
        await fixture.runJsonCli(['identity', 'create', 'Offline Receiver'], { withoutTmux: true })
      ).identity;
      const message = 'Review the frozen whiteboard snapshot.\nDo not cancel when I stop waiting.';
      const waiting = fixture.runCliProcess(
        [
          '--json',
          'talk',
          receiver.canonicalName,
          message,
          '--inbox',
          '--identity',
          sender.canonicalName,
          '--timeout',
          '120',
        ],
        { withoutTmux: true }
      );
      await fixture.waitFor(
        () =>
          requestAttempts(fixture).some(
            (row) => row.status === 'queued' && row.wait_active === 1
          ) && processWaitsInInterruptPoll(waiting.pid),
        2_000,
        'queued request and interruptible sender observation'
      );
      const before = requestAttempts(fixture);
      expect(before).toHaveLength(1);
      const requestId = before[0]!.request_id;
      expect(before[0]).toMatchObject({
        status: 'queued',
        wait_active: 1,
        message_text: message,
        originator_identity_id: sender.id,
        recipient_identity_id: receiver.id,
      });
      waiting.kill('SIGINT');
      const interrupted = await waiting.result;
      expect(interrupted.code).toBe(1);
      expect(interrupted.stderr).toBe('');
      expect(interrupted.json).toMatchObject({ requestId, error: { code: 'INTERRUPTED' } });
      await fixture.waitFor(
        () => !processGroupIsRunning(waiting.pid),
        2_000,
        'sender process cleanup'
      );
      const after = requestAttempts(fixture);
      expect(after).toHaveLength(1);
      expect(after[0]).toEqual({
        ...before[0],
        wait_active: 0,
        wait_released_at_ms: expect.any(Number),
      });

      const incoming = expectJsonResult<{
        items: Array<{ requestId: string; acknowledged: boolean }>;
      }>(
        await fixture.runJsonCli(
          [
            'x',
            'listen',
            '--identity',
            receiver.canonicalName,
            '--timeout',
            '1s',
            '--debounce',
            '1ms',
          ],
          { withoutTmux: true }
        )
      );
      expect(incoming.items).toEqual([expect.objectContaining({ requestId, acknowledged: false })]);
      const detail = expectJsonResult<{
        exchange: { prompt: { message: string }; reply: { receipt: string } };
      }>(
        await fixture.runJsonCli(
          ['x', 'show', requestId, '--incoming', '--identity', receiver.canonicalName],
          { withoutTmux: true }
        )
      );
      expect(detail.exchange.prompt.message).toBe(message);
      expectJsonResult(
        await fixture.runJsonCli(
          [
            'reply',
            requestId,
            '--receipt',
            detail.exchange.reply.receipt,
            '--message',
            'Reviewed after you left.',
          ],
          { withoutTmux: true }
        )
      );
      expect(
        expectJsonResult(await fixture.runJsonCli(['result', requestId], { withoutTmux: true }))
      ).toMatchObject({ status: 'completed', response: 'Reviewed after you left.' });
      expect(fs.existsSync(fixture.forbiddenTmuxLogPath)).toBe(false);
    });
  }, 15_000);

  it('interrupts a listener without acknowledging or mutating its queued request', async () => {
    await withE2EFixture(async (fixture) => {
      const sender = expectJsonResult<{ identity: PublicIdentity }>(
        await fixture.runJsonCli(['identity', 'create', 'Interrupt Sender'], {
          withoutTmux: true,
        })
      ).identity;
      const receiver = expectJsonResult<{ identity: PublicIdentity }>(
        await fixture.runJsonCli(['identity', 'create', 'Interrupt Receiver'], {
          withoutTmux: true,
        })
      ).identity;
      const queued = expectJsonResult<{ requestId: string }>(
        await fixture.runJsonCli(
          [
            'talk',
            receiver.canonicalName,
            'remain unread after interrupt',
            '--inbox',
            '--identity',
            sender.canonicalName,
            '--detach',
          ],
          { withoutTmux: true }
        )
      );
      const before = requestAttempts(fixture).find((row) => row.request_id === queued.requestId);
      expect(before).toMatchObject({ status: 'queued', wait_active: 0 });
      const listener = fixture.runCliProcess(
        [
          '--json',
          'x',
          'listen',
          '--identity',
          receiver.canonicalName,
          '--timeout',
          '120s',
          '--debounce',
          '120s',
        ],
        { withoutTmux: true }
      );
      await fixture.waitFor(
        () => listenerWaitIsReady(fixture, listener.pid),
        2_000,
        'interruptible listener wait readiness'
      );
      listener.kill('SIGINT');
      const interrupted = await listener.result;
      expect(interrupted.code).toBe(1);
      expect(interrupted.stderr).toBe('');
      expect(interrupted.json).toMatchObject({ error: { code: 'INTERRUPTED' } });
      await fixture.waitFor(
        () => !processGroupIsRunning(listener.pid),
        2_000,
        'interrupted listener process cleanup'
      );
      expect(requestAttempts(fixture).find((row) => row.request_id === queued.requestId)).toEqual(
        before
      );
      const remaining = expectJsonResult<{
        items: Array<{ requestId: string; acknowledged: boolean }>;
      }>(
        await fixture.runJsonCli(
          [
            'x',
            'listen',
            '--identity',
            receiver.canonicalName,
            '--timeout',
            '1s',
            '--debounce',
            '1ms',
          ],
          { withoutTmux: true }
        )
      );
      expect(remaining.items).toEqual([
        expect.objectContaining({ requestId: queued.requestId, acknowledged: false }),
      ]);
    });
  }, 15_000);

  it('ends a waiting retired identity and keeps same-name replacement inbox isolated', async () => {
    await withE2EFixture(async (fixture) => {
      const sender = expectJsonResult<{ identity: PublicIdentity }>(
        await fixture.runJsonCli(['identity', 'create', 'Retire Sender'], { withoutTmux: true })
      ).identity;
      const original = expectJsonResult<{ identity: PublicIdentity }>(
        await fixture.runJsonCli(['identity', 'create', 'Replace Listener'], {
          withoutTmux: true,
        })
      ).identity;
      const listener = fixture.runCliProcess(
        [
          '--json',
          'x',
          'listen',
          '--identity',
          original.canonicalName,
          '--timeout',
          '120s',
          '--debounce',
          '10s',
        ],
        { withoutTmux: true }
      );
      await fixture.waitFor(
        () => listenerWaitIsReady(fixture, listener.pid),
        2_000,
        'retirable listener wait readiness'
      );
      expectJsonResult(
        await fixture.runJsonCli(['rm', original.canonicalName, '--force'], { withoutTmux: true })
      );
      const replacement = expectJsonResult<{ identity: PublicIdentity }>(
        await fixture.runJsonCli(['identity', 'create', 'Replace Listener'], {
          withoutTmux: true,
        })
      ).identity;
      expect(replacement.id).not.toBe(original.id);
      const retired = await listener.result;
      expect(retired.code).toBe(3);
      expect(retired.stderr).toBe('');
      expect(retired.json).toMatchObject({ error: { code: 'IDENTITY_RETIRED' } });
      await fixture.waitFor(
        () => !processGroupIsRunning(listener.pid),
        2_000,
        'retired listener process cleanup'
      );

      const queued = expectJsonResult<{ requestId: string }>(
        await fixture.runJsonCli(
          [
            'talk',
            replacement.canonicalName,
            'replacement-only request',
            '--inbox',
            '--identity',
            sender.canonicalName,
            '--detach',
          ],
          { withoutTmux: true }
        )
      );
      const replacementInbox = expectJsonResult<{
        identityId: string;
        items: Array<{ requestId: string }>;
      }>(
        await fixture.runJsonCli(
          [
            'x',
            'listen',
            '--identity',
            replacement.canonicalName,
            '--timeout',
            '1s',
            '--debounce',
            '1ms',
          ],
          { withoutTmux: true }
        )
      );
      expect(replacementInbox).toMatchObject({
        identityId: replacement.id,
        items: [{ requestId: queued.requestId }],
      });
    });
  }, 15_000);
});
