import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import { withE2EFixture, type E2EFixture } from './harness.js';
import { durableState } from './identity-state-oracle.js';
import { installTmuxTrace } from './tmux-trace.js';

const inputLog = { mode: 'input-log' } as const;

interface Capture {
  target: string;
  pane: string;
  identity?: { name: string; canonicalName: string };
  lines: number;
  output: string;
}

async function seedDiagnostic(fixture: E2EFixture, message: string): Promise<void> {
  fixture.tmux(['send-keys', '-l', '-t', fixture.pane, '--', message]);
  fixture.tmux(['send-keys', '-t', fixture.pane, 'Enter']);
  await fixture.waitForEvent(
    (event) => event.event === 'input' && event.pid === fixture.panePid && event.line === message
  );
  // Capture is intentionally terminal diagnostics, not a final reply assertion.
  await fixture.waitForCapture((output) => output.includes(message));
}

describe('current-server diagnostic routing', { concurrent: false }, () => {
  it('captures names, stable panes and tmux targets with configured and explicit line counts', async () => {
    await withE2EFixture(async (fixture) => {
      expect(await fixture.runJsonCli(['name', 'Alice'])).toMatchObject({
        code: 0,
        json: { bound: true },
      });
      const marker = 'native-check-causal-marker';
      await seedDiagnostic(fixture, marker);
      fs.writeFileSync(
        path.join(fixture.globalDir, 'config.json'),
        JSON.stringify({ defaults: { captureLines: 0 } })
      );
      const metadata = fixture.paneMetadata();
      for (const target of ['aLiCe', fixture.pane, fixture.paneTarget(fixture.pane)]) {
        const captured = await fixture.runJsonCli<Capture>(['check', target]);
        expect(captured).toMatchObject({
          code: 0,
          stderr: '',
          json: {
            target,
            pane: fixture.pane,
            identity: { name: 'Alice', canonicalName: 'alice' },
            lines: 0,
            output: expect.stringContaining(marker),
          },
        });
        expect(Object.keys(captured.json ?? {}).sort()).toEqual([
          'identity',
          'lines',
          'output',
          'pane',
          'target',
        ]);
      }
      const maximum = await fixture.runJsonCli<Capture>(['read', 'Alice', '--lines=2147483647']);
      expect(maximum).toMatchObject({
        code: 0,
        json: { lines: 2147483647, output: expect.stringContaining(marker) },
      });
      expect(fixture.paneMetadata()).toBe(metadata);
      const human = await fixture.runCli(['check', 'Alice', '--lines', '0']);
      expect(human.code).toBe(0);
      expect(human.stderr).toBe('');
      expect(human.stdout).toContain(`OUTPUT Alice ${fixture.pane}\n`);
      expect(human.stdout).toContain(marker);
      expect(await fixture.runJsonCli(['unbind'])).toMatchObject({ code: 0 });
      const unbound = await fixture.runJsonCli<Capture>(['read', fixture.pane, '0']);
      expect(unbound).toMatchObject({
        code: 0,
        json: { pane: fixture.pane, output: expect.stringContaining(marker) },
      });
      expect(unbound.json).not.toHaveProperty('identity');
      const missing = await fixture.runJsonCli(['check', '%999999']);
      expect(missing).toMatchObject({ code: 3, json: { error: { code: 'PANE_NOT_FOUND' } } });
    }, inputLog);
  });

  it.each([false, true])(
    'does not route to an equal pane ID on a foreign server (copied UUID: %s)',
    async (copyUuid) => {
      await withE2EFixture(async (first) => {
        expect(await first.runJsonCli(['name', 'Foreign'])).toMatchObject({ code: 0 });
        const serverId = JSON.parse(first.paneMetadata()).globalIdentity.serverId;
        await withE2EFixture(
          async (second) => {
            expect(second.pane).toBe(first.pane);
            if (copyUuid) second.tmux(['set-option', '-s', '@tmux-team.server-id', serverId]);
            expect(await second.runJsonCli(['list', 'Foreign'])).toMatchObject({
              code: 0,
              json: { identity: { name: 'Foreign' }, presence: 'active' },
            });
            const before = durableState(first);
            const trace = installTmuxTrace(second);
            const result = await second.runJsonCli(['check', 'Foreign'], { outsideTmux: true });
            expect(result).toMatchObject({
              code: 3,
              stderr: '',
              json: { error: { code: 'NAME_NOT_FOUND' } },
            });
            expect(trace.invocations().some((line) => line.includes('capture-pane'))).toBe(false);
            expect(durableState(first)).toEqual(before);
            expect(second.paneMetadata()).toBe('');
          },
          { ...inputLog, globalDir: first.globalDir }
        );
      }, inputLog);
    }
  );

  it('keeps selected-name inspection bounded among unrelated unbound panes', async () => {
    await withE2EFixture(async (fixture) => {
      expect(await fixture.runJsonCli(['name', 'Scoped'])).toMatchObject({ code: 0 });
      await seedDiagnostic(fixture, 'scoped-native-check');
      const trace = installTmuxTrace(fixture);
      const capture = async () => {
        trace.clear();
        const result = await fixture.runJsonCli<Capture>(['check', 'Scoped'], {
          outsideTmux: true,
        });
        expect(result).toMatchObject({
          code: 0,
          json: { output: expect.stringContaining('scoped-native-check') },
        });
        return trace.invocations();
      };
      const small = await capture();
      for (let index = 0; index < 30; index++)
        fixture.tmux(['new-window', '-d', '-t', 'e2e:', '-n', `unbound-${index}`, 'sleep', '300']);
      const large = await capture();
      expect(large).toHaveLength(small.length);
      expect(small).toHaveLength(3);
      expect(large.filter((line) => line.includes('capture-pane'))).toHaveLength(1);
      const paneQueries = large.filter((line) => line.includes('list-panes'));
      expect(paneQueries).toHaveLength(1);
      expect(paneQueries[0]).toContain(' -f ');
      expect(paneQueries[0]).toContain(fixture.pane);
    }, inputLog);
  });

  it.each(['exit', 'overflow', 'timeout'] as const)(
    'returns failure without partial output after capture %s',
    async (fault) => {
      await withE2EFixture(async (fixture) => {
        const result = await fixture.runJsonCli<Capture>(['check', fixture.pane], {
          captureFault: fault,
        });
        expect(result).toMatchObject({ code: 1, stderr: '', json: { error: { code: 'ERROR' } } });
        expect(result.json).not.toHaveProperty('output');
        expect(result.stdout).not.toContain('\u0000');
        expect(fixture.paneMetadata()).toBe('');
      }, inputLog);
    }
  );
});
