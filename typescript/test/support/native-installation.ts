import path from 'node:path';
import { expect } from 'vitest';
import { parseWholeStdout, runCli, type Sandbox } from './cli-process.js';
import type { ArtifactFixture } from './native-artifact.js';

// Debug payload hashing/decompression and fsync are installation work, not the
// ordinary command-startup budget. Keep a separate finite process deadline.
export const INSTALL_PROCESS_BUDGET_MS = 15_000;

type InstallResult = {
  readonly executable: string;
  readonly version: string;
  readonly changed: boolean;
};

export function installPrefix(sandbox: Sandbox): string {
  return path.join(sandbox.root, 'native install prefix with spaces');
}

export async function install(
  sandbox: Sandbox,
  fixture: ArtifactFixture,
  prefix: string,
  flags: readonly string[] = [],
  tmuxTeamHome?: string
): Promise<InstallResult> {
  const selected =
    tmuxTeamHome === undefined
      ? sandbox
      : {
          ...sandbox,
          cli: {
            executable: '/usr/bin/env',
            args: [`TMUX_TEAM_HOME=${tmuxTeamHome}`, sandbox.cli.executable, ...sandbox.cli.args],
          },
        };
  const result = await runCli(
    selected,
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
      ...flags,
      '--json',
    ],
    { deadlineMs: INSTALL_PROCESS_BUDGET_MS }
  );
  expect(result.status, result.stdout + result.stderr).toBe(0);
  expect(result.stderr).toBe('');
  return parseWholeStdout(result) as unknown as InstallResult;
}

export function currentPointer(prefix: string): string {
  return path.join(prefix, 'lib', 'tmux-team', 'current');
}

export function receiptPath(prefix: string): string {
  return path.join(currentPointer(prefix), 'receipt.json');
}
