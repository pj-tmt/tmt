import { describe, expect, it } from 'vitest';
import { E2EFixture, withE2EFixture } from './harness.js';
import { requestAttempts } from './request-state-oracle.js';

interface TalkResult {
  requestId?: string;
  response?: string;
  status?: string;
}

interface ErrorResult {
  error: {
    code: string;
    message: string;
    stage?: string;
  };
}

function protectedMessage(): string {
  // Non-ASCII fixture text is intentional Unicode transport data.
  return [
    '! protected bang',
    'Enter C-c --leading-dash "double" \'single\' $dollar `backtick` 日本語',
    'second line with ! and --another-dash',
  ].join('\n');
}

function expectedPayload(message: string): string {
  return message.replaceAll('!', '！');
}

function stages(trace: string[]): string[] {
  return trace.map((line) => line.split('|', 1)[0]);
}

function bufferName(trace: string[]): string {
  const setBuffer = trace.find((line) => line.startsWith('set-buffer.'));
  const match = setBuffer?.match(/\[(tmt-[^\]]+)\]/);
  if (!match) throw new Error(`Transport trace has no buffer name: ${trace.join('\n')}`);
  return match[1];
}

function bufferNames(fixture: E2EFixture): string[] {
  return fixture
    .tmux(['list-buffers', '-F', '#{buffer_name}'])
    .split('\n')
    .map((name) => name.trim())
    .filter(Boolean);
}

function showBuffer(fixture: { tmux(args: string[]): string }, name: string): string {
  return fixture.tmux(['show-buffer', '-b', name]).trimEnd();
}

async function waitForInput(fixture: E2EFixture, pid: number, line: string): Promise<void> {
  await fixture.waitForEvent(
    (event) => event.event === 'input' && event.pid === pid && event.line === line
  );
}

