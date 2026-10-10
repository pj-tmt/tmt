import { describe, expect, it } from 'vite-plus/test';
import path from 'node:path';
import { withE2EFixture } from './harness.js';
import { installTmuxTrace } from './tmux-trace.js';

describe('tmux invocation trace', { concurrent: false }, () => {
  it('refuses foreign socket metadata writes while allowing the private socket', async () => {
    await withE2EFixture(async (fixture) => {
      const foreignSocket = path.join(fixture.root, 'foreign.sock');
      for (const prefix of [
        ['-S', foreignSocket],
        [`-S${foreignSocket}`],
        ['-L', 'foreign-server'],
        ['-Lforeign-server'],
        ['-c', 'true', '-S', foreignSocket],
        ['-T', 'RGB', '-S', foreignSocket],
        ['-ctrue', '-S', foreignSocket],
        ['-TRGB', '-S', foreignSocket],
      ]) {
        expect(() =>
          fixture.tmux([...prefix, 'set-option', '-p', '-t', fixture.pane, '@tmt.agent', 'foreign'])
        ).toThrow('E2E fixture refuses a foreign tmux socket');
        expect(fixture.paneMetadata()).toBe('');
      }
      for (const option of ['-uS', '-uT']) {
        expect(() => fixture.tmux([option, foreignSocket])).toThrow(
          'E2E fixture refuses combined tmux socket options'
        );
      }
      expect(() => fixture.tmux(['-S'])).toThrow('E2E fixture requires a tmux option value');
      fixture.tmux([
        '-T',
        'RGB',
        '-S',
        fixture.socketPath,
        'set-option',
        '-p',
        '-t',
        fixture.pane,
        '@tmt.agent',
        'private',
      ]);
      expect(fixture.paneMetadata()).toBe('private');
      fixture.tmux([
        '-S',
        fixture.socketPath,
        'set-option',
        '-p',
        '-u',
        '-t',
        fixture.pane,
        '@tmt.agent',
      ]);
      expect(fixture.paneMetadata()).toBe('');
    });
  });

  it.each(['ambient', 'explicit socket'])(
    'traces %s commands without changing multiline or option-like payloads',
    async (selection) => {
      await withE2EFixture(async (fixture) => {
        const trace = installTmuxTrace(fixture);
        const bufferName = `trace-baseline-${process.pid}`;
        const payload = 'first line\nsecond line\n<html>request body</html>\n-S not-a-socket';
        const prefix = selection === 'explicit socket' ? ['-S', fixture.socketPath] : [];

        try {
          trace.clear();
          fixture.tmux([...prefix, 'set-buffer', '-b', bufferName, '--', payload]);

          const invocations = trace.invocations();
          expect(invocations).toHaveLength(1);
          expect(trace.commands()).toEqual(['set-buffer']);
          expect(invocations[0]).toContain('first line second line');
          expect(invocations[0]).not.toContain('\n');

          const saved = fixture.tmux([...prefix, 'save-buffer', '-b', bufferName, '-']);
          expect(saved).toBe(payload);
          expect(trace.commands()).toEqual(['set-buffer', 'save-buffer']);
        } finally {
          try {
            fixture.tmux(['delete-buffer', '-b', bufferName]);
          } catch {
            // The fixture server still owns cleanup if set-buffer failed early.
          }
        }

        expect(fixture.tmux(['list-buffers', '-F', '#{buffer_name}'])).not.toContain(bufferName);
      });
    }
  );
});
