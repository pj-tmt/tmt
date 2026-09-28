import {
  chmodSync,
  copyFileSync,
  existsSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  readlinkSync,
  realpathSync,
  symlinkSync,
  unlinkSync,
  writeFileSync,
} from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { calibrateTmuxTripwire } from './tmux-tripwire.js';
import {
  expectError,
  parseWholeStdout,
  runCli,
  type Sandbox,
  withSandbox,
} from '../support/cli-process.js';

type InstallItem = {
  readonly skill: string;
  readonly agent?: string;
  readonly target: string;
  readonly changed: boolean;
  readonly backup?: string;
};

type InstallDocument = {
  readonly installed: InstallItem[];
};

const PROVIDERS = ['claude', 'codex', 'gemini', 'agy', 'pi', 'opencode'] as const;

function canonicalSkill(): Buffer {
  return readFileSync(
    path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../../skills/tmux-team/SKILL.md')
  );
}

function inboxSkill(): Buffer {
  return readFileSync(
    path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../../skills/tmt-inbox/SKILL.md')
  );
}

function officeSkill(): Buffer {
  return readFileSync(
    path.resolve(
      path.dirname(fileURLToPath(import.meta.url)),
      '../../../extensions/tmt-office/skills/tmt-office/SKILL.md'
    )
  );
}

function propCreateSkill(): Buffer {
  return readFileSync(
    path.resolve(
      path.dirname(fileURLToPath(import.meta.url)),
      '../../../extensions/tmt-office/skills/tmt-prop-create/SKILL.md'
    )
  );
}

function avatarCreateSkill(): Buffer {
  return readFileSync(
    path.resolve(
      path.dirname(fileURLToPath(import.meta.url)),
      '../../../extensions/tmt-office/skills/tmt-avatar-create/SKILL.md'
    )
  );
}

function inboxTarget(target: string): string {
  return path.join(path.dirname(target), 'tmt-inbox');
}

function expectedInstalled(target: string, changed: boolean, agent?: string): InstallItem[] {
  return [
    { ...(agent ? { agent } : {}), skill: 'tmux-team', target, changed },
    {
      ...(agent ? { agent } : {}),
      skill: 'tmt-inbox',
      target: inboxTarget(target),
      changed,
    },
  ];
}

function physicalFilePath(filePath: string): string {
  return path.join(realpathSync(path.dirname(filePath)), path.basename(filePath));
}

function targetFor(sandbox: Sandbox, provider: (typeof PROVIDERS)[number]): string {
  switch (provider) {
    case 'claude':
      return path.join(sandbox.home, '.claude', 'skills', 'tmux-team');
    case 'agy':
      return path.join(sandbox.home, '.gemini', 'config', 'skills', 'tmux-team');
    case 'pi':
      return path.join(sandbox.home, '.pi', 'agent', 'skills', 'tmux-team');
    case 'codex':
    case 'gemini':
    case 'opencode':
      return path.join(sandbox.home, '.agents', 'skills', 'tmux-team');
  }
}

function assertSkillLink(target: string, expected: Buffer): string {
  expect(lstatSync(target).isSymbolicLink()).toBe(true);
  const source = realpathSync(target);
  expect(readFileSync(path.join(source, 'SKILL.md'))).toEqual(expected);
  expect(readFileSync(path.join(target, 'SKILL.md'))).toEqual(expected);
  return source;
}

async function isolateExternalCommands(sandbox: Sandbox): Promise<{
  readonly logPath: string;
  readonly baseline: string;
}> {
  const logPath = await calibrateTmuxTripwire(sandbox);
  // The native executable is selected by absolute path. Keep child PATH limited
  // to the task-owned tripwire so provider/Node/Rust discovery cannot leak in.
  sandbox.env.PATH = path.join(sandbox.root, 'task-owned-tripwire');
  return { logPath, baseline: readFileSync(logPath, 'utf8') };
}

function assertNoExternalEffects(sandbox: Sandbox, logPath: string, baseline: string): void {
  expect(readFileSync(logPath, 'utf8')).toBe(baseline);
  expect(existsSync(sandbox.database)).toBe(false);
}

function installDocument(result: Awaited<ReturnType<typeof runCli>>): InstallDocument {
  expect(result.status).toBe(0);
  return parseWholeStdout(result) as unknown as InstallDocument;
}

