import {
  existsSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  readlinkSync,
  writeFileSync,
} from 'node:fs';
import path from 'node:path';
import { expect } from 'vite-plus/test';
import { runCli, type Sandbox } from './cli-process.js';
import { createArtifact } from './native-artifact.js';

// Every path here is under the sandbox: a disposable HOME, XDG root and prefix.
export const INSTALL_BUDGET_MS = 30_000;
const USER_HOOK = '{ "hooks" : [{ "type": "command", "command": "user-command" }] }';
export const CLAUDE_ORIGINAL = `{\n "permissions": {"allow": ["Bash(git *)"]},\n "hooks": {"SessionStart": [${USER_HOOK}]}\n}\n`;

/** Files and symlinks (by target) under `root`, without following links. */
export function tree(root: string): Record<string, string> {
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

export function paths(sandbox: Sandbox) {
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

export async function setUpMachine(sandbox: Sandbox): Promise<Sandbox> {
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
