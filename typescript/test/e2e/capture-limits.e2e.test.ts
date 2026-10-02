import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import { withE2EFixture } from './harness.js';

describe('capture numeric contract', { concurrent: false }, () => {
  it('reports invalid loaded config without touching tmux or rewriting its config', async () => {
    await withE2EFixture(async (fixture) => {
      const configFile = path.join(fixture.globalDir, 'config.json');
      const configBytes = JSON.stringify({ defaults: { captureLines: '12junk' }, unrelated: true });
      fs.writeFileSync(configFile, configBytes);
      const result = await fixture.runJsonCli<{ error: { code: string } }>(['check', 'missing'], {
        withoutTmux: true,
      });
      expect(result).toMatchObject({
        code: 1,
        json: { error: { code: 'CONFIG_ERROR' } },
      });
      expect(fs.readFileSync(configFile, 'utf8')).toBe(configBytes);
      expect(fs.existsSync(path.join(fixture.globalDir, 'tmux-team.db'))).toBe(false);
      expect(fs.existsSync(fixture.forbiddenTmuxLogPath)).toBe(false);
    });
  });

  it('rejects invalid counts before creating storage or invoking tmux', async () => {
    await withE2EFixture(async (fixture) => {
      const database = path.join(fixture.globalDir, 'tmux-team.db');
      expect(fs.existsSync(database)).toBe(false);
      const metadata = fixture.paneMetadata(fixture.pane);
      for (const args of [
        ['check', fixture.pane, '9'.repeat(400)],
        ['read', fixture.pane, '--lines=2147483648'],
        ['--lines', '2147483648', 'check', fixture.pane, '0'],
      ]) {
        const result = await fixture.runJsonCli<{ error: { code: string } }>(args, {
          withoutTmux: true,
        });
        expect(result).toMatchObject({ code: 1, json: { error: { code: 'USAGE_ERROR' } } });
      }
      expect(fs.existsSync(database)).toBe(false);
      expect(fs.existsSync(fixture.forbiddenTmuxLogPath)).toBe(false);
      expect(fixture.paneMetadata(fixture.pane)).toBe(metadata);
    });
  });

  it('captures causal mock output at zero and the supported maximum without changing binding', async () => {
    await withE2EFixture(async (fixture) => {
      expect((await fixture.runJsonCli(['name', 'CapturePeer'])).code).toBe(0);
      const message = 'capture boundary response';
      expect(
        (await fixture.runJsonCli(['talk', 'CapturePeer', message, '--timeout', '10'])).code
      ).toBe(0);
      await fixture.waitForEvent((event) => event.event === 'summary' && event.message === message);
      const metadata = fixture.paneMetadata(fixture.pane);
      fs.writeFileSync(
        path.join(fixture.globalDir, 'config.json'),
        JSON.stringify({ defaults: { captureLines: 0 } })
      );
      for (const [command, value] of [
        ['check', '0'],
        ['read', '2147483647'],
      ] as const) {
        const result = await fixture.runJsonCli<{ lines: number; output: string }>([
          command,
          'CapturePeer',
          ...(value === '0' ? [] : [`--lines=${value}`]),
        ]);
        expect(result.code).toBe(0);
        expect(result.json?.lines).toBe(Number(value));
        expect(result.json?.output).toContain(`mock-agent summary: ${message}`);
      }
      expect(fixture.paneMetadata(fixture.pane)).toBe(metadata);
    });
  });
});
