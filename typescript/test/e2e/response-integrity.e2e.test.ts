import { describe, expect, it } from 'vite-plus/test';
import { withE2EFixture } from './harness.js';

interface TalkResult {
  requestId: string;
  pane: string;
  response: string;
  bodyBytes: number;
  submittedAtMs: number;
  status: string;
}

interface CheckResult {
  output: string;
}

function expectedVirtualizedResponse(token: string): string {
  return [
    `VIRTUALIZED-BEGIN:${token}`,
    ...Array.from(
      { length: 200 },
      (_, index) => `VIRTUALIZED-LINE-${String(index + 1).padStart(3, '0')}:${token}`
    ),
    `VIRTUALIZED-END:${token}`,
  ].join('\n');
}

describe('TMT-39 durable response integrity', { concurrent: false }, () => {
  it('returns the exact full body when the pane renders only a virtualized tail', async () => {
    await withE2EFixture(
      async (fixture) => {
        const peer = await fixture.createMockPane('virtualized-agent');
        const binding = await fixture.runJsonCli(['add', peer.pane, 'Virtualized']);
        expect(binding.code).toBe(0);

        const token = 'tmt39-virtualized-response-🙂-日本語';
        const expectedResponse = expectedVirtualizedResponse(token);
        const talk = await fixture.runJsonCli<TalkResult>([
          'talk',
          'Virtualized',
          token,
          '--no-preamble',
          '--timeout',
          '8',
        ]);

        expect(talk.code).toBe(0);
        expect(talk.json).toMatchObject({ status: 'completed' });
        expect(talk.json?.pane).toBe(peer.pane);
        expect(talk.json?.requestId).toMatch(/^req_[0-9a-f-]+$/);
        expect(talk.json?.response).toBe(expectedResponse);
        expect(talk.json?.bodyBytes).toBe(Buffer.byteLength(expectedResponse));
        expect(talk.json).not.toHaveProperty('nonce');
        expect(talk.json).not.toHaveProperty('endMarker');
        expect(talk.json).not.toHaveProperty('truncated');

        const requestEvent = await fixture.waitForEvent(
          (event) =>
            event.event === 'request' &&
            event.pid === peer.pid &&
            event.requestId === talk.json?.requestId
        );
        expect(requestEvent).toMatchObject({
          message: token,
          mode: 'virtualized',
          requestId: talk.json?.requestId,
          pid: peer.pid,
        });
        const responseEvent = await fixture.waitForEvent(
          (event) =>
            event.event === 'submitted' &&
            event.pid === peer.pid &&
            event.requestId === talk.json?.requestId
        );
        expect(responseEvent.message).toBe(token);
        expect(responseEvent.body).toBe(expectedResponse);
        expect(Buffer.byteLength(responseEvent.body ?? '')).toBe(
          Buffer.byteLength(expectedResponse)
        );
        expect(responseEvent.bodyBytes).toBe(talk.json?.bodyBytes);
        expect(responseEvent.submittedAtMs).toBe(talk.json?.submittedAtMs);
        await fixture.waitForEvent(
          (event) => event.event === 'summary' && event.requestId === talk.json?.requestId
        );
        const events = fixture.events();
        const requestIndex = events.findIndex(
          (event) =>
            event.event === 'request' &&
            event.pid === peer.pid &&
            event.requestId === talk.json?.requestId
        );
        const responseIndex = events.findIndex(
          (event) =>
            event.event === 'submitted' &&
            event.pid === peer.pid &&
            event.requestId === talk.json?.requestId
        );
        const summaryIndex = events.findIndex(
          (event) =>
            event.event === 'summary' &&
            event.pid === peer.pid &&
            event.requestId === talk.json?.requestId
        );
        expect(requestIndex).toBeGreaterThanOrEqual(0);
        expect(responseIndex).toBeGreaterThan(requestIndex);
        expect(summaryIndex).toBeGreaterThan(responseIndex);

        const retrieved = await fixture.runJsonCli<TalkResult>([
          'result',
          talk.json?.requestId ?? '',
        ]);
        expect(retrieved).toMatchObject({
          code: 0,
          json: {
            status: 'completed',
            requestId: talk.json?.requestId,
            response: expectedResponse,
            bodyBytes: Buffer.byteLength(expectedResponse),
            submittedAtMs: talk.json?.submittedAtMs,
          },
        });

        const capturedAtDefault = await fixture.runJsonCli<CheckResult>([
          'check',
          'Virtualized',
          '100',
        ]);
        const capturedAtMax = await fixture.runJsonCli<CheckResult>([
          'check',
          'Virtualized',
          '2000',
        ]);
        for (const captured of [capturedAtDefault, capturedAtMax]) {
          expect(captured.code).toBe(0);
          expect(captured.json?.output).toContain(`VIRTUALIZED-END:${token}`);
          expect(captured.json?.output).not.toContain(`VIRTUALIZED-LINE-100:${token}`);
        }
      },
      { mode: 'virtualized' }
    );
  }, 15_000);
});
