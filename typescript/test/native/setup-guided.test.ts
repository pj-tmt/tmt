import { createHash } from 'node:crypto';
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

import { installSetupExtensions } from '../support/native-installation.js';

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
      else snapshot[key] = createHash('sha256').update(readFileSync(entryPath)).digest('hex');
    }
  };
  if (existsSync(root)) visit(root);
  return snapshot;
}

function launcher(sandbox: Sandbox): string {
  const bin = path.join(sandbox.root, 'bin');
  mkdirSync(bin);
  symlinkSync(sandbox.cli.executable, path.join(bin, 'tmt'));
  sandbox.env.PATH = `${bin}${path.delimiter}/usr/bin${path.delimiter}/bin`;
  return bin;
}

/**
 * A machine with a runnable claude and codex (fake executables that record
 * if they ever run), a gemini configuration without its executable, an agy
 * file that is not executable, a user Claude hook, and the stable launcher.
 */
function machine(sandbox: Sandbox): string {
  const bin = launcher(sandbox);
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

// Each scenario may prepare four real archive installations; command deadlines stay unchanged.
describe('guided tmt setup', { timeout: 60_000 }, () => {
  it.each(['present', 'configOnly'] as const)(
    'requires consent for %s Muse skills and preserves its settings',
    async (state) => {
      await withSandbox(async (sandbox) => {
        await installSetupExtensions(sandbox);
        const bin = path.join(sandbox.root, 'muse-bin');
        mkdirSync(bin);
        symlinkSync(sandbox.cli.executable, path.join(bin, 'tmt'));
        sandbox.env.PATH = bin;
        const settings = path.join(sandbox.home, '.config', 'muse', 'settings.json');
        mkdirSync(path.dirname(settings), { recursive: true });
        writeFileSync(settings, '{"userSetting":true}\n');
        if (state === 'present') {
          writeExecutable(
            path.join(bin, 'muse'),
            '#!/bin/sh\nprintf ran > "$HOME/ran-muse"\n',
            0o755
          );
        }

        const before = tree(sandbox.root);
        expectError(await runCli(sandbox, ['setup', '--json']), 'SETUP_CONSENT_REQUIRED');
        expect(tree(sandbox.root)).toEqual(before);

        const applied = await runCli(sandbox, ['setup', '--yes', '--json']);
        expect(applied.status, applied.stdout + applied.stderr).toBe(0);
        const report = parseWholeStdout(applied);
        expect(report.agents).toContainEqual({ name: 'muse', state });
        const plan = report.plan as { kind: string; path: string }[];
        expect(plan).toHaveLength(2);
        expect(plan.every((item) => item.kind === 'skill')).toBe(true);
        for (const name of ['tmt', 'tmt-inbox']) {
          expect(
            lstatSync(path.join(sandbox.home, '.agents', 'skills', name)).isSymbolicLink()
          ).toBe(true);
        }
        expect(readFileSync(settings, 'utf8')).toBe('{"userSetting":true}\n');
        expect(ran(sandbox)).toEqual([]);
        expect(existsSync(sandbox.database)).toBe(false);
      });
    }
  );

  it('sets up every detected agent after one approval and is idempotent', async () => {
    await withSandbox(async (sandbox) => {
      await installSetupExtensions(sandbox);
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
      expect(refused.stdout).toContain('not found: pi · opencode · muse');
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
        { name: 'muse', state: 'absent' },
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
      expect(again.stdout).not.toContain('hint: tmt extension install ops');
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
      await installSetupExtensions(sandbox);
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
      await installSetupExtensions(sandbox);
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
  it('requires consent for missing extensions even when no agent needs setup', async () => {
    await withSandbox(async (sandbox) => {
      launcher(sandbox);
      const before = tree(sandbox.root);
      const refused = await runCli(sandbox, ['setup', '--json']);
      expectError(refused, 'SETUP_CONSENT_REQUIRED');
      expect(tree(sandbox.root)).toEqual(before);
      const plain = await runCli(sandbox, ['setup']);
      expect(plain.status).toBe(1);
      for (const product of ['ops', 'digest', 'remote', 'colab'])
        expect(plain.stdout).toContain(product);
      expect(plain.stdout).toContain('needs remote');
      expect(tree(sandbox.root)).toEqual(before);
    });
  });

  it('keeps healthy binaries and retries only missing bundled skills from the active release', async () => {
    await withSandbox(async (sandbox) => {
      launcher(sandbox);
      await installSetupExtensions(sandbox, true);
      const prefix = path.join(sandbox.home, '.local');
      const before = ['ops', 'digest', 'remote', 'colab'].map((product) => ({
        product,
        receipt: readFileSync(
          path.join(prefix, 'lib', `tmt-${product}`, 'current', 'receipt.json')
        ),
      }));
      const target = path.join(sandbox.home, '.agents', 'skills', 'tmt-ops');
      rmSync(target);
      const result = await runCli(sandbox, ['setup', '--yes', '--json']);
      expect(result.status, result.stdout + result.stderr).toBe(0);
      const report = parseWholeStdout(result);
      expect(report.extensions).toHaveLength(4);
      expect((report.extensions as Array<{ changed: boolean }>).every((row) => !row.changed)).toBe(
        true
      );
      expect(readFileSync(path.join(target, 'SKILL.md'), 'utf8')).toBe('ops fixture guidance\n');
      for (const item of before)
        expect(
          readFileSync(path.join(prefix, 'lib', `tmt-${item.product}`, 'current', 'receipt.json'))
        ).toEqual(item.receipt);
      const settled = tree(sandbox.root);
      const again = await runCli(sandbox, ['setup', '--json']);
      expect(again.status, again.stdout + again.stderr).toBe(0);
      expect(parseWholeStdout(again)).toMatchObject({
        applied: false,
        extensions: expect.arrayContaining([
          expect.objectContaining({ extension: 'ops', status: 'current' }),
        ]),
      });
      expect(tree(sandbox.root)).toEqual(settled);
    });
  });

  it('reports partial failure in one JSON document, skips Colab and preserves completed products', async () => {
    await withSandbox(async (sandbox) => {
      launcher(sandbox);
      await installSetupExtensions(sandbox);
      const prefix = path.join(sandbox.home, '.local');
      const remote = path.join(prefix, 'bin', 'tmt-remote');
      const destination = readlinkSync(remote);
      rmSync(remote);
      writeFileSync(remote, 'foreign executable; never overwrite\n');
      const result = await runCli(sandbox, ['setup', '--yes', '--json']);
      expect(result.status, result.stdout + result.stderr).toBe(1);
      const report = parseWholeStdout(result);
      expect(report.error).toMatchObject({ code: 'SETUP_INCOMPLETE' });
      expect(
        (report.extensions as Array<{ extension: string; status: string }>).map(
          ({ extension, status }) => [extension, status]
        )
      ).toEqual([
        ['ops', 'current'],
        ['digest', 'current'],
        ['remote', 'failed'],
        ['colab', 'skipped'],
      ]);
      expect(readFileSync(remote, 'utf8')).toBe('foreign executable; never overwrite\n');
      rmSync(remote);
      symlinkSync(destination, remote);
      const retry = await runCli(sandbox, ['setup', '--yes', '--json']);
      expect(retry.status, retry.stdout + retry.stderr).toBe(0);
      expect(parseWholeStdout(retry)).toMatchObject({ applied: false });
    });
  });
});
