import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { withE2EFixture, type E2EFixture, type E2EFixtureOptions } from './harness.js';

function options(): E2EFixtureOptions {
  const probe = process.env.TMT_TEST_TMUX_PROBE;
  if (!probe) throw new Error('Native transport E2E requires the Docker-built adapter probe.');
  return { mode: 'input-log', executableEnv: { TMT_TEST_CLI: probe } };
}

function traceStages(fixture: E2EFixture, start: number): string[] {
  return fixture
    .transportTrace()
    .slice(start)
    .map((line) => line.split('|', 1)[0]);
}

function inputLines(fixture: E2EFixture, pid: number): (string | undefined)[] {
  return fixture
    .events()
    .filter((event) => event.event === 'input' && event.pid === pid)
    .map((event) => event.line);
}

describe('transport adapter probe: delivery stages and capture', { concurrent: false }, () => {
  it.each([false, true])(
    'delivers protected text once and preserves unrelated buffers (fallback: %s)',
    async (fallback) => {
      await withE2EFixture(async (fixture) => {
        const sentinel = 'tmt-e2e-native-transport-sentinel';
        fixture.tmux(['set-buffer', '-b', sentinel, '--', 'keep-me']);
        // Non-ASCII text is intentional transport data, not repository prose.
        const message =
          '! Native 日本語\nEnter C-c --leading-dash "quotes" $dollar `literal`\nend!';
        const start = fixture.transportTrace().length;
        const result = await fixture.runJsonCli(
          ['send', fixture.socketPath, fixture.pane, message],
          fallback ? { transportFault: { stage: 'set-buffer' } } : { transportTrace: true }
        );
        expect(result).toMatchObject({
          code: 0,
          stderr: '',
          json: { sent: true, commandCount: fallback ? 4 : 3 },
        });
        await fixture.waitForEvent(
          (event) =>
            event.event === 'input' && event.pid === fixture.panePid && event.line === 'end！'
        );
        expect(inputLines(fixture, fixture.panePid).filter((line) => line !== '')).toEqual([
          '！ Native 日本語',
          'Enter C-c --leading-dash "quotes" $dollar `literal`',
          'end！',
        ]);
        expect(traceStages(fixture, start)).toEqual(
          fallback
            ? [
                'set-buffer.fault-before',
                'literal-input.before',
                'literal-input.after.0',
                'submit.before',
                'submit.after.0',
              ]
            : [
                'set-buffer.before',
                'set-buffer.after.0',
                'paste-buffer.before',
                'paste-buffer.after.0',
                'submit.before',
                'submit.after.0',
              ]
        );
        expect(fixture.tmux(['show-buffer', '-b', sentinel]).trimEnd()).toBe('keep-me');
        expect(fixture.tmux(['list-buffers', '-F', '#{buffer_name}']).trim()).toBe(sentinel);
        expect(fs.existsSync(path.join(fixture.globalDir, 'tmux-team.db'))).toBe(false);
      }, options());
    }
  );

  it.each(['paste', 'submit'] as const)(
    'reports uncertain %s after real pane delivery without replay',
    async (stage) => {
      await withE2EFixture(async (fixture) => {
        const start = fixture.transportTrace().length;
        const result = await fixture.runJsonCli(
          ['send', fixture.socketPath, fixture.pane, 'causal-input!'],
          { transportFault: { stage } }
        );
        expect(result).toMatchObject({
          code: 1,
          stderr: '',
          json: {
            error: {
              code: 'DELIVERY_UNCERTAIN',
              message: `Message delivery is uncertain during ${stage}.`,
            },
          },
        });
        await fixture.waitForEvent(
          (event) =>
            event.event === 'input' &&
            event.pid === fixture.panePid &&
            event.line === 'causal-input！'
        );
        expect(
          inputLines(fixture, fixture.panePid).filter((line) => line === 'causal-input！')
        ).toHaveLength(1);
        expect(traceStages(fixture, start)).toEqual(
          stage === 'paste'
            ? [
                'set-buffer.before',
                'set-buffer.after.0',
                'paste-buffer.before',
                'paste-buffer.after.0',
                'paste-buffer.fault-after',
              ]
            : [
                'set-buffer.before',
                'set-buffer.after.0',
                'paste-buffer.before',
                'paste-buffer.after.0',
                'submit.before',
                'submit.after.0',
                'submit.fault-after',
              ]
        );
        expect(fixture.tmux(['list-buffers', '-F', '#{buffer_name}']).trim()).toBe('');
      }, options());
    }
  );

  it('captures diagnostic text independently and fails for a missing pane', async () => {
    await withE2EFixture(async (fixture) => {
      const marker = 'native-diagnostic-marker';
      expect(
        await fixture.runJsonCli(['send', fixture.socketPath, fixture.pane, marker])
      ).toMatchObject({ code: 0, json: { sent: true } });
      await fixture.waitForEvent(
        (event) => event.event === 'input' && event.pid === fixture.panePid && event.line === marker
      );
      await fixture.waitForCapture((output) => output.includes(marker));
      const expected = fixture.tmux(['capture-pane', '-t', fixture.pane, '-p', '-S', '-0']);
      expect(expected).toContain(marker);
      const result = await fixture.runJsonCli(['capture', fixture.socketPath, fixture.pane, '0']);
      expect(result).toMatchObject({
        code: 0,
        stderr: '',
        json: { output: expected, commandCount: 1 },
      });
      const missing = await fixture.runJsonCli([
        'send',
        fixture.socketPath,
        '%999999',
        'must-not-arrive',
      ]);
      expect(missing).toMatchObject({ code: 1, json: { error: { code: 'DELIVERY_UNCERTAIN' } } });
      expect(inputLines(fixture, fixture.panePid)).not.toContain('must-not-arrive');
      expect(fixture.tmux(['list-buffers', '-F', '#{buffer_name}']).trim()).toBe('');
      const captureMissing = await fixture.runJsonCli([
        'capture',
        fixture.socketPath,
        '%999999',
        '20',
      ]);
      expect(captureMissing).toMatchObject({ code: 1, json: { error: { code: 'TMUX_ERROR' } } });
      expect(captureMissing.json).not.toHaveProperty('output');
    }, options());
  });
});
