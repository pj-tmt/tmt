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
import { describe, expect, it } from 'vitest';
import {
  expectError,
  parseWholeStdout,
  runCli,
  withSandbox,
  type Sandbox,
} from '../support/cli-process.js';
import { createArtifact } from '../support/native-artifact.js';

// Every path here is under the sandbox: a disposable HOME, XDG root and prefix.
const INSTALL_BUDGET_MS = 30_000;
const USER_HOOK = '{ "hooks" : [{ "type": "command", "command": "user-command" }] }';
const CLAUDE_ORIGINAL = `{\n "permissions": {"allow": ["Bash(git *)"]},\n "hooks": {"SessionStart": [${USER_HOOK}]}\n}\n`;

/** Files and symlinks (by target) under `root`, without following links. */
function tree(root: string): Record<string, string> {
  const snapshot: Record<string, string> = {};
  const visit = (current: string): void => {
    for (const entry of readdirSync(current)) {
      const entryPath = path.join(current, entry);
      const stat = lstatSync(entryPath);
      const key = path.relative(root, entryPath);
      if (stat.isSymbolicLink()) snapshot[key] = `-> ${readlinkSync(entryPath)}`;
      else if (stat.isDirectory()) visit(entryPath);
      else snapshot[key] = readFileSync(entryPath, 'utf8');
    }
  };
  if (existsSync(root)) visit(root);
  return snapshot;
}

function paths(sandbox: Sandbox) {
  const prefix = path.join(sandbox.root, 'install prefix');
  return {
    prefix,
    tmt: path.join(prefix, 'bin', 'tmt'),
    claudeSettings: path.join(sandbox.home, '.claude', 'settings.json'),
    codexHooks: path.join(sandbox.home, '.codex', 'hooks.json'),
    userSkill: path.join(sandbox.home, '.claude', 'skills', 'user-skill', 'SKILL.md'),
    record: path.join(sandbox.globalDir, 'setup-record.json'),
  };
}

/** A sandbox whose commands run the TMT installed in its prefix. */
function installed(sandbox: Sandbox): Sandbox {
  const { prefix, tmt } = paths(sandbox);
  sandbox.env.PATH = `${path.join(prefix, 'bin')}${path.delimiter}/usr/bin${path.delimiter}/bin`;
  return { ...sandbox, cli: { executable: tmt, args: [] } };
}

async function setUpMachine(sandbox: Sandbox): Promise<Sandbox> {
  const { prefix, claudeSettings, userSkill } = paths(sandbox);
  mkdirSync(path.dirname(claudeSettings), { recursive: true });
  writeFileSync(claudeSettings, CLAUDE_ORIGINAL);
  mkdirSync(path.join(sandbox.home, '.codex'));
  mkdirSync(path.dirname(userSkill), { recursive: true });
  writeFileSync(userSkill, 'user skill\n');
  const fixture = await createArtifact(sandbox, '5.0.0-alpha.90');
  const install = await runCli(
    sandbox,
    [
      '__native-install',
      '--archive',
      fixture.archive,
      '--manifest',
      fixture.manifest,
      '--prefix',
      prefix,
      '--channel',
      'alpha',
      '--json',
    ],
    { deadlineMs: INSTALL_BUDGET_MS }
  );
  expect(install.status, install.stdout + install.stderr).toBe(0);
  const tmt = installed(sandbox);
  for (const args of [
    ['install', 'all', '--json'],
    ['setup', 'claude', '--yes', '--json'],
    ['setup', 'codex', '--yes', '--json'],
  ]) {
    const result = await runCli(tmt, args, { deadlineMs: INSTALL_BUDGET_MS });
    expect(result.status, result.stdout + result.stderr).toBe(0);
  }
  return tmt;
}

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
      expect(existsSync(codexHooks)).toBe(false);
      expect(tree(prefix)).toEqual({});
      expect(recordBytes).toContain('"codex"');
    });
  });

  it('stops a running Office service before removing its files', async () => {
    await withSandbox(async (sandbox) => {
      const tmt = await setUpMachine(sandbox);
      const { prefix } = paths(sandbox);
      const office = (args: string[]) =>
        runCli(tmt, ['office', '--prefix', prefix, ...args, '--json'], {
          deadlineMs: INSTALL_BUDGET_MS,
        });
      const artifact = await createArtifact(sandbox, '0.1.0-alpha.4', new Uint8Array(), 'office');
      try {
        const installedOffice = await office([
          'install',
          '--yes',
          '--archive',
          artifact.archive,
          '--manifest',
          artifact.manifest,
        ]);
        expect(installedOffice.status, installedOffice.stdout + installedOffice.stderr).toBe(0);
        const started = await office(['start']);
        expect(started.status, started.stdout + started.stderr).toBe(0);
        const receipt = path.join(sandbox.globalDir, 'office', 'runtime', 'service-v1.json');
        expect(existsSync(receipt)).toBe(true);

        const result = await runCli(tmt, ['uninstall', '--yes', '--json'], {
          deadlineMs: INSTALL_BUDGET_MS,
        });
        expect(result.status, result.stdout + result.stderr).toBe(0);
        expect(parseWholeStdout(result).officeStopped).toBe(true);
        expect(existsSync(receipt)).toBe(false);
        expect(tree(prefix)).toEqual({});
      } finally {
        // The in-process stop path needs no installed companion.
        await runCli(sandbox, ['office', 'stop', '--json']);
      }
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