describe('native installation process contract', () => {
  it('runs a moved native binary from a path with spaces and prints exact canonical guidance', async () => {
    await withSandbox(async (sandbox) => {
      const movedDirectory = path.join(sandbox.root, "native build's files");
      mkdirSync(movedDirectory);
      const movedExecutable = path.join(movedDirectory, 'tmt preview');
      copyFileSync(sandbox.cli.executable, movedExecutable);
      chmodSync(movedExecutable, 0o755);
      const moved = { ...sandbox, cli: { executable: movedExecutable, args: [] } };
      const { logPath, baseline } = await isolateExternalCommands(sandbox);

      const result = await runCli(moved, ['learn', '--skill']);
      expect(result.status).toBe(0);
      expect(result.stderr).toBe('');
      expect(result.stdout).toBe(canonicalSkill().toString('utf8'));
      const officeGuidance = await runCli(moved, ['learn', '--skill', 'tmt-office']);
      expect(officeGuidance.status).toBe(0);
      expect(officeGuidance.stderr).toBe('');
      expect(officeGuidance.stdout).toBe(officeSkill().toString('utf8'));
      const propGuidance = await runCli(moved, ['learn', '--skill', 'tmt-prop-create']);
      expect(propGuidance.status).toBe(0);
      expect(propGuidance.stderr).toBe('');
      expect(propGuidance.stdout).toBe(propCreateSkill().toString('utf8'));
      expect(propGuidance.stdout).not.toContain('extensions/tmt-office/contracts/');
      for (const required of [
        '"formatVersion": 1',
        'tmt office prop validate --file',
        'tmt office prop list --local',
        'tmt office prop install --local',
        'tmt office layout apply --file',
        'tmt office prop remove --local',
        '`.layout`',
      ])
        expect(propGuidance.stdout).toContain(required);
      const avatarGuidance = await runCli(moved, ['learn', '--skill', 'tmt-avatar-create']);
      expect(avatarGuidance.status).toBe(0);
      expect(avatarGuidance.stderr).toBe('');
      expect(avatarGuidance.stdout).toBe(avatarCreateSkill().toString('utf8'));
      expect(avatarGuidance.stdout).not.toContain('extensions/tmt-office/contracts/');
      for (const required of [
        '`formatVersion: 1`',
        '32×48 pixels',
        'tmt office avatar validate --file',
        'tmt office avatar list --local',
        'tmt office avatar install --local',
        'tmt office profile apply --local',
        'tmt office avatar remove --local',
        'maintained `Avatar` composition',
      ])
        expect(avatarGuidance.stdout).toContain(required);
      for (const required of [
        '`tmt-prop-create`',
        '`tmt-avatar-create`',
        'tmt office prop list --local',
        'tmt office prop install --local',
        'tmt office prop remove --local',
        '`.layout`',
        '{"version":1,"map":{...},"objects":[...]}',
        '--legacy-basis',
      ])
        expect(officeGuidance.stdout).toContain(required);
      expect(existsSync(sandbox.localConfig)).toBe(false);
      expect(existsSync(sandbox.globalDir)).toBe(false);

      const customRoot = path.join(sandbox.cwd, 'moved custom skills');
      mkdirSync(customRoot);
      const target = path.join(realpathSync(customRoot), 'tmux-team');
      const installed = installDocument(
        await runCli(moved, ['install', '--dir', 'moved custom skills', '--json'])
      );
      expect(installed).toEqual({ installed: expectedInstalled(target, true) });
      const source = assertSkillLink(target, canonicalSkill());
      assertSkillLink(inboxTarget(target), inboxSkill());
      const assetsRoot = path.join(realpathSync(sandbox.globalDir), 'skill-assets');
      expect(source.startsWith(`${assetsRoot}${path.sep}`)).toBe(true);
      expect(
        JSON.parse(readFileSync(path.join(sandbox.globalDir, 'skill-installations.json'), 'utf8'))
      ).toEqual({
        version: 1,
        targets: [inboxTarget(target), target],
      });
      assertNoExternalEffects(sandbox, logPath, baseline);
    });
  });

  it('initializes the exact local file without storage, tmux, or global configuration', async () => {
    await withSandbox(async (sandbox) => {
      const { logPath, baseline } = await isolateExternalCommands(sandbox);
      const result = await runCli(sandbox, ['init', '--json']);
      expect(result.status).toBe(0);
      expect(parseWholeStdout(result)).toEqual({ created: physicalFilePath(sandbox.localConfig) });
      expect(readFileSync(sandbox.localConfig, 'utf8')).toBe('{}\n');
      expect(existsSync(sandbox.globalDir)).toBe(false);
      assertNoExternalEffects(sandbox, logPath, baseline);
    });
  });

  it('refuses an existing malformed file, an ancestor file, and a broken link', async () => {
    await withSandbox(async (sandbox) => {
      const { logPath, baseline } = await isolateExternalCommands(sandbox);
      const malformed = '{ user-owned malformed settings';
      writeFileSync(sandbox.localConfig, malformed);
      const existing = await runCli(sandbox, ['init', '--json']);
      expect(existing.status).toBe(1);
      expectError(existing, 'ERROR');
      expect(readFileSync(sandbox.localConfig, 'utf8')).toBe(malformed);

      const ancestor = sandbox.localConfig;
      const nested = path.join(sandbox.cwd, 'child', 'nested');
      mkdirSync(nested, { recursive: true });
      writeFileSync(ancestor, '{}\n');
      const nestedSandbox = {
        ...sandbox,
        cwd: nested,
        localConfig: ancestor,
      };
      const ancestorResult = await runCli(nestedSandbox, ['init', '--json']);
      expect(ancestorResult.status).toBe(1);
      expectError(ancestorResult, 'ERROR');
      expect(readFileSync(ancestor, 'utf8')).toBe('{}\n');
      expect(existsSync(path.join(nested, 'tmux-team.json'))).toBe(false);

      // Re-select the ordinary local path and replace it with a dangling link.
      // create_new must reject the link itself, without following or deleting it.
      const missing = path.join(sandbox.root, 'missing-settings.json');
      unlinkSync(sandbox.localConfig);
      symlinkSync(missing, sandbox.localConfig);
      const linkResult = await runCli(sandbox, ['init', '--json']);
      expect(linkResult.status).toBe(1);
      expectError(linkResult, 'ERROR');
      expect(readlinkSync(sandbox.localConfig)).toBe(missing);
      expect(existsSync(missing)).toBe(false);
      assertNoExternalEffects(sandbox, logPath, baseline);
    });
  });

  it('has exactly one successful concurrent init and leaves the winning bytes intact', async () => {
    await withSandbox(async (sandbox) => {
      const { logPath, baseline } = await isolateExternalCommands(sandbox);
      const results = await Promise.all(
        Array.from({ length: 8 }, () => runCli(sandbox, ['init', '--json']))
      );
      expect(results.filter((result) => result.status === 0)).toHaveLength(1);
      expect(results.filter((result) => result.status === 1)).toHaveLength(7);
      for (const result of results) {
        if (result.status === 1) expectError(result, 'ERROR');
        else
          expect(parseWholeStdout(result)).toEqual({
            created: physicalFilePath(sandbox.localConfig),
          });
      }
      expect(readFileSync(sandbox.localConfig, 'utf8')).toBe('{}\n');
      assertNoExternalEffects(sandbox, logPath, baseline);
    });
  });

  it.each(PROVIDERS)(
    'installs explicit %s at its canonical target and repeats as a no-op',
    async (provider) => {
      await withSandbox(async (sandbox) => {
        const expected = canonicalSkill();
        const { logPath, baseline } = await isolateExternalCommands(sandbox);
        mkdirSync(sandbox.globalDir, { recursive: true });
        writeFileSync(sandbox.globalConfig, '{ malformed global config');
        writeFileSync(sandbox.localConfig, '{ malformed local config');
        const target = targetFor(sandbox, provider);

        const first = installDocument(await runCli(sandbox, ['install', provider, '--json']));
        expect(first).toEqual({ installed: expectedInstalled(target, true, provider) });
        const source = assertSkillLink(target, expected);
        const inboxSource = assertSkillLink(inboxTarget(target), inboxSkill());
        const second = installDocument(await runCli(sandbox, ['install', provider, '--json']));
        expect(second).toEqual({ installed: expectedInstalled(target, false, provider) });
        expect(assertSkillLink(target, expected)).toBe(source);
        expect(assertSkillLink(inboxTarget(target), inboxSkill())).toBe(inboxSource);
        assertNoExternalEffects(sandbox, logPath, baseline);
      });
    }
  );

  it('installs all providers in stable order with shared targets and repeat no-op', async () => {
    await withSandbox(async (sandbox) => {
      const expected = canonicalSkill();
      const { logPath, baseline } = await isolateExternalCommands(sandbox);
      mkdirSync(sandbox.globalDir, { recursive: true });
      writeFileSync(sandbox.globalConfig, '{ malformed global config');
      writeFileSync(sandbox.localConfig, '{ malformed local config');
      const targets = Object.fromEntries(
        PROVIDERS.map((provider) => [provider, targetFor(sandbox, provider)])
      );
      const first = installDocument(await runCli(sandbox, ['install', 'all', '--json']));
      expect(first).toEqual({
        installed: PROVIDERS.flatMap((agent, index) =>
          expectedInstalled(targets[agent], [true, true, false, true, true, false][index], agent)
        ),
      });
      for (const target of new Set(Object.values(targets))) {
        assertSkillLink(target, expected);
        assertSkillLink(inboxTarget(target), inboxSkill());
      }
      const second = installDocument(await runCli(sandbox, ['install', 'all', '--json']));
      expect(second.installed.map((item) => item.changed)).toEqual(
        PROVIDERS.flatMap(() => [false, false])
      );
      assertNoExternalEffects(sandbox, logPath, baseline);
    });
  });

  it('uses the neutral target when no provider is detected', async () => {
    await withSandbox(async (sandbox) => {
      const expected = canonicalSkill();
      const { logPath, baseline } = await isolateExternalCommands(sandbox);
      writeFileSync(sandbox.localConfig, '{ malformed local config');
      const target = path.join(sandbox.home, '.agents', 'skills', 'tmux-team');
      const first = installDocument(await runCli(sandbox, ['install', '--json']));
      expect(first).toEqual({ installed: expectedInstalled(target, true) });
      assertSkillLink(target, expected);
      assertSkillLink(inboxTarget(target), inboxSkill());
      const second = installDocument(await runCli(sandbox, ['install', '--json']));
      expect(second).toEqual({ installed: expectedInstalled(target, false) });
      assertNoExternalEffects(sandbox, logPath, baseline);
    });
  });

  it('isolates custom installation roots and preserves unrelated siblings', async () => {
    await withSandbox(async (sandbox) => {
      const expected = canonicalSkill();
      const { logPath, baseline } = await isolateExternalCommands(sandbox);
      const customRoot = path.join(sandbox.cwd, 'custom skills');
      mkdirSync(customRoot);
      writeFileSync(path.join(customRoot, 'unrelated.txt'), 'keep this sibling');
      writeFileSync(sandbox.localConfig, '{ malformed local config');
      const target = path.join(realpathSync(customRoot), 'tmux-team');
      const first = installDocument(
        await runCli(sandbox, ['install', '--dir', 'custom skills', '--json'])
      );
      expect(first).toEqual({ installed: expectedInstalled(target, true) });
      assertSkillLink(target, expected);
      assertSkillLink(inboxTarget(target), inboxSkill());
      const second = installDocument(
        await runCli(sandbox, ['install', '--dir', 'custom skills', '--json'])
      );
      expect(second).toEqual({ installed: expectedInstalled(target, false) });
      expect(readFileSync(path.join(customRoot, 'unrelated.txt'), 'utf8')).toBe(
        'keep this sibling'
      );
      assertNoExternalEffects(sandbox, logPath, baseline);
    });
  });

  it('reports unmanaged conflicts and creates a recoverable JSON backup with force', async () => {
    await withSandbox(async (sandbox) => {
      const { logPath, baseline } = await isolateExternalCommands(sandbox);
      const target = targetFor(sandbox, 'claude');
      mkdirSync(target, { recursive: true });
      writeFileSync(path.join(target, 'user-owned.md'), 'keep this content');
      const refused = await runCli(sandbox, ['install', 'claude', '--json']);
      expect(refused.status).toBe(1);
      expectError(refused, 'ERROR');
      expect(readFileSync(path.join(target, 'user-owned.md'), 'utf8')).toBe('keep this content');

      const forced = installDocument(
        await runCli(sandbox, ['install', 'claude', '--force', '--json'])
      );
      const item = forced.installed[0];
      expect(item).toMatchObject({ agent: 'claude', target, changed: true });
      expect(item.backup).toEqual(expect.any(String));
      const backup = item.backup as string;
      expect(backup).toContain(path.join(sandbox.home, '.claude', '.tmt-skill-backups'));
      expect(readFileSync(path.join(backup, 'user-owned.md'), 'utf8')).toBe('keep this content');
      assertSkillLink(target, canonicalSkill());
      assertNoExternalEffects(sandbox, logPath, baseline);
    });
  });

  it('rejects unknown providers and --dir conflicts before filesystem effects', async () => {
    await withSandbox(async (sandbox) => {
      const { logPath, baseline } = await isolateExternalCommands(sandbox);
      const custom = path.join(sandbox.cwd, 'custom skills');
      for (const args of [
        ['install', 'unknown-provider', '--json'],
        ['install', 'claude', '--dir', 'custom skills', '--json'],
        ['install', 'all', '--dir', 'custom skills', '--json'],
        ['install', '--dir', '', '--json'],
      ]) {
        const result = await runCli(sandbox, args);
        expect(result.status, args.join(' ')).toBe(1);
        expectError(result, 'USAGE_ERROR');
      }
      expect(existsSync(custom)).toBe(false);
      expect(existsSync(sandbox.globalDir)).toBe(false);
      expect(existsSync(sandbox.database)).toBe(false);
      assertNoExternalEffects(sandbox, logPath, baseline);
    });
  });
});
