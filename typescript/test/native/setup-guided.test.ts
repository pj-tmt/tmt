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
import { writeExecutable } from '../support/executable-fixture.mjs';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import {
  expectError,
  parseWholeStdout,
  runCli,
  withSandbox,
  type Sandbox,
} from '../support/cli-process.js';

// Everything lives under the sandbox: a disposable HOME, XDG root and PATH.
const USER_HOOK = '{ "hooks" : [{ "type": "command", "command": "user-command" }] }';
const CLAUDE_ORIGINAL = `{\n "hooks": {"SessionStart": [${USER_HOOK}]}\n}\n`;

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

/**
 * A machine with a runnable claude and codex (fake executables that record
 * if they ever run), a gemini configuration without its executable, an agy
 * file that is not executable, a user Claude hook, and the stable launcher.
 */
function machine(sandbox: Sandbox): string {
  const bin = path.join(sandbox.root, 'bin');
  mkdirSync(bin);
  symlinkSync(sandbox.cli.executable, path.join(bin, 'tmt'));
  for (const agent of ['claude', 'codex']) {
    writeExecutable(path.join(bin, agent), `#!/bin/sh\ntouch "$HOME/ran-${agent}"\n`, 0o755);
  }
  writeExecutable(path.join(bin, 'agy'), '#!/bin/sh\n', 0o644);
  mkdirSync(path.join(sandbox.home, '.gemini'));
  mkdirSync(path.join(sandbox.home, '.claude'));
  writeFileSync(path.join(sandbox.home, '.claude', 'settings.json'), CLAUDE_ORIGINAL);
  sandbox.env.PATH = `${bin}${path.delimiter}/usr/bin${path.delimiter}/bin`;
  return bin;
}

function ran(sandbox: Sandbox): string[] {
  return readdirSync(sandbox.home).filter((name) => name.startsWith('ran-'));
}

