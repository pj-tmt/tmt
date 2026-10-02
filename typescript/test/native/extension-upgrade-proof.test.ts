import { readFileSync } from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { runCli, withSandbox, type Sandbox } from '../support/cli-process.js';
import { createArtifact, nativeTarget } from '../support/native-artifact.js';

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const verifier = path.join(repositoryRoot, 'scripts/verify-native-extension-upgrade.mjs');
const installationVerifier = path.join(repositoryRoot, 'scripts/verify-native-installation.mjs');
const PROOF_BUDGET_MS = 60_000;

type Artifact = Awaited<ReturnType<typeof createArtifact>>;

// The proof's verifier drives the newest published CLI, so it may only use what that CLI has. The
// driver here is the freshly built CLI behind a script that records the command it was given.
// Squad only: the Office installer also checks that the executable reports the version it is
// installed as, which a built `tmt-office` can do for one version only; the verifier builds the
// same commands for both extensions.
describe('extension upgrade proof against the real CLI', () => {
  // The driver is the newest published CLI: today's, carrying its companion, or one released
  // before companions existed (5.0.0-alpha.39), read against its own manifest.
  it.each([
    ['a current CLI', undefined],
    ['a CLI published before companions', null],
  ] as const)(
    'upgrades Squad through `tmt extension install` and `tmt extension ls` only, driven by %s',
    async (_, driverCompanions) => {
      await withSandbox(async (sandbox) => {
        const log = path.join(sandbox.root, 'driver commands.log');
        const wrapper = path.join(sandbox.root, 'recording tmt');
        // The proof runs it with an empty PATH: only shell builtins and an absolute path are used.
        writeExecutable(
          wrapper,
          `#!/bin/sh\nprintf '%s %s\\n' "$1" "$2" >> "${log}"\nexec "${sandbox.cli.executable}" "$@"\n`,
          0o755
        );
        expect(sandbox.cli.args).toEqual([]);

        const artifact = (name: string, version: string, product: 'cli' | 'squad') =>
          createArtifact(
            {
              root: path.join(sandbox.root, name),
              // The wrapper stands in for tmt; its companions are the build's.
              cli: {
                executable: wrapper,
                companions: driverCompanions === null ? null : path.dirname(sandbox.cli.executable),
              },
            },
            version,
            new Uint8Array(),
            product,
            undefined,
            product === 'squad' ? { 'tmt-squad/SKILL.md': 'lead skill\n' } : {}
          );
        const previous = await artifact('previous', '0.1.0-alpha.1', 'squad');
        const candidate = await artifact('candidate', '0.1.0-alpha.2', 'squad');
        const driver = await artifact('driver', '5.0.0-alpha.1', 'cli');

        const result = await runProof(sandbox, { previous, candidate, driver });
        expect(result.stderr).toBe('');
        expect(result.status).toBe(0);
        expect(result.stdout).toBe(
          `Extension upgrade verified: squad 0.1.0-alpha.1 -> 0.1.0-alpha.2 (${nativeTarget()})\n`
        );

        const commands = new Set(readFileSync(log, 'utf8').trim().split('\n'));
        expect([...commands].sort()).toEqual(['extension install', 'extension list']);
      });
    },
    PROOF_BUDGET_MS
  );
});

describe('CLI installation proof', () => {
  it('refuses a CLI archive under release that lacks a companion', async () => {
    await withSandbox(async (sandbox) => {
      const cli = (name: string, version: string, companions: string | null) =>
        createArtifact(
          { root: path.join(sandbox.root, name), cli: { ...sandbox.cli, companions } },
          version
        );
      const previous = await cli('previous', '5.0.0-alpha.39', null);
      const current = await cli('current', '5.0.0-alpha.40', null);
      const result = await runCli({ ...sandbox, cli: { executable: process.execPath, args: [] } }, [
        installationVerifier,
        '--archive',
        current.archive,
        '--manifest',
        current.manifest,
        '--previous-archive',
        previous.archive,
        '--previous-manifest',
        previous.manifest,
        '--target',
        nativeTarget(),
        '--skill',
        path.join(repositoryRoot, '..', 'skills', 'tmux-team', 'SKILL.md'),
      ]);
      expect(result.status).not.toBe(0);
      expect(result.stderr).toContain('A release archive must carry every companion executable');
    });
  });
});

function runProof(
  sandbox: Sandbox,
  { previous, candidate, driver }: Record<'previous' | 'candidate' | 'driver', Artifact>
) {
  return runCli(
    { ...sandbox, cli: { executable: process.execPath, args: [] } },
    [
      verifier,
      '--product',
      'squad',
      '--archive',
      candidate.archive,
      '--manifest',
      candidate.manifest,
      '--previous-archive',
      previous.archive,
      '--previous-manifest',
      previous.manifest,
      '--driver-archive',
      driver.archive,
      '--driver-manifest',
      driver.manifest,
      '--target',
      nativeTarget(),
    ],
    { deadlineMs: PROOF_BUDGET_MS - 10_000 }
  );
}
