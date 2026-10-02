import { describe, expect, it } from 'vite-plus/test';
import { expectJsonResult } from './cli-assertions.js';
import { withE2EFixture, type E2EFixture, type MockEvent } from './harness.js';
import { preambleCounters, requestAttempts } from './request-state-oracle.js';
import { durableState } from './identity-state-oracle.js';

interface TalkOutput {
  requestId?: string;
  target?: string;
  pane?: string;
  identity?: { name: string; canonicalName: string };
  status?: string;
  response?: string;
  bodyBytes?: number;
  submittedAtMs?: number;
  error?: { code: string; message: string; stage?: string };
}

async function nativeRequest(fixture: E2EFixture, requestId: string): Promise<MockEvent> {
  const request = await fixture.waitForEvent(
    (event) => event.event === 'request' && event.requestId === requestId,
    5_000
  );
  expect(request.receipt).toEqual(expect.stringMatching(/^v2_[A-Za-z0-9_-]{22}$/));
  expect(request.receipt).toHaveLength(25);
  await fixture.waitForEvent(
    (event) => event.event === 'child-start' && event.requestId === requestId,
    5_000
  );
  return request;
}

async function submittedBody(
  fixture: E2EFixture,
  requestId: string,
  body: string
): Promise<MockEvent> {
  const child = await fixture.waitForEvent(
    (event) => event.event === 'child-start' && event.requestId === requestId,
    5_000
  );
  expect(child?.replyInput).toBe('stdin');
  const submitted = await fixture.waitForEvent(
    (event) => event.event === 'submitted' && event.requestId === requestId,
    5_000
  );
  expect(submitted.body).toBe(body);
  expect(submitted.bodyBytes).toBe(Buffer.byteLength(body));
  return submitted;
}