describe('TMT-24 safe transport', { concurrent: false }, () => {
  it('preserves protected multiline payloads across normal and literal fallback sends', async () => {
    await withE2EFixture(async (fixture) => {
      const normal = await fixture.createMockPane('normal');
      const fallback = await fixture.createMockPane('fallback');
      expect((await fixture.runJsonCli(['add', normal.pane, 'Normal'])).code).toBe(0);
      expect((await fixture.runJsonCli(['add', fallback.pane, 'Fallback'])).code).toBe(0);

      const message = protectedMessage();
      const expected = expectedPayload(message);

      const normalTalk = await fixture.runJsonCli<TalkResult>([
        'talk',
        'Normal',
        message,
        '--no-preamble',
        '--timeout',
        '8',
      ]);
      expect(normalTalk).toMatchObject({ code: 0, json: { status: 'completed' } });
      const normalResponse = await fixture.waitForEvent(
        (event) =>
          event.event === 'submitted' &&
          event.pid === normal.pid &&
          event.requestId === normalTalk.json?.requestId
      );
      expect(normalResponse.message).toBe(expected);

      const sentinel = 'tmt-e2e-transport-sentinel';
      fixture.tmux(['set-buffer', '-b', sentinel, '--', 'keep-this-buffer']);
      const traceStart = fixture.transportTrace().length;
      const fallbackTalk = await fixture.runJsonCli<TalkResult>(
        ['talk', 'Fallback', message, '--no-preamble', '--timeout', '8'],
        { transportFault: { stage: 'set-buffer' } }
      );
      expect(fallbackTalk).toMatchObject({ code: 0, json: { status: 'completed' } });
      const fallbackResponse = await fixture.waitForEvent(
        (event) =>
          event.event === 'submitted' &&
          event.pid === fallback.pid &&
          event.requestId === fallbackTalk.json?.requestId
      );
      expect(fallbackResponse.message).toBe(expected);
      expect(showBuffer(fixture, sentinel)).toBe('keep-this-buffer');

      const fallbackTrace = fixture.transportTrace().slice(traceStart);
      const fallbackStages = stages(fallbackTrace);
      expect(fallbackStages).toEqual([
        'set-buffer.fault-before',
        'literal-input.before',
        'literal-input.after.0',
        'submit.before',
        'submit.after.0',
      ]);
      expect(fallbackStages).not.toContain('paste-buffer.before');
      expect(fallbackStages.filter((stage) => stage === 'literal-input.before')).toHaveLength(1);
      expect(fallbackStages.filter((stage) => stage === 'submit.before')).toHaveLength(1);
      expect(bufferNames(fixture)).not.toContain(bufferName(fallbackTrace));
    });
  }, 30_000);

  it('reports uncertain paste and submit without replay in explicit detach mode', async () => {
    await withE2EFixture(
      async (fixture) => {
        const literalPeer = await fixture.createMockPane('literal-fault');
        const pastePeer = await fixture.createMockPane('paste-fault');
        const submitPeer = await fixture.createMockPane('submit-fault');
        expect((await fixture.runJsonCli(['add', literalPeer.pane, 'LiteralFault'])).code).toBe(0);
        expect((await fixture.runJsonCli(['add', pastePeer.pane, 'PasteFault'])).code).toBe(0);
        expect((await fixture.runJsonCli(['add', submitPeer.pane, 'SubmitFault'])).code).toBe(0);

        const literalSentinel = 'tmt-e2e-literal-sentinel';
        fixture.tmux(['set-buffer', '-b', literalSentinel, '--', 'keep-literal-buffer']);
        const literalTraceStart = fixture.transportTrace().length;
        const literal = await fixture.runJsonCli<TalkResult>(
          ['talk', 'LiteralFault', 'Enter', '--no-preamble', '--detach'],
          { transportFault: { stage: 'set-buffer' } }
        );
        expect(literal).toMatchObject({ code: 0, json: { status: 'sent' } });
        await waitForInput(fixture, literalPeer.pid, 'Enter');
        expect(showBuffer(fixture, literalSentinel)).toBe('keep-literal-buffer');
        const literalTrace = fixture.transportTrace().slice(literalTraceStart);
        expect(stages(literalTrace)).toEqual([
          'set-buffer.fault-before',
          'literal-input.before',
          'literal-input.after.0',
          'submit.before',
          'submit.after.0',
        ]);
        expect(bufferNames(fixture)).not.toContain(bufferName(literalTrace));

        const pasteSentinel = 'tmt-e2e-paste-sentinel';
        fixture.tmux(['set-buffer', '-b', pasteSentinel, '--', 'keep-paste-buffer']);
        const pasteTraceStart = fixture.transportTrace().length;
        const paste = await fixture.runJsonCli<ErrorResult>(
          ['talk', 'PasteFault', 'C-c', '--no-preamble', '--detach'],
          { transportFault: { stage: 'paste' } }
        );
        expect(paste).toMatchObject({
          code: 1,
          json: { error: { code: 'DELIVERY_UNCERTAIN', stage: 'paste' } },
        });
        await waitForInput(fixture, pastePeer.pid, 'C-c');
        expect(showBuffer(fixture, pasteSentinel)).toBe('keep-paste-buffer');
        const pasteTrace = fixture.transportTrace().slice(pasteTraceStart);
        expect(stages(pasteTrace)).toEqual([
          'set-buffer.before',
          'set-buffer.after.0',
          'paste-buffer.before',
          'paste-buffer.after.0',
          'paste-buffer.fault-after',
        ]);
        expect(stages(pasteTrace)).not.toContain('literal-input.before');
        expect(stages(pasteTrace)).not.toContain('submit.before');
        expect(bufferNames(fixture)).not.toContain(bufferName(pasteTrace));
        expect(
          fixture
            .events()
            .filter(
              (event) =>
                event.event === 'input' && event.pid === pastePeer.pid && event.line === 'C-c'
            )
        ).toHaveLength(1);

        const submitSentinel = 'tmt-e2e-submit-sentinel';
        fixture.tmux(['set-buffer', '-b', submitSentinel, '--', 'keep-submit-buffer']);
        const submitTraceStart = fixture.transportTrace().length;
        const submit = await fixture.runJsonCli<ErrorResult>(
          ['talk', 'SubmitFault', '--no-preamble', '--detach', '--', '--leading-dash'],
          { transportFault: { stage: 'submit' } }
        );
        expect(submit).toMatchObject({
          code: 1,
          json: { error: { code: 'DELIVERY_UNCERTAIN', stage: 'submit' } },
        });
        await waitForInput(fixture, submitPeer.pid, '--leading-dash');
        expect(showBuffer(fixture, submitSentinel)).toBe('keep-submit-buffer');
        const submitTrace = fixture.transportTrace().slice(submitTraceStart);
        expect(stages(submitTrace)).toEqual([
          'set-buffer.before',
          'set-buffer.after.0',
          'paste-buffer.before',
          'paste-buffer.after.0',
          'submit.before',
          'submit.after.0',
          'submit.fault-after',
        ]);
        expect(stages(submitTrace)).not.toContain('literal-input.before');
        expect(bufferNames(fixture)).not.toContain(bufferName(submitTrace));
        expect(
          fixture
            .events()
            .filter(
              (event) =>
                event.event === 'input' &&
                event.pid === submitPeer.pid &&
                event.line === '--leading-dash'
            )
        ).toHaveLength(1);
      },
      { mode: 'input-log' }
    );
  }, 30_000);

  it('preserves the single-receipt reply frame across protected transport', async () => {
    await withE2EFixture(
      async (fixture) => {
        const message = protectedMessage();
        const sent = await fixture.runJsonCli<TalkResult>([
          'talk',
          fixture.pane,
          message,
          '--no-preamble',
          '--detach',
        ]);
        expect(sent).toMatchObject({ code: 0, json: { status: 'sent' } });
        await waitForInput(fixture, fixture.panePid, '</tmt-reply>');

        const lines = fixture
          .events()
          .filter((event) => event.event === 'input' && event.pid === fixture.panePid)
          .map((event) => event.line);
        expect(lines).toContain('！ protected bang');
        const begin = lines.indexOf('<tmt-reply from="unknown">');
        const end = lines.indexOf('</tmt-reply>');
        expect(begin).toBeGreaterThanOrEqual(0);
        expect(end).toBeGreaterThan(begin);
        const frame = lines.slice(begin, end + 1);
        expect(frame).toHaveLength(3);
        expect(frame[0]).toBe('<tmt-reply from="unknown">');
        expect(frame[2]).toBe('</tmt-reply>');
        expect(frame[1]).toMatch(
          /^tmt reply req_[0-9a-f-]+ --receipt [A-Za-z0-9_-]+ --message <text>$/
        );
        const receipt = frame[1]?.match(/--receipt (\S+) --message/)?.[1];
        expect(receipt).toBeTruthy();
        expect(lines.filter((line) => line?.includes(receipt ?? '')).length).toBe(1);
      },
      { mode: 'input-log' }
    );
  });

  it('clears wait request state after an uncertain submit', async () => {
    await withE2EFixture(
      async (fixture) => {
        const peer = await fixture.createMockPane('wait-submit-fault');
        expect((await fixture.runJsonCli(['add', peer.pane, 'WaitSubmitFault'])).code).toBe(0);

        const result = await fixture.runJsonCli<ErrorResult>(
          ['talk', 'WaitSubmitFault', 'state cleanup', '--no-preamble', '--timeout', '8'],
          { transportFault: { stage: 'submit' } }
        );
        expect(result).toMatchObject({
          code: 1,
          json: { error: { code: 'DELIVERY_UNCERTAIN', stage: 'submit' } },
        });
        expect(requestAttempts(fixture)).toHaveLength(1);
        expect(requestAttempts(fixture)[0]).toMatchObject({
          status: 'queued',
          route_kind: 'inbox',
          wake_state: 'uncertain',
          wait_active: 0,
          recipient_identity_id: expect.any(String),
        });
      },
      { mode: 'respond' }
    );
  }, 15_000);
});
