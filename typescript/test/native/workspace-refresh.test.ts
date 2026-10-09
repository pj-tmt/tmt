import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { describe, expect, it } from 'vite-plus/test';
import { runCli, withSandbox } from '../support/cli-process.js';
import { installTmuxTripwire } from './tmux-tripwire.js';

describe('advisory workspace command refresh', () => {
  it('silently preserves complete text/JSON and failure codes when prior data refuses refresh', async () => {
    await withSandbox(async (sandbox) => {
      const tripwire = installTmuxTripwire(sandbox);
      const socket = path.join(sandbox.root, 'absent-socket');
      sandbox.env.TMUX = `${socket},10,0`;
      sandbox.env.TMUX_PANE = '%1';
      const directory = path.join(
        sandbox.globalDir,
        'workspace',
        createHash('sha256').update(socket).digest('hex')
      );
      fs.mkdirSync(directory, { recursive: true, mode: 0o700 });
      const latest = path.join(directory, 'latest.json');
      const unknown = '{"version":999}';
      fs.writeFileSync(latest, unknown, { mode: 0o600 });
      const commands = [
        ['identity', 'list'],
        ['identity', 'list', '--json'],
        ['identity', 'meta', 'get', 'missing', '--identity=missing'],
        ['identity', 'meta', 'get', 'missing', '--identity=missing', '--json'],
      ];
      for (const args of commands) {
        fs.writeFileSync(sandbox.globalConfig, '{"workspace":{"snapshotEnabled":false}}');
        const original = await runCli(sandbox, args);
        fs.writeFileSync(sandbox.globalConfig, '{"workspace":{"snapshotEnabled":true}}');
        const refreshed = await runCli(sandbox, args);
        expect(refreshed).toEqual(original);
        expect(refreshed.signal).toBeNull();
        expect(refreshed.status).toBe(args.includes('get') ? 3 : 0);
        expect(fs.readFileSync(latest, 'utf8')).toBe(unknown);
      }
      expect(fs.existsSync(tripwire)).toBe(false);
    });
  });

  it('skips snapshot filesystem effects outside a selected pane', async () => {
    await withSandbox(async (sandbox) => {
      const tripwire = installTmuxTripwire(sandbox);
      for (const coordinate of [undefined, 'malformed']) {
        if (coordinate === undefined) delete sandbox.env.TMUX;
        else sandbox.env.TMUX = coordinate;
        sandbox.env.TMUX_PANE = '%1';
        const result = await runCli(sandbox, ['identity', 'list', '--json']);
        expect(result.status).toBe(0);
        expect(result.stderr).toBe('');
        expect(fs.existsSync(path.join(sandbox.globalDir, 'workspace'))).toBe(false);
      }
      expect(fs.existsSync(tripwire)).toBe(false);
    });
  });
});