describe('public talk completion and observer lifecycle', { concurrent: false }, () => {
  it.each(['completed', 'detached', 'timeout'] as const)(
    'keeps human %s output correlated with the durable request and exact final',
    async (mode) => {
      const body = '  human final\r\nsecond line  ';
      await withE2EFixture(
        async (fixture) => {
          const args = [
            'talk',
            fixture.pane,
            `human-${mode}`,
            '--no-preamble',
            ...(mode === 'detached' ? ['--detach'] : ['--timeout', mode === 'timeout' ? '1' : '8']),
          ];
          const result = await fixture.runCli(args);
          expect(result.code, result.stderr || result.stdout).toBe(mode === 'timeout' ? 4 : 0);
          const attempts = requestAttempts(fixture);
          expect(attempts).toHaveLength(1);
          const requestId = attempts[0]!.request_id;
          expect(requestId).toMatch(/^req_[0-9a-f-]+$/);
          expect(attempts[0]).toMatchObject({
            message_text: `human-${mode}`,
            status: 'sent',
            wait_active: 0,
          });
          if (mode === 'timeout') {
            expect(result.stdout).toBe('');
            expect(result.stderr).toBe(
              `error: Timed out waiting for ${fixture.pane} after 1s\nhint: Inspect with 'tmt result ${requestId}' and 'tmt check ${fixture.pane}' before deciding whether to retry\n`
            );
          } else {
            expect(result.stderr).toBe('');
            expect(result.stdout).toBe(
              (mode === 'completed'
                ? `✓ Completed request ${requestId} for ${fixture.pane} (${fixture.pane})\n${body}\n`
                : `✓ Sent request ${requestId} to ${fixture.pane} (${fixture.pane})\n`) +
                `hint: retrieve it later with tmt result ${requestId}\n`
            );
          }
          if (mode !== 'completed') {
            expect(fixture.events().filter((event) => event.event === 'submitted')).toEqual([]);
            fixture.releaseReplyGate(requestId);
          }
          await submittedBody(fixture, requestId, body);
          expect(
            expectJsonResult(
              await fixture.runJsonCli<TalkOutput>(['result', requestId], { withoutTmux: true })
            )
          ).toMatchObject({
            requestId,
            status: 'completed',
            response: body,
            bodyBytes: Buffer.byteLength(body),
          });
        },
        {
          replyGate: mode !== 'completed',
          responseBodyBase64: Buffer.from(body).toString('base64'),
        }
      );
    },
    15_000
  );

  it.each([
    ['empty', ''],
    ['whitespace', ' \t  \n\r\n '],
    ['Unicode and marker-like text', '\uFEFF日本語🙂\u0000\r\nRESPONSE-END-fake-marker'],
  ])(
    'returns the exact %s nested native reply',
    async (_label, body) => {
      await withE2EFixture(
        async (fixture) => {
          const talk = expectJsonResult(
            await fixture.runJsonCli<TalkOutput>([
              'talk',
              fixture.pane,
              'exact native reply',
              '--no-preamble',
              '--timeout',
              '8',
            ])
          );
          expect(talk).toMatchObject({
            status: 'completed',
            response: body,
            bodyBytes: Buffer.byteLength(body),
            requestId: expect.stringMatching(/^req_[0-9a-f-]+$/),
          });
          const request = await nativeRequest(fixture, talk.requestId ?? '');
          expect(request.message).toBe('exact native reply');
          const submitted = await submittedBody(fixture, talk.requestId ?? '', body);
          expect(submitted.submittedAtMs).toBe(talk.submittedAtMs);

          const result = expectJsonResult(
            await fixture.runJsonCli<TalkOutput>(['result', talk.requestId ?? ''], {
              withoutTmux: true,
            })
          );
          expect(result).toEqual({
            status: 'completed',
            requestId: talk.requestId,
            response: body,
            bodyBytes: Buffer.byteLength(body),
            submittedAtMs: talk.submittedAtMs,
          });
        },
        {
          responseBodyBase64: Buffer.from(body, 'utf8').toString('base64'),
          replyInput: 'stdin',
        }
      );
    },
    20_000
  );

  it('supports detach followed by storage-only result through the native nested reply', async () => {
    const body = 'detached 日本語\r\nreply';
    await withE2EFixture(
      async (fixture) => {
        const sent = expectJsonResult(
          await fixture.runJsonCli<TalkOutput>([
            'talk',
            fixture.pane,
            'detached native request',
            '--no-preamble',
            '--detach',
          ])
        );
        expect(sent).toMatchObject({
          status: 'sent',
          requestId: expect.stringMatching(/^req_[0-9a-f-]+$/),
          target: fixture.pane,
          pane: fixture.pane,
        });
        expect(sent).not.toHaveProperty('response');
        await nativeRequest(fixture, sent.requestId ?? '');
        await submittedBody(fixture, sent.requestId ?? '', body);

        const result = expectJsonResult(
          await fixture.runJsonCli<TalkOutput>(['result', sent.requestId ?? ''], {
            withoutTmux: true,
          })
        );
        expect(result).toMatchObject({
          status: 'completed',
          requestId: sent.requestId,
          response: body,
          bodyBytes: Buffer.byteLength(body),
        });
      },
      {
        responseBodyBase64: Buffer.from(body, 'utf8').toString('base64'),
        replyInput: 'stdin',
      }
    );
  }, 15_000);

  it('returns timeout while retaining a late native final for result', async () => {
    const body = 'late native final';
    await withE2EFixture(
      async (fixture) => {
        const process = fixture.runCliProcess<TalkOutput>([
          '--json',
          'talk',
          fixture.pane,
          'native timeout request',
          '--no-preamble',
          '--timeout',
          '1',
        ]);
        const timedOut = await process.result;
        expect(timedOut.code).toBe(4);
        expect(timedOut.stderr).toBe('');
        expect(timedOut.json).toMatchObject({
          status: 'timeout',
          requestId: expect.stringMatching(/^req_[0-9a-f-]+$/),
          error: { code: 'TIMEOUT' },
        });
        expect(timedOut.json).not.toHaveProperty('response');

        const request = await fixture.waitForEvent(
          (event) => event.event === 'request' && event.requestId === timedOut.json?.requestId,
          5_000
        );
        expect(request.receipt).toEqual(expect.stringMatching(/^v2_[A-Za-z0-9_-]{22}$/));
        expect(
          fixture
            .events()
            .some(
              (event) =>
                (event.event === 'child-start' || event.event === 'submitted') &&
                event.requestId === timedOut.json?.requestId
            )
        ).toBe(false);
        await fixture.waitFor(
          () => {
            const attempt = requestAttempts(fixture).find(
              (item) => item.request_id === timedOut.json?.requestId
            );
            return attempt?.status === 'sent' && attempt.wait_active === 0;
          },
          5_000,
          'native timeout waiter cleanup'
        );
        expect(
          fixture
            .events()
            .some(
              (event) =>
                (event.event === 'child-start' || event.event === 'submitted') &&
                event.requestId === timedOut.json?.requestId
            )
        ).toBe(false);
        fixture.releaseReplyGate(timedOut.json?.requestId);
        await submittedBody(fixture, timedOut.json?.requestId ?? '', body);
        const result = expectJsonResult(
          await fixture.runJsonCli<TalkOutput>(['result', timedOut.json?.requestId ?? ''], {
            withoutTmux: true,
          })
        );
        expect(result.response).toBe(body);
        expect(result.bodyBytes).toBe(Buffer.byteLength(body));
      },
      {
        replyGate: true,
        responseBodyBase64: Buffer.from(body).toString('base64'),
      }
    );
  }, 10_000);

  it('interrupts only the observer, releases its wait, and accepts the late native final', async () => {
    const body = 'late final after SIGINT';
    await withE2EFixture(
      async (fixture) => {
        const process = fixture.runCliProcess<TalkOutput>([
          '--json',
          'talk',
          fixture.pane,
          'native interrupt request',
          '--no-preamble',
          '--timeout',
          '20',
        ]);
        const request = await fixture.waitForEvent(
          (event) => event.event === 'request' && event.message === 'native interrupt request',
          5_000
        );
        await fixture.waitFor(
          () => {
            const attempt = requestAttempts(fixture).find(
              (item) => item.request_id === request.requestId
            );
            return attempt?.status === 'sent';
          },
          5_000,
          'native interrupt request settlement'
        );
        process.kill('SIGINT');

        const interrupted = await process.result;
        expect(interrupted.code).toBe(1);
        expect(interrupted.stderr).toBe('');
        expect(interrupted.json).toMatchObject({
          requestId: request.requestId,
          error: { code: 'INTERRUPTED' },
        });
        await fixture.waitFor(
          () =>
            requestAttempts(fixture).find((item) => item.request_id === request.requestId)
              ?.wait_active === 0,
          5_000,
          'native SIGINT waiter cleanup'
        );

        expect(
          fixture
            .events()
            .some(
              (event) =>
                (event.event === 'child-start' || event.event === 'submitted') &&
                event.requestId === request.requestId
            )
        ).toBe(false);
        fixture.releaseReplyGate(request.requestId);
        await submittedBody(fixture, request.requestId ?? '', body);
        const result = expectJsonResult(
          await fixture.runJsonCli<TalkOutput>(['result', request.requestId ?? ''], {
            withoutTmux: true,
          })
        );
        expect(result).toMatchObject({
          status: 'completed',
          requestId: request.requestId,
          response: body,
          bodyBytes: Buffer.byteLength(body),
        });
      },
      {
        replyGate: true,
        responseBodyBase64: Buffer.from(body).toString('base64'),
      }
    );
  }, 12_000);

  it('delivers a compact v2 receipt and retrieves a manually submitted native result', async () => {
    const original = 'receipt guidance request';
    const body = 'manual native final';
    await withE2EFixture(
      async (fixture) => {
        const sent = expectJsonResult(
          await fixture.runJsonCli<TalkOutput>([
            'talk',
            fixture.pane,
            original,
            '--no-preamble',
            '--detach',
          ])
        );
        const guidance = await fixture.waitForEvent(
          (event) =>
            event.event === 'input' &&
            event.pid === fixture.panePid &&
            event.line?.startsWith('tmt reply ') === true,
          5_000
        );
        const match = guidance.line?.match(
          /^tmt reply (req_[0-9a-f-]+) --receipt (v2_[A-Za-z0-9_-]{22}) --message <text>$/
        );
        expect(match).not.toBeNull();
        expect(match?.[1]).toBe(sent.requestId);
        const receipt = match?.[2] ?? '';
        expect(receipt).toHaveLength(25);
        expect(receipt).toMatch(/^v2_[A-Za-z0-9_-]{22}$/);
        expect(
          requestAttempts(fixture).find((row) => row.request_id === sent.requestId)
        ).toMatchObject({
          message_text: original,
          status: 'sent',
          wait_active: 0,
        });

        const reply = expectJsonResult(
          await fixture.runJsonCli<TalkOutput>(
            ['reply', sent.requestId ?? '', '--receipt', receipt, '--message', body],
            { outsideTmux: true }
          )
        );
        expect(reply).toMatchObject({
          status: 'submitted',
          requestId: sent.requestId,
          bodyBytes: Buffer.byteLength(body),
        });
        const result = expectJsonResult(
          await fixture.runJsonCli<TalkOutput>(['result', sent.requestId ?? ''], {
            withoutTmux: true,
          })
        );
        expect(result).toMatchObject({
          status: 'completed',
          requestId: sent.requestId,
          response: body,
          bodyBytes: Buffer.byteLength(body),
          submittedAtMs: reply.submittedAtMs,
        });
      },
      { mode: 'input-log' }
    );
  }, 15_000);

  it('preserves the original prompt while preamble and bang protection affect only delivery', async () => {
    const original = 'if (!ready) { return; }';
    // The mock agent reconstructs received non-empty lines and therefore drops
    // the intentional blank separator from the delivered preamble payload.
    const delivered = '[SYSTEM: Review before answering.]\nif (！ready) { return; }';
    const body = `mock-agent response: ${delivered}`;
    await withE2EFixture(async (fixture) => {
      const peer = await fixture.createMockPane('native-peer');
      expect((await fixture.runJsonCli(['name', 'NativeCaller', '-s'])).code).toBe(0);
      expect((await fixture.runJsonCli(['add', peer.pane, 'NativePeer'])).code).toBe(0);
      expect(
        (await fixture.runJsonCli(['preamble', 'set', 'NativePeer', 'Review before answering.']))
          .code
      ).toBe(0);
      expect(
        (
          await fixture.runJsonCli([
            'role',
            'set',
            'ROLE_MUST_NOT_BE_INJECTED',
            '--identity',
            'NativePeer',
          ])
        ).code
      ).toBe(0);

      const talk = expectJsonResult(
        await fixture.runJsonCli<TalkOutput>(['talk', 'NativePeer', original, '--timeout', '8'])
      );
      const request = await nativeRequest(fixture, talk.requestId ?? '');
      expect(request.message).toBe(delivered);
      const submitted = await submittedBody(fixture, talk.requestId ?? '', body);
      expect(submitted.body).not.toBe(original);
      expect(talk.response).toBe(body);

      const attempt = requestAttempts(fixture).find((row) => row.request_id === talk.requestId);
      expect(attempt).toMatchObject({
        request_id: talk.requestId,
        message_text: original,
        message_bytes: Buffer.byteLength(original),
        originator_kind: 'verified',
        inject_preamble: 1,
        wait_active: 0,
        status: 'queued',
        route_kind: 'inbox',
        wake_state: 'sent',
      });
      const identities = durableState(fixture).identities;
      const callerIdentity = identities.find((identity) => identity.name === 'NativeCaller');
      const peerIdentity = identities.find((identity) => identity.name === 'NativePeer');
      expect(callerIdentity?.id).toEqual(expect.any(String));
      expect(peerIdentity?.id).toEqual(expect.any(String));
      expect(callerIdentity?.id).not.toBe(peerIdentity?.id);
      expect(attempt?.originator_identity_id).toBe(callerIdentity?.id);
      expect(attempt?.recipient_identity_id).toBe(peerIdentity?.id);
      expect(preambleCounters(fixture)[peerIdentity?.id as string]).toBe(1);

      // A raw stable pane resolves the same bound identity and cadence owner.
      // The default cadence does not inject again on its second reservation.
      const direct = expectJsonResult(
        await fixture.runJsonCli<TalkOutput>(['talk', peer.pane, original, '--timeout', '8'])
      );
      expect(direct.identity).toEqual({ name: 'NativePeer', canonicalName: 'nativepeer' });
      const directRequest = await nativeRequest(fixture, direct.requestId ?? '');
      expect(directRequest.message).toBe('if (！ready) { return; }');
      expect(direct.response).toBe('mock-agent response: if (！ready) { return; }');
      expect(
        requestAttempts(fixture).find((row) => row.request_id === direct.requestId)
      ).toMatchObject({
        recipient_identity_id: peerIdentity?.id,
        inject_preamble: 0,
        message_text: original,
      });
      expect(preambleCounters(fixture)[peerIdentity?.id as string]).toBe(2);
    });
  }, 20_000);

  it('honors an explicit offline originator and rejects a missing one before sending', async () => {
    await withE2EFixture(async (fixture) => {
      const peer = await fixture.createMockPane('explicit-originator-peer');
      expect((await fixture.runJsonCli(['name', 'NativeCaller', '-s'])).code).toBe(0);
      expect((await fixture.runJsonCli(['identity', 'create', 'OfflineOriginator'])).code).toBe(0);
      const before = requestAttempts(fixture);
      const missing = await fixture.runJsonCli<TalkOutput>([
        'talk',
        peer.pane,
        'missing originator must not send',
        '--identity',
        'DoesNotExist',
        '--no-preamble',
        '--detach',
      ]);
      expect(missing).toMatchObject({
        code: 3,
        stderr: '',
        json: { error: { code: 'NAME_NOT_FOUND' } },
      });
      expect(requestAttempts(fixture)).toEqual(before);
      expect(
        fixture
          .events()
          .some(
            (event) =>
              event.event === 'request' && event.message === 'missing originator must not send'
          )
      ).toBe(false);
      expect(
        fixture
          .events()
          .some(
            (event) => event.event === 'input' && event.line === 'missing originator must not send'
          )
      ).toBe(false);

      const sent = expectJsonResult(
        await fixture.runJsonCli<TalkOutput>([
          'talk',
          peer.pane,
          'explicit offline originator',
          '--identity',
          'OfflineOriginator',
          '--no-preamble',
          '--timeout',
          '8',
        ])
      );
      const request = await nativeRequest(fixture, sent.requestId ?? '');
      expect(request.message).toBe('explicit offline originator');
      await submittedBody(
        fixture,
        sent.requestId ?? '',
        'mock-agent response: explicit offline originator'
      );
      const identities = durableState(fixture).identities;
      const caller = identities.find((identity) => identity.name === 'NativeCaller');
      const originator = identities.find((identity) => identity.name === 'OfflineOriginator');
      const attempt = requestAttempts(fixture).find((row) => row.request_id === sent.requestId);
      expect(caller?.id).toEqual(expect.any(String));
      expect(originator?.id).toEqual(expect.any(String));
      expect(originator?.id).not.toBe(caller?.id);
      expect(attempt).toMatchObject({
        originator_kind: 'explicit',
        originator_identity_id: originator?.id,
        recipient_identity_id: null,
        status: 'sent',
        wait_active: 0,
      });
    });
  }, 20_000);

  it.each(['json', 'human', 'forced'] as const)(
    'keeps %s overlapping requests correlated with appropriate warnings',
    async (mode) => {
      await withE2EFixture(
        async (fixture) => {
          const slow = fixture.runCliProcess<TalkOutput>([
            '--json',
            'talk',
            fixture.pane,
            'native slow overlap',
            '--no-preamble',
            '--timeout',
            '8',
          ]);
          const slowRequest = await fixture.waitForEvent(
            (event) => event.event === 'request' && event.message === 'native slow overlap',
            5_000
          );
          const fast = fixture.runCliProcess<TalkOutput>([
            ...(mode === 'json' ? ['--json'] : mode === 'forced' ? ['--force'] : []),
            'talk',
            fixture.pane,
            'native fast overlap',
            '--no-preamble',
            '--timeout',
            '8',
          ]);
          const fastRequest = await fixture.waitForEvent(
            (event) => event.event === 'request' && event.message === 'native fast overlap',
            5_000
          );
          expect(slowRequest.requestId).toEqual(expect.stringMatching(/^req_[0-9a-f-]+$/));
          expect(fastRequest.requestId).toEqual(expect.stringMatching(/^req_[0-9a-f-]+$/));
          expect(slowRequest.requestId).not.toBe(fastRequest.requestId);
          await fixture.waitFor(
            () =>
              requestAttempts(fixture).filter(
                (row) =>
                  (row.request_id === slowRequest.requestId ||
                    row.request_id === fastRequest.requestId) &&
                  row.status === 'sent' &&
                  row.wait_active === 1
              ).length === 2,
            5_000,
            'overlapping native requests to settle independently'
          );

          fixture.releaseReplyGate(fastRequest.requestId);
          await submittedBody(
            fixture,
            fastRequest.requestId ?? '',
            'mock-agent response: native fast overlap'
          );
          expect(
            fixture
              .events()
              .some(
                (event) => event.event === 'submitted' && event.requestId === slowRequest.requestId
              )
          ).toBe(false);
          const fastResult = await fast.result;
          expect(fastResult.code).toBe(0);
          if (mode === 'json') {
            expect(fastResult.json).toMatchObject({
              status: 'completed',
              requestId: fastRequest.requestId,
              response: 'mock-agent response: native fast overlap',
            });
          } else {
            expect(fastResult.stdout).toContain(
              `✓ Completed request ${fastRequest.requestId} for ${fixture.pane} (${fixture.pane})\nmock-agent response: native fast overlap\n`
            );
          }
          expect(fastResult.stderr).toBe(
            mode === 'human'
              ? `warning: Another recent request exists for '${fixture.pane}' (id: ${slowRequest.requestId}). Input processing is not serialized; durable results remain associated by request ID\n`
              : ''
          );

          fixture.releaseReplyGate(slowRequest.requestId);
          await submittedBody(
            fixture,
            slowRequest.requestId ?? '',
            'mock-agent response: native slow overlap'
          );
          const slowResult = await slow.result;
          expect(slowResult).toMatchObject({
            code: 0,
            json: {
              status: 'completed',
              requestId: slowRequest.requestId,
              response: 'mock-agent response: native slow overlap',
            },
          });
          expect(requestAttempts(fixture)).toEqual(
            expect.arrayContaining([
              expect.objectContaining({
                request_id: slowRequest.requestId,
                status: 'sent',
                wait_active: 0,
              }),
              expect.objectContaining({
                request_id: fastRequest.requestId,
                status: 'sent',
                wait_active: 0,
              }),
            ])
          );
        },
        { replyGate: true }
      );
    },
    25_000
  );

  it('reports ambiguous paste without replaying native talk input', async () => {
    const original = 'ambiguous! send';
    await withE2EFixture(
      async (fixture) => {
        const result = await fixture.runJsonCli<TalkOutput>(
          ['talk', fixture.pane, original, '--no-preamble', '--detach'],
          { transportFault: { stage: 'paste' } }
        );
        expect(result).toMatchObject({
          code: 1,
          stderr: '',
          json: { error: { code: 'DELIVERY_UNCERTAIN', stage: 'paste' } },
        });
        const requestId = result.json?.requestId ?? '';
        await fixture.waitForEvent(
          (event) => event.event === 'input' && event.pid === fixture.panePid,
          5_000
        );
        const protectedInput = 'ambiguous！ send';
        expect(
          fixture
            .events()
            .filter(
              (event) =>
                event.event === 'input' &&
                event.pid === fixture.panePid &&
                event.line === protectedInput
            )
        ).toHaveLength(1);
        expect(fixture.transportTrace().map((line) => line.split('|', 1)[0])).toEqual([
          'set-buffer.before',
          'set-buffer.after.0',
          'paste-buffer.before',
          'paste-buffer.after.0',
          'paste-buffer.fault-after',
        ]);
        expect(fixture.events().some((event) => event.event === 'submitted')).toBe(false);
        expect(fixture.events().some((event) => event.event === 'child-start')).toBe(false);
        expect(requestAttempts(fixture).find((row) => row.request_id === requestId)).toMatchObject({
          request_id: requestId,
          status: 'uncertain',
          wait_active: 0,
          message_text: original,
        });
      },
      { mode: 'input-log' }
    );
  }, 15_000);
});
