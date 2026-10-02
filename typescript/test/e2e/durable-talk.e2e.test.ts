import { describe, expect, it } from 'vitest';
import { withE2EFixture, type E2EFixture, type MockEvent } from './harness.js';

interface TalkResult {
  status?: string;
  requestId?: string;
  response?: string;
  bodyBytes?: number;
  submittedAtMs?: number;
  error?: { code: string };
}

interface ResultOutput {
  status?: string;
  requestId?: string;
  response?: string;
  bodyBytes?: number;
  submittedAtMs?: number;
  error?: { code: string };
}

function submittedFor(events: MockEvent[], requestId: string): MockEvent[] {
  return events.filter((event) => event.event === 'submitted' && event.requestId === requestId);
}

/**
 * The mock agent logs `submitted` only after its `tmt reply` child has closed, which
 * can be after `talk` has already returned the response, so read it with a bounded
 * wait rather than once.
 */
function waitForSubmitted(fixture: E2EFixture, requestId: string): Promise<MockEvent> {
  return fixture.waitForEvent(
    (event) => event.event === 'submitted' && event.requestId === requestId,
    5_000
  );
}

describe('TMT-39 durable talk contract', { concurrent: false }, () => {
  it.each([
    ['empty', ''],
    ['whitespace', ' \t  \n\r\n '],
    ['structured unicode', '\uFEFF日本語🙂\u0000\r\nRESPONSE-END-fake-marker'],
  ])('returns the exact %s response body', async (_label, body) => {
    await withE2EFixture(
      async (fixture) => {
        const result = await fixture.runJsonCli<TalkResult>([
          'talk',
          fixture.pane,
          'exact body request',
          '--no-preamble',
          '--timeout',
          '8',
        ]);
        expect(result.code, result.stderr || result.stdout).toBe(0);
        expect(result.json).toMatchObject({
          status: 'completed',
          response: body,
          bodyBytes: Buffer.byteLength(body),
          requestId: expect.stringMatching(/^req_[0-9a-f-]+$/),
          submittedAtMs: expect.any(Number),
        });
        const event = await waitForSubmitted(fixture, result.json?.requestId ?? '');
        expect(event).toMatchObject({ body, bodyBytes: Buffer.byteLength(body) });
        expect(event.submittedAtMs).toBe(result.json?.submittedAtMs);
      },
      { responseBodyBase64: Buffer.from(body, 'utf8').toString('base64') }
    );
  });

  it('submits an exact Unicode and whitespace body through the inline public reply', async () => {
    const body = '  日本語🙂\t\nline two  \r\n';
    await withE2EFixture(
      async (fixture) => {
        const talk = await fixture.runJsonCli<TalkResult>([
          'talk',
          fixture.pane,
          'inline public reply request',
          '--no-preamble',
          '--timeout',
          '8',
        ]);
        expect(talk.code, talk.stderr || talk.stdout).toBe(0);
        expect(talk.json).toMatchObject({
          status: 'completed',
          response: body,
          bodyBytes: Buffer.byteLength(body),
        });

        const requestId = talk.json?.requestId ?? '';
        const child = await fixture.waitForEvent(
          (event) => event.event === 'child-start' && event.requestId === requestId
        );
        expect(child.replyInput).toBe('message');
        const submitted = await fixture.waitForEvent(
          (event) => event.event === 'submitted' && event.requestId === requestId
        );
        expect(submitted.body).toBe(body);
        expect(submitted.bodyBytes).toBe(Buffer.byteLength(body));

        const result = await fixture.runJsonCli<ResultOutput>(['result', requestId]);
        expect(result).toMatchObject({
          code: 0,
          json: {
            status: 'completed',
            requestId,
            response: body,
            bodyBytes: Buffer.byteLength(body),
            submittedAtMs: talk.json?.submittedAtMs,
          },
        });
      },
      {
        replyInput: 'message',
        responseBodyBase64: Buffer.from(body, 'utf8').toString('base64'),
      }
    );
  });

  it('accepts the exact one-megabyte UTF-8 boundary and rejects one byte over it', async () => {
    const boundary = '🙂'.repeat(Math.floor((1024 * 1024) / 4)) + 'a'.repeat((1024 * 1024) % 4);
    await withE2EFixture(
      async (fixture) => {
        const result = await fixture.runJsonCli<TalkResult>([
          'talk',
          fixture.pane,
          'one megabyte',
          '--no-preamble',
          '--timeout',
          '12',
        ]);
        expect(result.code, result.stderr || result.stdout).toBe(0);
        expect(result.json?.response).toBe(boundary);
        expect(result.json?.bodyBytes).toBe(1024 * 1024);
        expect((await waitForSubmitted(fixture, result.json?.requestId ?? '')).body).toBe(boundary);
      },
      { responseBytes: 1024 * 1024, responseMultibyte: true }
    );

    await withE2EFixture(
      async (fixture) => {
        const process = fixture.runCliProcess<TalkResult>([
          '--json',
          'talk',
          fixture.pane,
          'oversize body',
          '--no-preamble',
          '--timeout',
          '2',
        ]);
        const failure = await fixture.waitForEvent(
          (event) => event.event === 'failure' && event.stage === 'reply'
        );
        expect(failure.error).toMatchObject({ code: 'RESPONSE_INPUT_TOO_LARGE' });
        const result = await process.result;
        expect(result.code).toBe(4);
        expect(result.json).toMatchObject({ status: 'timeout', error: { code: 'TIMEOUT' } });
        expect(fixture.events().some((event) => event.event === 'summary')).toBe(false);
      },
      { responseBytes: 1024 * 1024 + 1, responseMultibyte: true }
    );
  }, 30_000);

  it('supports detach followed by result without confusing submission with task completion', async () => {
    await withE2EFixture(
      async (fixture) => {
        const sent = await fixture.runJsonCli<TalkResult>([
          'talk',
          fixture.pane,
          'detached request',
          '--no-preamble',
          '--detach',
        ]);
        expect(sent.code).toBe(0);
        expect(sent.json).toMatchObject({ status: 'sent', requestId: expect.any(String) });
        const submitted = await fixture.waitForEvent(
          (event) => event.event === 'submitted' && event.requestId === sent.json?.requestId,
          5_000
        );
        expect(submitted.body).toBe('mock-agent response: detached request');
        const result = await fixture.runJsonCli<ResultOutput>([
          'result',
          sent.json?.requestId ?? '',
        ]);
        expect(result).toMatchObject({
          code: 0,
          json: {
            status: 'completed',
            requestId: sent.json?.requestId,
            response: 'mock-agent response: detached request',
            bodyBytes: Buffer.byteLength('mock-agent response: detached request'),
          },
        });
      },
      { replyDelayMs: 150 }
    );
  });

  it('makes a late reply available after the observer times out', async () => {
    await withE2EFixture(
      async (fixture) => {
        const process = fixture.runCliProcess<TalkResult>([
          '--json',
          'talk',
          fixture.pane,
          'late request',
          '--no-preamble',
          '--timeout',
          '1',
        ]);
        const timedOut = await process.result;
        expect(timedOut.code).toBe(4);
        expect(timedOut.json).toMatchObject({ status: 'timeout', error: { code: 'TIMEOUT' } });
        const submitted = await fixture.waitForEvent(
          (event) => event.event === 'submitted' && event.requestId === timedOut.json?.requestId,
          5_000
        );
        expect(submitted.body).toBe('mock-agent response: late request');
        const result = await fixture.runJsonCli<ResultOutput>([
          'result',
          timedOut.json?.requestId ?? '',
        ]);
        expect(result).toMatchObject({
          code: 0,
          json: { status: 'completed', response: submitted.body },
        });
      },
      { replyDelayMs: 1_500 }
    );
  }, 10_000);

  it('records a fake marker as non-terminal output without a durable submission', async () => {
    await withE2EFixture(
      async (fixture) => {
        const process = fixture.runCliProcess<TalkResult>([
          '--json',
          'talk',
          fixture.pane,
          'fake marker request',
          '--no-preamble',
          '--timeout',
          '1',
        ]);
        const request = await fixture.waitForEvent(
          (event) => event.event === 'request' && event.message === 'fake marker request'
        );
        const marker = await fixture.waitForEvent(
          (event) => event.event === 'fake-marker' && event.requestId === request.requestId
        );
        expect(marker.requestId).toBe(request.requestId);
        const result = await process.result;
        expect(result.code).toBe(4);
        expect(result.json).toMatchObject({ status: 'timeout', error: { code: 'TIMEOUT' } });
        expect(fixture.events().some((event) => event.event === 'submitted')).toBe(false);
      },
      { mode: 'fake-marker' }
    );
  });

  it('correlates same-pane concurrent replies when the fast reply commits first', async () => {
    await withE2EFixture(
      async (fixture) => {
        const slow = fixture.runCliProcess<TalkResult>([
          '--json',
          'talk',
          fixture.pane,
          'slow same pane',
          '--no-preamble',
          '--timeout',
          '8',
        ]);
        const fast = fixture.runCliProcess<TalkResult>([
          '--json',
          'talk',
          fixture.pane,
          'fast same pane',
          '--no-preamble',
          '--timeout',
          '8',
        ]);
        const slowRequest = await fixture.waitForEvent(
          (event) => event.event === 'request' && event.message === 'slow same pane'
        );
        const fastRequest = await fixture.waitForEvent(
          (event) => event.event === 'request' && event.message === 'fast same pane'
        );
        expect(slowRequest.requestId).toEqual(expect.any(String));
        expect(fastRequest.requestId).toEqual(expect.any(String));
        fixture.releaseReplyGate(fastRequest.requestId!);
        await fixture.waitForEvent(
          (event) => event.event === 'submitted' && event.requestId === fastRequest.requestId
        );
        fixture.releaseReplyGate(slowRequest.requestId!);
        const [slowResult, fastResult] = await Promise.all([slow.result, fast.result]);
        expect(slowResult.code).toBe(0);
        expect(fastResult.code).toBe(0);
        expect(slowResult.json?.response).toBe('mock-agent response: slow same pane');
        expect(fastResult.json?.response).toBe('mock-agent response: fast same pane');
        const submissions = fixture
          .events()
          .filter((event) => event.event === 'submitted')
          .filter(
            (event) =>
              event.requestId === slowResult.json?.requestId ||
              event.requestId === fastResult.json?.requestId
          );
        expect(submissions.map((event) => event.message)).toEqual([
          'fast same pane',
          'slow same pane',
        ]);
        expect(submissions.map((event) => event.body)).toEqual([
          'mock-agent response: fast same pane',
          'mock-agent response: slow same pane',
        ]);
      },
      { replyGate: true }
    );
  }, 15_000);

  it('accepts an identical public reply retry without resubmitting a different body', async () => {
    await withE2EFixture(
      async (fixture) => {
        const result = await fixture.runJsonCli<TalkResult>([
          'talk',
          fixture.pane,
          'identical retry request',
          '--no-preamble',
          '--timeout',
          '8',
        ]);
        expect(result.code).toBe(0);
        const requestId = result.json?.requestId ?? '';
        await fixture.waitForEvent(
          (event) =>
            event.event === 'submitted' && event.requestId === requestId && event.stage === 'retry'
        );
        const submissions = submittedFor(fixture.events(), requestId);
        expect(submissions).toHaveLength(2);
        expect(submissions[0]?.body).toBe(submissions[1]?.body);
        expect(submissions[0]?.submittedAtMs).toBe(submissions[1]?.submittedAtMs);
      },
      { replyRetry: true }
    );
  });

  it('surfaces a failed submission without a success summary', async () => {
    await withE2EFixture(
      async (fixture) => {
        const process = fixture.runCliProcess<TalkResult>([
          '--json',
          'talk',
          fixture.pane,
          'failed submission request',
          '--no-preamble',
          '--timeout',
          '1',
        ]);
        const failure = await fixture.waitForEvent(
          (event) => event.event === 'failure' && event.stage === 'reply'
        );
        expect(failure.error).toMatchObject({ code: 'RESPONSE_REQUEST_NOT_FOUND' });
        const result = await process.result;
        expect(result.code).toBe(4);
        expect(fixture.events().some((event) => event.event === 'summary')).toBe(false);
      },
      { replyFailure: true }
    );
  });

  it('records accepted reply, retry, conflict, ack loss, and summary failure causally', async () => {
    await withE2EFixture(
      async (fixture) => {
        const result = await fixture.runJsonCli<TalkResult>([
          'talk',
          fixture.pane,
          'retry request',
          '--no-preamble',
          '--timeout',
          '8',
        ]);
        expect(result.code).toBe(0);
        const requestId = result.json?.requestId ?? '';
        await fixture.waitForEvent(
          (event) => event.event === 'failure' && event.stage === 'ack-lost'
        );
        await fixture.waitForEvent(
          (event) => event.event === 'submitted' && event.stage === 'retry'
        );
        expect(
          submittedFor(fixture.events(), requestId).every(
            (event) => event.body === result.json?.response
          )
        ).toBe(true);
      },
      { replyAckLoss: true }
    );

    await withE2EFixture(
      async (fixture) => {
        const result = await fixture.runJsonCli<TalkResult>([
          'talk',
          fixture.pane,
          'conflict request',
          '--no-preamble',
          '--timeout',
          '8',
        ]);
        expect(result.code).toBe(0);
        const requestId = result.json?.requestId ?? '';
        const conflict = await fixture.waitForEvent(
          (event) => event.event === 'failure' && event.stage === 'conflict'
        );
        expect(conflict.error).toMatchObject({ code: 'RESPONSE_CONFLICT' });
        expect(submittedFor(fixture.events(), requestId)).toHaveLength(1);
      },
      { replyConflict: true }
    );

    await withE2EFixture(
      async (fixture) => {
        const result = await fixture.runJsonCli<TalkResult>([
          'talk',
          fixture.pane,
          'summary failure request',
          '--no-preamble',
          '--timeout',
          '8',
        ]);
        expect(result.code).toBe(0);
        const requestId = result.json?.requestId ?? '';
        await fixture.waitForEvent(
          (event) => event.event === 'failure' && event.stage === 'summary'
        );
        expect(submittedFor(fixture.events(), requestId)).toHaveLength(1);
        expect(
          fixture
            .events()
            .some((event) => event.event === 'summary' && event.requestId === requestId)
        ).toBe(false);
      },
      { summaryFailure: true }
    );
  }, 35_000);
});
