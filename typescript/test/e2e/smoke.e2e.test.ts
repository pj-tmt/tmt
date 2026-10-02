import fs from 'node:fs';
import { describe, expect, it } from 'vitest';
import { E2EFixture, withE2EFixture } from './harness.js';

interface TalkResult {
  status?: string;
  requestId?: string;
  response?: string;
  error?: { code: string };
}

describe('Docker/Vitest tmux foundation smoke scenarios', { concurrent: false }, () => {
  it('propagates real CLI stdout, stderr, and exit codes', async () => {
    await withE2EFixture(async (fixture) => {
      const version = await fixture.runCli(['--version']);
      expect(version.code).toBe(0);
      expect(version.stdout.trim()).toMatch(/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/);

      const help = await fixture.runCli(['--help']);
      expect(help.code).toBe(0);
      expect(help.stdout).toContain('TMT native alpha');

      const invalid = await fixture.runCli(['definitely-not-a-command']);
      expect(invalid.code).not.toBe(0);
      expect(invalid.stderr.toLowerCase()).toContain('unrecognized subcommand');
    });
  });

  it('transports a durable request and records request, submission, and summary through real tmux', async () => {
    await withE2EFixture(async (fixture) => {
      expect(fixture.serverIsRunning()).toBe(true);
      expect(fixture.tmux(['list-panes', '-a']).trim()).toContain(fixture.pane);

      const message = 'hello from the foundation';
      const result = await fixture.runJsonCli<TalkResult>([
        'talk',
        fixture.pane,
        message,
        '--no-preamble',
        '--timeout',
        '5',
      ]);
      expect(result.code).toBe(0);
      expect(result.json).toMatchObject({
        status: 'completed',
        response: `mock-agent response: ${message}`,
      });
      const request = await fixture.waitForEvent(
        (event) => event.event === 'request' && event.requestId === result.json?.requestId
      );
      const submitted = await fixture.waitForEvent(
        (event) => event.event === 'submitted' && event.requestId === result.json?.requestId
      );
      const summary = await fixture.waitForEvent(
        (event) => event.event === 'summary' && event.requestId === result.json?.requestId
      );
      expect(request.message).toBe(message);
      expect(submitted.body).toBe(result.json?.response);
      expect(summary.message).toBe(message);
      const events = fixture.events();
      const requestIndex = events.findIndex(
        (event) => event.event === 'request' && event.requestId === result.json?.requestId
      );
      const submittedIndex = events.findIndex(
        (event) => event.event === 'submitted' && event.requestId === result.json?.requestId
      );
      const summaryIndex = events.findIndex(
        (event) => event.event === 'summary' && event.requestId === result.json?.requestId
      );
      expect(requestIndex).toBeGreaterThanOrEqual(0);
      expect(submittedIndex).toBeGreaterThan(requestIndex);
      expect(summaryIndex).toBeGreaterThan(submittedIndex);
    });
  });

  it('records a silent mock-agent lifecycle without fabricating a response', async () => {
    await withE2EFixture(
      async (fixture) => {
        const result = await fixture.runJsonCli<TalkResult>([
          'talk',
          fixture.pane,
          'silent request',
          '--no-preamble',
          '--timeout',
          '1',
        ]);
        expect(result.code).toBe(4);
        expect(result.json).toMatchObject({ status: 'timeout', error: { code: 'TIMEOUT' } });
        await fixture.waitForEvent(
          (event) => event.event === 'silent' && event.requestId === result.json?.requestId
        );
        const events = fixture.events();
        expect(events.map((event) => event.event)).toEqual(['ready', 'request', 'silent']);
        expect(fixture.capture()).not.toContain('mock-agent response: silent request');
      },
      { mode: 'silent' }
    );
  });

  it('records malformed output separately from a valid response', async () => {
    await withE2EFixture(
      async (fixture) => {
        const process = fixture.runCliProcess<TalkResult>([
          '--json',
          'talk',
          fixture.pane,
          'malformed request',
          '--no-preamble',
          '--timeout',
          '1',
        ]);
        const malformed = await fixture.waitForEvent((event) => event.event === 'malformed');
        const result = await process.result;
        expect(result.code).toBe(4);
        expect(result.json).toMatchObject({ status: 'timeout', error: { code: 'TIMEOUT' } });
        const events = fixture.events();
        expect(events.map((event) => event.event)).toEqual(['ready', 'request', 'malformed']);
        expect(malformed.message).toBe('malformed request');
        expect(fixture.capture()).not.toContain('RESPONSE-END-');
      },
      { mode: 'malformed' }
    );
  });

  it('keeps the canonical pane ID across a real tmux move', async () => {
    await withE2EFixture((fixture) => {
      const originalPane = fixture.pane;
      const originalTarget = fixture.paneTarget(originalPane);
      const targetPane = fixture
        .tmux(['new-window', '-d', '-P', '-F', '#{pane_id}', '-t', 'e2e', '-n', 'sink', 'sleep 30'])
        .trim();
      fixture.tmux(['move-pane', '-s', originalPane, '-t', targetPane]);
      const movedTarget = fixture.paneTarget(originalPane);
      const sinkTarget = fixture.paneTarget(targetPane);

      expect(fixture.pane).toBe(originalPane);
      expect(movedTarget).not.toBe(originalTarget);
      expect(movedTarget.split('.')[0]).toBe(sinkTarget.split('.')[0]);
      expect(fixture.tmux(['list-panes', '-a', '-F', '#{pane_id}']).trim()).toContain(originalPane);
    });
  });

  it('cleans the private server and files after a thrown scenario error', async () => {
    let failedFixture: E2EFixture | undefined;
    await expect(
      withE2EFixture((fixture) => {
        failedFixture = fixture;
        throw new Error('simulated scenario failure');
      })
    ).rejects.toThrow('simulated scenario failure');

    expect(failedFixture).toBeDefined();
    expect(failedFixture?.serverIsRunning()).toBe(false);
    expect(failedFixture?.mockProcessIsRunning()).toBe(false);
    expect(fs.existsSync(failedFixture?.socketPath ?? '')).toBe(false);
    expect(fs.existsSync(failedFixture?.root ?? '')).toBe(false);
    expect(fs.existsSync(failedFixture?.socketRoot ?? '')).toBe(false);

    let secondFixture: E2EFixture | undefined;
    await withE2EFixture((fixture) => {
      secondFixture = fixture;
      expect(fixture.serverIsRunning()).toBe(true);
      expect(fixture.socket).not.toBe(failedFixture?.socket);
      expect(fixture.root).not.toBe(failedFixture?.root);
    });
    expect(secondFixture?.serverIsRunning()).toBe(false);
    expect(secondFixture?.mockProcessIsRunning()).toBe(false);
  });
});
