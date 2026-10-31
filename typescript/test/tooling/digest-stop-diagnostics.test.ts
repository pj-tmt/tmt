import { expect, it } from 'vite-plus/test';
import { digestStopDiagnostics } from '../e2e/digest-stop-diagnostics.js';

it('names fresh/resume and the hook outcome without printing argv, paths or provider text', () => {
  const phase = {
    provider: 'codex' as const,
    channel: false,
    setup: true,
    resume: true,
    legacy: false,
    step: 'boundary',
  };
  const result = digestStopDiagnostics(
    phase,
    [
      {
        status: 'admitted',
        step: 'context',
        reason: 'worker_error',
        detail: 'digest_stop:worker:open:busy',
        secret: 'private-settings',
      },
    ],
    [
      {
        event: 'digest-handler',
        step: 'boundary',
        ok: true,
        exitCode: 0,
        stdout: 'private-provider-text',
        stderr: 'private-error-path',
        command: 'private-argv',
      },
    ]
  );
  expect(result).toEqual({
    phase,
    learning: [
      {
        status: 'admitted',
        step: 'context',
        reason: 'worker_error',
        detail: 'digest_stop:worker:open:busy',
      },
    ],
    handlers: [
      {
        event: 'digest-handler',
        step: 'boundary',
        ok: true,
        exitCode: 0,
        stdoutBytes: 21,
        stderrBytes: 18,
      },
    ],
  });
  expect(JSON.stringify(result)).not.toContain('private-');
  expect(digestStopDiagnostics({ ...phase, resume: false }, [], []).phase).toMatchObject({
    resume: false,
  });
});

it('caps histories and labels and excludes unrelated provider events', () => {
  const phase = {
    provider: 'claude' as const,
    channel: true,
    setup: false,
    resume: false,
    legacy: false,
    step: 'x'.repeat(300),
  };
  const outcome = {
    event: 'hook-outcome',
    hookEvent: 'Stop',
    ok: false,
    exitCode: 1,
    stdoutBytes: 0,
    stderrBytes: 4,
  };
  const result = digestStopDiagnostics(
    phase,
    Array.from({ length: 30 }, () => ({ detail: 'x'.repeat(300) })),
    [
      ...Array.from({ length: 30 }, () => outcome),
      { event: 'paste', content: 'private-provider-text' },
    ]
  );
  expect(result.handlers).toHaveLength(8);
  expect(result.learning).toHaveLength(8);
  expect(result.phase).toMatchObject({ step: 'x'.repeat(96) });
  expect(JSON.stringify(result)).not.toContain('private-provider-text');
  expect(JSON.stringify(result).length).toBeLessThan(3000);
});