describe('guided tmt setup', () => {
  it('sets up every detected agent after one approval and is idempotent', async () => {
    await withSandbox(async (sandbox) => {
      machine(sandbox);
      const before = tree(sandbox.root);
      // No terminal and no --yes: the plan is printed, nothing changes.
      const refused = await runCli(sandbox, ['setup']);
      expect(refused.status).toBe(1);
      expect(refused.stdout).toContain('CHANGES 6');
      expect(refused.stdout).toContain('✓  claude');
      expect(refused.stdout).toContain('!  gemini');
      expect(refused.stdout).toContain('✗  agy');
      expect(refused.stdout).toContain('~/.claude/settings.json');
      expect(refused.stdout).toContain('not found: pi · opencode');
      expect(refused.stderr).toContain('nothing was changed');
      expectError(await runCli(sandbox, ['setup', '--json']), 'SETUP_CONSENT_REQUIRED');
      expect(tree(sandbox.root)).toEqual(before);

      const applied = await runCli(sandbox, ['setup', '--yes', '--json']);
      expect(applied.status, applied.stdout + applied.stderr).toBe(0);
      const report = parseWholeStdout(applied);
      expect(report.applied).toBe(true);
      expect(report.agents).toEqual([
        { name: 'claude', state: 'present' },
        { name: 'codex', state: 'present' },
        { name: 'gemini', state: 'configOnly' },
        { name: 'agy', state: 'broken' },
        { name: 'pi', state: 'absent' },
        { name: 'opencode', state: 'absent' },
      ]);
      const plan = report.plan as { kind: string; driver: string | null; path: string }[];
      expect(plan.filter((item) => item.kind === 'hooks').map((item) => item.driver)).toEqual([
        'claude',
        'codex',
      ]);
      for (const skill of ['.claude/skills/tmt', '.agents/skills/tmt-inbox']) {
        expect(lstatSync(path.join(sandbox.home, skill)).isSymbolicLink()).toBe(true);
      }
      // Present agents get hooks; the user's own hook stays.
      const claude = readFileSync(path.join(sandbox.home, '.claude', 'settings.json'), 'utf8');
      expect(claude).toContain(USER_HOOK);
      expect(claude).toContain('__hook claude');
      expect(readFileSync(path.join(sandbox.home, '.codex', 'hooks.json'), 'utf8')).toContain(
        '__hook codex'
      );
      // A configured agent without an executable gets skills, never hooks;
      // a broken one gets nothing.
      expect(existsSync(path.join(sandbox.home, '.gemini', 'settings.json'))).toBe(false);
      expect(existsSync(path.join(sandbox.home, '.gemini', 'config', 'skills'))).toBe(false);
      expect(ran(sandbox)).toEqual([]);

      const settled = tree(sandbox.root);
      const again = await runCli(sandbox, ['setup']);
      expect(again.status, again.stderr).toBe(0);
      expect(again.stdout).toContain('✓ Everything is set up');
      expect(again.stdout).not.toContain('CHANGES');
      expect(again.stdout).toContain('hint: tmt extension install ops');
      expect(tree(sandbox.root)).toEqual(settled);
      const json = parseWholeStdout(await runCli(sandbox, ['setup', '--json']));
      expect(json).toMatchObject({ applied: false, plan: [] });

      // Current hooks with no record (installed before it existed) are
      // adopted without a prompt: only TMT's own record is written.
      const record = path.join(sandbox.globalDir, 'setup-record.json');
      const recordBytes = readFileSync(record, 'utf8');
      rmSync(record);
      const adopted = await runCli(sandbox, ['setup']);
      expect(adopted.status, adopted.stderr).toBe(0);
      expect(adopted.stdout).toContain('✓ Everything is set up');
      expect(readFileSync(record, 'utf8')).toBe(recordBytes);
      const afterAdoption = tree(sandbox.root);
      expect((await runCli(sandbox, ['setup'])).status).toBe(0);
      expect(tree(sandbox.root)).toEqual(afterAdoption);
      expect(ran(sandbox)).toEqual([]);
    });
  });

  it('replaces planned published names without backups and leaves source destinations untouched', async () => {
    await withSandbox(async (sandbox) => {
      machine(sandbox);
      // A real directory at the published name is replaced.
      const occupied = path.join(sandbox.home, '.claude', 'skills', 'tmt');
      mkdirSync(occupied, { recursive: true });
      writeFileSync(path.join(occupied, 'SKILL.md'), '---\nname: tmt\n---\nmine\n');
      // Codex's skill links into an earlier TMT home's assets.
      const oldSource = path.join(sandbox.root, 'old-home', 'skill-assets', 'abc123', 'tmt');
      mkdirSync(oldSource, { recursive: true });
      writeFileSync(path.join(oldSource, 'SKILL.md'), '---\nname: tmt\n---\nold\n');
      const foreign = path.join(sandbox.home, '.agents', 'skills', 'tmt');
      mkdirSync(path.dirname(foreign), { recursive: true });
      symlinkSync(oldSource, foreign);

      const planned = await runCli(sandbox, ['setup']);
      expect(planned.stdout).toContain('replaces an older TMT skill from');
      expect(planned.stdout).toContain('~/.claude/skills/tmt');

      const applied = await runCli(sandbox, ['setup', '--yes', '--json']);
      expect(applied.status, applied.stdout + applied.stderr).toBe(0);
      expect(applied.stdout + applied.stderr).not.toContain('--force');
      const report = parseWholeStdout(applied);
      expect(report).toMatchObject({ applied: true, kept: [], skipped: [] });
      const plan = report.plan as { kind: string; path: string; replaces?: string }[];
      expect(plan.find((item) => item.path === foreign)?.replaces).toBe(oldSource);
      expect(plan.some((item) => item.path === occupied)).toBe(true);
      // The real directory and foreign link both become current links.
      expect(lstatSync(occupied).isSymbolicLink()).toBe(true);
      expect(readFileSync(path.join(occupied, 'SKILL.md'), 'utf8')).not.toContain(
        '---\nname: tmt\n---\nmine\n'
      );
      expect(
        lstatSync(path.join(sandbox.home, '.claude', 'skills', 'tmt-inbox')).isSymbolicLink()
      ).toBe(true);
      expect(readFileSync(path.join(sandbox.home, '.claude', 'settings.json'), 'utf8')).toContain(
        '__hook claude'
      );
      // The foreign link changes without touching its destination or creating a backup.
      expect(readlinkSync(foreign)).toContain(path.join(sandbox.globalDir, 'skill-assets'));
      expect(existsSync(path.join(sandbox.home, '.agents', '.tmt-skill-backups'))).toBe(false);
      expect(readFileSync(path.join(oldSource, 'SKILL.md'), 'utf8')).toContain('old');

      const again = await runCli(sandbox, ['setup']);
      expect(again.status, again.stderr).toBe(0);
      expect(again.stdout).toContain('✓ Everything is set up');
      expect(ran(sandbox)).toEqual([]);
    });
  });

  it('reports a machine with no agents and changes nothing', async () => {
    await withSandbox(async (sandbox) => {
      const bin = path.join(sandbox.root, 'bin');
      mkdirSync(bin);
      symlinkSync(sandbox.cli.executable, path.join(bin, 'tmt'));
      sandbox.env.PATH = `${bin}${path.delimiter}/usr/bin${path.delimiter}/bin`;
      const before = tree(sandbox.root);
      const result = await runCli(sandbox, ['setup']);
      expect(result.status, result.stderr).toBe(0);
      expect(result.stdout).toContain('AGENTS 0');
      expect(result.stdout).toContain('✓ Everything is set up');
      expect(tree(sandbox.root)).toEqual(before);
    });
  });
});
