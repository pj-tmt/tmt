import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import { withE2EFixture, type CliResult, type E2EFixture } from './harness.js';
import { preambleCounters, requestAttempts } from './request-state-oracle.js';

interface TalkOutput {
  requestId?: string;
  target?: string;
  pane?: string;
  identity?: { name: string; canonicalName: string };
  status?: string;
  response?: string;
  bodyBytes?: number;
  submittedAtMs?: number;
  error?: { code: string; message: string };
}

interface RoleOutput {
  identity: { id: string; name: string; canonicalName: string };
  role: { content: string } | null;
}

function json<T>(result: CliResult<T>): T {
  expect(result.code, result.stderr || result.stdout).toBe(0);
  expect(result.stderr).toBe('');
  expect(result.json).toBeDefined();
  return result.json as T;
}

async function identityId(fixture: E2EFixture, name: string): Promise<string> {
  const shown = await fixture.runJsonCli<RoleOutput>(['role', 'show', '--identity', name], {
    outsideTmux: true,
  });
  return json(shown).identity.id;
}

describe('TMT-55 request context and provenance', { concurrent: false }, () => {
  it('escapes sender markup without changing stored attribution or exact replies', async () => {
    await withE2EFixture(async (fixture) => {
      const peer = await fixture.createMockPane('escaped-sender-peer');
      const sender = 'A&"<\'>&amp;';
      expect((await fixture.runJsonCli(['identity', 'create', sender])).code).toBe(0);
      const senderId = await identityId(fixture, sender);
      const original = 'Review the sender label.';
      const completed = json(
        await fixture.runJsonCli<TalkOutput>(
          [
            'talk',
            peer.pane,
            original,
            '--identity',
            sender.toLowerCase(),
            '--no-preamble',
            '--timeout',
            '8',
          ],
          { outsideTmux: true }
        )
      );
      const request = await fixture.waitForEvent(
        (event) =>
          event.event === 'request' &&
          event.requestId === completed.requestId &&
          event.pid === peer.pid
      );
      expect(request.replyFrame).toBe('<tmt-reply from="A&amp;&quot;&lt;&apos;&gt;&amp;amp;">');
      expect(request.message).toBe(original);
      expect(completed.status).toBe('completed');
      expect(completed.response).toBe(`mock-agent response: ${original}`);
      expect(
        requestAttempts(fixture).find((row) => row.request_id === completed.requestId)
      ).toMatchObject({
        originator_kind: 'explicit',
        originator_identity_id: senderId,
        message_text: original,
        wait_active: 0,
      });
    });
  });

  it('retains original bound context while delivery composes preamble and receipt framing', async () => {
    await withE2EFixture(
      async (fixture) => {
        const peer = await fixture.createMockPane('peer');
        expect((await fixture.runJsonCli(['name', 'Caller', '-s'])).code).toBe(0);
        expect((await fixture.runJsonCli(['add', peer.pane, 'Peer'])).code).toBe(0);
        const callerId = await identityId(fixture, 'Caller');
        const peerId = await identityId(fixture, 'Peer');
        expect(
          (await fixture.runJsonCli(['preamble', 'set', 'Peer', 'Review before answering.'])).code
        ).toBe(0);

        const original = 'if (!ready) { return; }';
        const process = fixture.runCliProcess<TalkOutput>([
          '--json',
          'talk',
          'Peer',
          original,
          '--timeout',
          '8',
        ]);
        const request = await fixture.waitForEvent(
          (event) => event.event === 'request' && event.message?.includes('if (！ready)') === true,
          5_000
        );
        expect(request.message).toContain('[SYSTEM: Review before answering.]');
        expect(request.message).not.toContain('tmt reply');
        expect(request.replyFrame).toBe('<tmt-reply from="Caller">');

        await fixture.waitFor(() =>
          requestAttempts(fixture).some(
            (row) => row.request_id === request.requestId && row.wake_state === 'sent'
          )
        );
        const prepared = requestAttempts(fixture)[0];
        expect(prepared).toMatchObject({
          request_id: request.requestId,
          originator_kind: 'verified',
          originator_identity_id: callerId,
          recipient_identity_id: peerId,
          message_text: original,
          message_bytes: Buffer.byteLength(original),
          inject_preamble: 1,
          wait_active: 1,
          status: 'queued',
          route_kind: 'inbox',
          wake_state: 'sent',
        });
        expect(prepared.message_expires_at_ms).toBe(prepared.prepared_at_ms + 90 * 86_400_000);
        expect(preambleCounters(fixture)[peerId]).toBe(1);

        fixture.releaseReplyGate(request.requestId);
        const completed = await process.result;
        const output = json(completed);
        const submitted = await fixture.waitForEvent(
          (event) =>
            event.event === 'submitted' &&
            event.requestId === request.requestId &&
            event.pid === peer.pid
        );
        expect(submitted.body).toBe(
          `mock-agent response: [SYSTEM: Review before answering.]\n${original.replaceAll('!', '！')}`
        );
        expect(output.response).toBe(submitted.body);
        expect(Object.keys(output).sort()).toEqual([
          'bodyBytes',
          'identity',
          'pane',
          'requestId',
          'response',
          'status',
          'submittedAtMs',
          'target',
        ]);
        expect(output).toMatchObject({
          requestId: request.requestId,
          target: 'Peer',
          pane: peer.pane,
          identity: { name: 'Peer', canonicalName: 'peer' },
          status: 'completed',
        });
        expect(output).not.toHaveProperty('message');
        expect(output).not.toHaveProperty('prompt');
        expect(output).not.toHaveProperty('receipt');

        const result = await fixture.runJsonCli<TalkOutput>(['result', request.requestId ?? ''], {
          withoutTmux: true,
        });
        const resultOutput = json(result);
        expect(Object.keys(resultOutput).sort()).toEqual([
          'bodyBytes',
          'requestId',
          'response',
          'status',
          'submittedAtMs',
        ]);
        expect(resultOutput).toMatchObject({
          requestId: request.requestId,
          status: 'completed',
        });

        const noPreamble = await fixture.runJsonCli<TalkOutput>([
          'talk',
          peer.pane,
          original,
          '--no-preamble',
          '--detach',
        ]);
        const noPreambleOutput = json(noPreamble);
        const noPreambleRequest = await fixture.waitForEvent(
          (event) => event.event === 'request' && event.requestId === noPreambleOutput.requestId,
          5_000
        );
        expect(noPreambleRequest.message).toBe(original.replaceAll('!', '！'));
        fixture.releaseReplyGate(noPreambleOutput.requestId);
        const second = await fixture.waitForEvent(
          (event) => event.event === 'submitted' && event.requestId === noPreambleOutput.requestId,
          5_000
        );
        expect(second.message).toBe(original.replaceAll('!', '！'));
        const secondRow = requestAttempts(fixture).find(
          (row) => row.request_id === noPreambleOutput.requestId
        );
        expect(secondRow).toMatchObject({
          originator_kind: 'verified',
          originator_identity_id: callerId,
          recipient_identity_id: peerId,
          message_text: original,
          inject_preamble: 0,
          wait_active: 0,
          status: 'queued',
          route_kind: 'inbox',
          wake_state: 'sent',
        });
        expect(preambleCounters(fixture)[peerId]).toBe(1);

        const previousRows = requestAttempts(fixture);
        expect((await fixture.runJsonCli(['unbind'])).code).toBe(0);
        expect((await fixture.runJsonCli(['name', 'Caller', '-s'])).code).toBe(0);
        expect(await identityId(fixture, 'Caller')).toBe(callerId);
        const rebound = await fixture.runJsonCli<TalkOutput>([
          'talk',
          'Peer',
          'after caller rebind',
          '--no-preamble',
          '--detach',
        ]);
        const reboundOutput = json(rebound);
        const reboundRequest = await fixture.waitForEvent(
          (event) => event.event === 'request' && event.requestId === reboundOutput.requestId,
          5_000
        );
        expect(reboundRequest.message).toBe('after caller rebind');
        fixture.releaseReplyGate(reboundOutput.requestId);
        await fixture.waitForEvent(
          (event) => event.event === 'submitted' && event.requestId === reboundOutput.requestId,
          5_000
        );
        const reboundRow = requestAttempts(fixture).find(
          (row) => row.request_id === reboundOutput.requestId
        );
        expect(reboundRow).toMatchObject({
          originator_kind: 'verified',
          originator_identity_id: callerId,
          recipient_identity_id: peerId,
          message_text: 'after caller rebind',
        });
        for (const previous of previousRows) {
          expect(
            requestAttempts(fixture).find((row) => row.request_id === previous.request_id)
          ).toEqual(previous);
        }
      },
      { replyGate: true }
    );
  }, 30_000);

  it('records explicit offline, anonymous direct, and fail-closed originators independently', async () => {
    await withE2EFixture(async (fixture) => {
      const peer = await fixture.createMockPane('peer');
      const anonymous = await fixture.createMockPane('anonymous');
      expect((await fixture.runJsonCli(['name', 'Caller', '-s'])).code).toBe(0);
      expect((await fixture.runJsonCli(['add', peer.pane, 'Peer'])).code).toBe(0);
      const callerId = await identityId(fixture, 'Caller');
      const peerId = await identityId(fixture, 'Peer');

      expect((await fixture.runJsonCli(['unbind'])).code).toBe(0);
      expect((await fixture.runJsonCli(['name', 'BoundCaller'])).code).toBe(0);
      const boundExplicit = await fixture.runJsonCli<TalkOutput>([
        'talk',
        'Peer',
        'explicit overrides bound caller',
        '--identity',
        'Caller',
        '--no-preamble',
        '--detach',
      ]);
      const boundExplicitOutput = json(boundExplicit);
      const boundRequest = await fixture.waitForEvent(
        (event) => event.event === 'request' && event.requestId === boundExplicitOutput.requestId
      );
      expect(boundRequest.replyFrame).toBe('<tmt-reply from="Caller">');
      await fixture.waitForEvent(
        (event) => event.event === 'submitted' && event.requestId === boundExplicitOutput.requestId,
        5_000
      );
      const boundExplicitRow = requestAttempts(fixture).find(
        (row) => row.request_id === boundExplicitOutput.requestId
      );
      expect(boundExplicitRow).toMatchObject({
        originator_kind: 'explicit',
        originator_identity_id: callerId,
        recipient_identity_id: peerId,
        message_text: 'explicit overrides bound caller',
      });

      expect((await fixture.runJsonCli(['unbind'])).code).toBe(0);
      const explicit = await fixture.runJsonCli<TalkOutput>(
        [
          'talk',
          'Peer',
          'explicit offline caller',
          '--identity',
          'caller',
          '--no-preamble',
          '--detach',
        ],
        { outsideTmux: true }
      );
      const explicitOutput = json(explicit);
      const explicitRequest = await fixture.waitForEvent(
        (event) => event.event === 'request' && event.requestId === explicitOutput.requestId
      );
      expect(explicitRequest.replyFrame).toBe('<tmt-reply from="Caller">');
      await fixture.waitForEvent(
        (event) => event.event === 'submitted' && event.requestId === explicitOutput.requestId,
        5_000
      );
      const explicitRow = requestAttempts(fixture).find(
        (row) => row.request_id === explicitOutput.requestId
      );
      expect(explicitRow).toMatchObject({
        originator_kind: 'explicit',
        originator_identity_id: callerId,
        recipient_identity_id: peerId,
        message_text: 'explicit offline caller',
      });

      const attemptsBeforeReject = requestAttempts(fixture).length;
      const countersBeforeReject = preambleCounters(fixture);
      const rejected = await fixture.runJsonCli<TalkOutput>(
        ['talk', 'missing-recipient', 'must not send', '--identity', 'missing-originator'],
        { outsideTmux: true }
      );
      expect(rejected.code).toBe(3);
      expect(rejected.json).toEqual({
        error: {
          code: 'NAME_NOT_FOUND',
          message: "Identity 'missing-recipient' was not found.",
        },
      });
      expect(requestAttempts(fixture)).toHaveLength(attemptsBeforeReject);
      expect(preambleCounters(fixture)).toEqual(countersBeforeReject);
      expect(fixture.events().some((event) => event.message === 'must not send')).toBe(false);

      const unknownOriginator = await fixture.runJsonCli<TalkOutput>(
        ['talk', 'Peer', 'must not send', '--identity', 'missing-originator'],
        { outsideTmux: true }
      );
      expect(unknownOriginator.code).toBe(3);
      expect(unknownOriginator.json).toEqual({
        error: {
          code: 'NAME_NOT_FOUND',
          message: "Identity 'missing-originator' was not found.",
        },
      });
      expect(requestAttempts(fixture)).toHaveLength(attemptsBeforeReject);
      expect(preambleCounters(fixture)).toEqual(countersBeforeReject);
      expect(fixture.events().some((event) => event.message === 'must not send')).toBe(false);

      const unknownRecipient = await fixture.runJsonCli<TalkOutput>(
        ['talk', 'Peer', 'anonymous caller with named recipient', '--no-preamble', '--detach'],
        { outsideTmux: true }
      );
      const unknownRecipientOutput = json(unknownRecipient);
      const unknownRequest = await fixture.waitForEvent(
        (event) => event.event === 'request' && event.requestId === unknownRecipientOutput.requestId
      );
      expect(unknownRequest.replyFrame).toBe('<tmt-reply from="unknown">');
      await fixture.waitForEvent(
        (event) =>
          event.event === 'submitted' && event.requestId === unknownRecipientOutput.requestId,
        5_000
      );
      const unknownRecipientRow = requestAttempts(fixture).find(
        (row) => row.request_id === unknownRecipientOutput.requestId
      );
      expect(unknownRecipientRow).toMatchObject({
        originator_kind: 'unknown',
        originator_identity_id: null,
        recipient_identity_id: peerId,
        message_text: 'anonymous caller with named recipient',
      });

      const anonymousTalk = await fixture.runJsonCli<TalkOutput>(
        ['talk', anonymous.pane, 'anonymous direct', '--no-preamble', '--detach'],
        { outsideTmux: true }
      );
      const anonymousOutput = json(anonymousTalk);
      await fixture.waitForEvent(
        (event) => event.event === 'submitted' && event.requestId === anonymousOutput.requestId,
        5_000
      );
      const anonymousRow = requestAttempts(fixture).find(
        (row) => row.request_id === anonymousOutput.requestId
      );
      expect(anonymousRow).toMatchObject({
        originator_kind: 'unknown',
        originator_identity_id: null,
        recipient_identity_id: null,
        message_text: 'anonymous direct',
      });

      expect(explicitRow).toBeDefined();
      const explicitRowBeforeInvalidConfig = { ...explicitRow };
      fs.writeFileSync(path.join(fixture.globalDir, 'config.json'), '{ invalid json');
      const invalidConfigResult = await fixture.runJsonCli<TalkOutput>(
        ['result', explicitOutput.requestId ?? ''],
        { withoutTmux: true }
      );
      expect(json(invalidConfigResult)).toEqual({
        status: 'completed',
        requestId: explicitOutput.requestId,
        response: 'mock-agent response: explicit offline caller',
        bodyBytes: Buffer.byteLength('mock-agent response: explicit offline caller'),
        submittedAtMs: expect.any(Number),
      });
      expect(
        requestAttempts(fixture).find((row) => row.request_id === explicitOutput.requestId)
      ).toEqual(explicitRowBeforeInvalidConfig);
      expect(fs.existsSync(fixture.forbiddenTmuxLogPath)).toBe(false);
    });
  }, 30_000);
});
