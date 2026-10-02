import {
  existsSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  readlinkSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import { expectError, parseWholeStdout, runCli, withSandbox } from '../support/cli-process.js';

import {
  CLAUDE_ORIGINAL,
  INSTALL_BUDGET_MS,
  paths,
  setUpMachine,
  tree,
} from '../support/native-uninstall.js';

describe('tmt uninstall on a disposable HOME and prefix', () => {
  it('removes everything TMT installed, keeps data and user files, and repeats as a no-op', async () => {
    await withSandbox(async (sandbox) => {
      const tmt = await setUpMachine(sandbox);
      const { prefix, claudeSettings, codexHooks, userSkill, record } = paths(sandbox);
      expect(readFileSync(claudeSettings, 'utf8')).toContain('__hook claude');
      expect(existsSync(codexHooks)).toBe(true);
      expect(existsSync(record)).toBe(true);
      expect(lstatSync(path.join(sandbox.home, '.claude/skills/tmux-team')).isSymbolicLink()).toBe(
        true
      );

      const result = await runCli(tmt, ['uninstall', '--yes', '--json'], {
        deadlineMs: INSTALL_BUDGET_MS,
      });
      expect(result.status, result.stdout + result.stderr).toBe(0);
      const report = parseWholeStdout(result);
      // Setup and uninstall each back up the Claude settings they changed.
      const backup = 'backup of your settings before TMT removed its hooks';
      const kept = report.kept as { path: string; reason: string }[];
      expect(kept.map((item) => item.reason)).toEqual([backup, backup]);
      for (const item of kept) {
        expect(path.dirname(item.path)).toBe(path.dirname(claudeSettings));
        expect(existsSync(item.path)).toBe(true);
      }
      expect(report.officeStopped).toBe(false);
      expect(report.data).toEqual({ path: sandbox.globalDir, deleted: false });

      expect(readFileSync(claudeSettings, 'utf8')).toBe(CLAUDE_ORIGINAL);
      expect(existsSync(codexHooks)).toBe(false);
      // Setup's lock goes with the last TMT hook in each directory.
      for (const settings of [claudeSettings, codexHooks]) {
        expect(existsSync(path.join(path.dirname(settings), '.tmt-setup.lock'))).toBe(false);
      }
      expect(readFileSync(userSkill, 'utf8')).toBe('user skill\n');
      for (const skills of ['.claude/skills', '.agents/skills']) {
        const names = existsSync(path.join(sandbox.home, skills))
          ? readdirSync(path.join(sandbox.home, skills))
          : [];
        expect(names.filter((name) => name !== 'user-skill')).toEqual([]);
      }
      expect(tree(prefix)).toEqual({});
      expect(existsSync(record)).toBe(false);
      expect(existsSync(path.join(sandbox.globalDir, 'skill-assets'))).toBe(false);
      expect(existsSync(sandbox.globalDir)).toBe(true);

      // Nothing is left to remove: the development CLI reports a no-op.
      const again = await runCli(sandbox, ['uninstall', '--prefix', prefix, '--yes', '--json']);
      expect(again.status, again.stderr).toBe(0);
      expect(parseWholeStdout(again).removed).toEqual([]);
    });
  });

  it('deletes nothing without consent, with or without --purge', async () => {
    await withSandbox(async (sandbox) => {
      const tmt = await setUpMachine(sandbox);
      const before = tree(sandbox.root);
      for (const args of [['uninstall'], ['uninstall', '--purge'], ['uninstall', '--json']]) {
        const refused = await runCli(tmt, args);
        expect(refused.status).toBe(1);
        expect(tree(sandbox.root)).toEqual(before);
      }
      expectError(
        await runCli(tmt, ['uninstall', '--purge', '--json']),
        'UNINSTALL_CONSENT_REQUIRED'
      );
      expect(tree(sandbox.root)).toEqual(before);
    });
  });

  it('offers --purge before consent and names no tmt command once tmt is gone', async () => {
    await withSandbox(async (sandbox) => {
      const tmt = await setUpMachine(sandbox);
      const refused = await runCli(tmt, ['uninstall']);
      expect(refused.status).toBe(1);
      expect(refused.stdout).toContain('add --purge to delete them too');
      const result = await runCli(tmt, ['uninstall', '--yes'], { deadlineMs: INSTALL_BUDGET_MS });
      expect(result.status, result.stdout + result.stderr).toBe(0);
      const hint = result.stdout.split('\n').find((line) => line.includes('hint:'));
      expect(hint).toContain(sandbox.globalDir);
      expect(hint).toContain('delete that directory if you no longer need it');
      expect(hint).not.toContain('tmt ');
      expect(existsSync(sandbox.globalDir)).toBe(true);
    });
  });

  it('deletes the data directory only with --purge', async () => {
    await withSandbox(async (sandbox) => {
      const tmt = await setUpMachine(sandbox);
      const result = await runCli(tmt, ['uninstall', '--purge', '--yes', '--json'], {
        deadlineMs: INSTALL_BUDGET_MS,
      });
      expect(result.status, result.stdout + result.stderr).toBe(0);
      expect(parseWholeStdout(result).data).toEqual({ path: sandbox.globalDir, deleted: true });
      expect(existsSync(sandbox.globalDir)).toBe(false);
      expect(readFileSync(paths(sandbox).userSkill, 'utf8')).toBe('user skill\n');
    });
  });

  it('keeps an edited hook, adopts hooks without a record and fails closed on a bad record', async () => {
    await withSandbox(async (sandbox) => {
      const tmt = await setUpMachine(sandbox);
      const { claudeSettings, codexHooks, record, prefix } = paths(sandbox);
      // An unreadable record stops the run before any change.
      const recordBytes = readFileSync(record, 'utf8');
      writeFileSync(record, '{"version":9}');
      const before = tree(sandbox.root);
      expectError(await runCli(tmt, ['uninstall', '--yes', '--json']), 'UNINSTALL_ERROR');
      expect(tree(sandbox.root)).toEqual(before);
      // Hooks installed before the record existed are recognized exactly.
      rmSync(record);
      const edited = readFileSync(claudeSettings, 'utf8').replace(
        '__hook claude',
        '__hook claude --edited'
      );
      writeFileSync(claudeSettings, edited);
      const result = await runCli(tmt, ['uninstall', '--yes', '--json'], {
        deadlineMs: INSTALL_BUDGET_MS,
      });
      expect(result.status, result.stdout + result.stderr).toBe(0);
      expect(parseWholeStdout(result).kept).toContainEqual({
        path: claudeSettings,
        reason: 'a TMT hook in it was edited',
      });
      expect(readFileSync(claudeSettings, 'utf8')).toBe(edited);
      expect(existsSync(path.join(path.dirname(claudeSettings), '.tmt-setup.lock'))).toBe(true);
      expect(existsSync(codexHooks)).toBe(false);
      expect(existsSync(path.join(path.dirname(codexHooks), '.tmt-setup.lock'))).toBe(false);
      expect(tree(prefix)).toEqual({});
      expect(recordBytes).toContain('"codex"');
    });
  });

  it('keeps a same-named command that is not TMT', async () => {
    await withSandbox(async (sandbox) => {
      const prefix = paths(sandbox).prefix;
      mkdirSync(path.join(prefix, 'bin'), { recursive: true });
      symlinkSync('/usr/bin/true', path.join(prefix, 'bin', 'tmt'));
      const result = await runCli(sandbox, ['uninstall', '--prefix', prefix, '--yes', '--json']);
      expect(result.status, result.stdout + result.stderr).toBe(0);
      expect(readlinkSync(path.join(prefix, 'bin', 'tmt'))).toBe('/usr/bin/true');
    });
  });
});
