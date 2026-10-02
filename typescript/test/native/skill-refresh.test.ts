import { createHash } from 'node:crypto';
import {
  existsSync,
  mkdirSync,
  readFileSync,
  readlinkSync,
  realpathSync,
  symlinkSync,
  unlinkSync,
  writeFileSync,
} from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import { expectError, parseWholeStdout, runCli, withSandbox } from '../support/cli-process.js';
import { calibrateTmuxTripwire } from './tmux-tripwire.js';

describe('native managed skill refresh', () => {
  it('keeps a never-installed home unchanged and hides internal grammar', async () => {
    await withSandbox(async (sandbox) => {
      const tripwire = await calibrateTmuxTripwire(sandbox);
      const baseline = readFileSync(tripwire);
      const result = await runCli(sandbox, ['__native-refresh-skills', '--json']);
      expect(result.status).toBe(0);
      expect(result.stderr).toBe('');
      expect(parseWholeStdout(result)).toEqual({ refreshed: [], skipped: [], conflicts: [] });
      for (const args of [[], ['completion', 'bash'], ['completion', 'zsh']]) {
        const help = await runCli(sandbox, args);
        expect(help.status).toBe(0);
        expect(help.stdout).not.toContain('__native-refresh-skills');
      }
      for (const args of [['--force'], ['--dir', 'new-skills'], ['claude']]) {
        const rejected = await runCli(sandbox, ['__native-refresh-skills', ...args, '--json']);
        expect(rejected.status).toBe(1);
        expectError(rejected, 'USAGE_ERROR');
      }
      expect(existsSync(sandbox.globalDir)).toBe(false);
      expect(existsSync(sandbox.database)).toBe(false);
      expect(readFileSync(tripwire)).toEqual(baseline);
    });
  });

  it('refreshes distinct old bytes, preserves conflicts, and reports a truthful retry', async () => {
    await withSandbox(async (sandbox) => {
      const tripwire = await calibrateTmuxTripwire(sandbox);
      const baseline = readFileSync(tripwire);
      const current = await runCli(sandbox, ['learn', '--skill']);
      expect(current.status).toBe(0);
      const old = Buffer.from(
        '---\nname: tmux-team\ndescription: Old fixture guidance.\n---\nOld version instructions.\n'
      );
      expect(current.stdout).not.toBe(old.toString());
      mkdirSync(sandbox.globalDir, { recursive: true });
      const global = realpathSync(sandbox.globalDir);
      const oldSource = path.join(
        global,
        'skill-assets',
        createHash('sha256').update(old).digest('hex'),
        'tmux-team'
      );
      mkdirSync(oldSource, { recursive: true });
      writeFileSync(path.join(oldSource, 'SKILL.md'), old);
      const targets = ['a-conflict', 'b-custom skills', 'c-deleted'].map((name) =>
        path.join(sandbox.cwd, name, 'tmux-team')
      );
      for (const target of targets.slice(0, 2))
        mkdirSync(path.dirname(target), { recursive: true });
      writeFileSync(targets[0], 'user-owned content');
      symlinkSync(oldSource, targets[1]);
      const registry = path.join(global, 'skill-installations.json');
      const intents = JSON.stringify({ version: 1, targets });
      writeFileSync(registry, intents);
      writeFileSync(sandbox.globalConfig, '{ malformed configuration must not be loaded');
      writeFileSync(sandbox.localConfig, '{ malformed local configuration');

      const result = await runCli(sandbox, ['__native-refresh-skills', '--json']);
      expect(result.status).toBe(1);
      expect(result.stderr).toBe('');
      expectError(result, 'SKILL_REFRESH_FAILED');
      expect(parseWholeStdout(result)).toMatchObject({
        refreshed: [{ target: targets[1], changed: true }],
        skipped: [targets[2]],
        conflicts: [targets[0]],
      });
      expect(readFileSync(path.join(targets[1], 'SKILL.md'), 'utf8')).toBe(current.stdout);
      expect(readlinkSync(targets[1])).not.toBe(oldSource);
      expect(readFileSync(path.join(oldSource, 'SKILL.md'))).toEqual(old);
      expect(readFileSync(targets[0], 'utf8')).toBe('user-owned content');
      expect(existsSync(targets[2])).toBe(false);
      expect(readFileSync(registry, 'utf8')).toBe(intents);

      unlinkSync(targets[0]);
      const retry = await runCli(sandbox, ['__native-refresh-skills', '--json']);
      expect(retry.status).toBe(0);
      expect(parseWholeStdout(retry)).toEqual({
        refreshed: [{ target: targets[1], changed: false }],
        skipped: [targets[0], targets[2]],
        conflicts: [],
      });
      const human = await runCli(sandbox, ['__native-refresh-skills']);
      expect(human.status).toBe(0);
      expect(human.stdout).toContain('hint: reload or restart your agent');
      expect(existsSync(sandbox.database)).toBe(false);
      expect(readFileSync(tripwire)).toEqual(baseline);
    });
  });
});
