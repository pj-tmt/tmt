import path from 'node:path';
import {
  readFileSync,
  realpathSync,
  renameSync,
  symlinkSync,
  unlinkSync,
  writeFileSync,
} from 'node:fs';
import { fileURLToPath } from 'node:url';
import { expect } from 'vite-plus/test';
import { parseWholeStdout, runCli, withSandbox, type Sandbox } from './cli-process.js';
import { syntheticAlphaVersion } from '../../scripts/release-versions.mjs';
import { workspaceVersion } from './workspace-version.js';
import { createArtifact, type ArtifactFixture } from './native-artifact.js';

/** Installation and upgrade tests use a real, mechanically injected synthetic alpha CLI. */
export function withReleaseSandbox<T>(callback: (sandbox: Sandbox) => T | Promise<T>): Promise<T> {
  const target =
    process.env.CARGO_TARGET_DIR ?? fileURLToPath(new URL('../../../rust/target', import.meta.url));
  return withSandbox(
    async (sandbox) => {
      const version = await runCli(sandbox, ['--version']);
      expect(version.status, version.stderr).toBe(0);
      expect(version.stdout.trim()).toBe(syntheticAlphaVersion(workspaceVersion('tmt-cli')));
      return callback(sandbox);
    },
    {
      TMT_TEST_CLI: JSON.stringify({
        executable: path.join(target, 'debug/native-release-fixture/tmt'),
        args: [],
      }),
    }
  );
}

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
  tmtHome?: string
): Promise<InstallResult> {
  const selected =
    tmtHome === undefined
      ? sandbox
      : {
          ...sandbox,
          cli: {
            executable: '/usr/bin/env',
            args: [`TMT_HOME=${tmtHome}`, sandbox.cli.executable, ...sandbox.cli.args],
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

/** Project a verified synthetic receipt into the shipped former Squad layout.
 * This matches the adapter's former-product fixture; no historical binary or
 * user installation is selected, and the Core CLI remains the command runner.
 */
export async function installFormerSquad(sandbox: Sandbox, prefix: string): Promise<void> {
  const artifact = await createArtifact(sandbox, '0.1.0-alpha.1', new Uint8Array(), 'ops');
  await install(sandbox, artifact, prefix, ['--product', 'ops']);
  const release = realpathSync(path.join(prefix, 'lib/tmt-ops/current'));
  const receiptFile = path.join(release, 'receipt.json');
  const receipt = JSON.parse(readFileSync(receiptFile, 'utf8'));
  receipt.file_sha256['tmt-squad'] = receipt.file_sha256['tmt-ops'];
  delete receipt.file_sha256['tmt-ops'];
  receipt.archive = receipt.archive.replace('ops-', 'squad-');
  renameSync(path.join(release, 'tmt-ops'), path.join(release, 'tmt-squad'));
  writeFileSync(receiptFile, JSON.stringify(receipt));
  unlinkSync(path.join(prefix, 'bin/tmt-ops'));
  renameSync(path.join(prefix, 'lib/tmt-ops'), path.join(prefix, 'lib/tmt-squad'));
  for (const name of ['tmt-squad', 'tmt-sq']) {
    symlinkSync('../lib/tmt-squad/current/tmt-squad', path.join(prefix, 'bin', name));
  }
}
