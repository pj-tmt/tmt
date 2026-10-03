import { readFileSync, realpathSync, writeFileSync } from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';
import { runCli, withSandbox, type Sandbox } from '../support/cli-process.js';
import { createArtifact, nativeTarget } from '../support/native-artifact.js';

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const verifier = path.join(repositoryRoot, 'scripts/verify-native-extension-upgrade.mjs');
const installationVerifier = path.join(repositoryRoot, 'scripts/verify-native-installation.mjs');
const PROOF_BUDGET_MS = 60_000;
const recordingDriver = fileURLToPath(
  new URL('../../../rust/target/debug/examples/recording-cli-fixture', import.meta.url)
);

type Artifact = Awaited<ReturnType<typeof createArtifact>>;

// The proof's verifier drives the newest published CLI, so it may only use what that CLI has. The
// driver here is the freshly built CLI behind a native fixture that records its public commands.
// Squad/Remote/Colab: the Office installer also checks that the executable reports the version it is
// installed as, which a built `tmt-office` can do for one version only; the verifier builds the
// same commands for every extension.
describe('extension upgrade proof against the real CLI', () => {
  // The driver is the newest published CLI: today's, carrying its companion, or one released
  // before companions existed (5.0.0-alpha.39), read against its own manifest.
  it.each(
    (['squad', 'remote', 'colab'] as const).flatMap(
      (product) =>
        [
          [product, 'a current CLI', undefined],
          [product, 'a CLI published before companions', null],
        ] as const
    )
  )(
    'upgrades %s through public commands, driven by %s',
    async (extension, _, driverCompanions) => {
      await withSandbox(async (sandbox) => {
        const log = path.join(sandbox.root, 'driver commands.log');
        expect(sandbox.cli.args).toEqual([]);

        const artifact = (
          name: string,
          version: string,
          product: 'cli' | 'squad' | 'remote' | 'colab'
        ) =>
          createArtifact(
            {
              root: path.join(sandbox.root, name),
              // The note travels with the native fixture through archive extraction.
              // The verifier clears its environment and PATH before invoking it.
              installationNote:
                product === 'cli'
                  ? JSON.stringify({ executable: sandbox.cli.executable, log })
                  : undefined,
              cli: {
                executable: recordingDriver,
                companions: driverCompanions === null ? null : path.dirname(sandbox.cli.executable),
              },
            },
            version,
            new Uint8Array(),
            product,
            undefined,
            product === 'squad' ? { 'tmt-squad/SKILL.md': 'lead skill\n' } : {}
          );
        const previous = await artifact('previous', '0.1.0-alpha.1', extension);
        const candidate = await artifact('candidate', '0.1.0-alpha.2', extension);
        const driver = await artifact('driver', '5.0.0-alpha.1', 'cli');

        const result = await runProof(sandbox, { previous, candidate, driver, product: extension });
        expect(result.stderr).toBe('');
        expect(result.status).toBe(0);
        expect(result.stdout).toBe(
          `Extension upgrade verified: ${extension} 0.1.0-alpha.1 -> 0.1.0-alpha.2 (${nativeTarget()})\n`
        );

        const commands = new Set(readFileSync(log, 'utf8').trim().split('\n'));
        expect([...commands].sort()).toEqual(['extension install', 'extension list']);
      });
    },
    PROOF_BUDGET_MS
  );
});

describe('native recording driver', () => {
  it('preserves argv, input, output, cwd, empty PATH and a failing delegate status', async () => {
    await withSandbox(async (sandbox) => {
      const executable = path.join(sandbox.root, 'recording driver with spaces');
      const log = path.join(sandbox.root, 'driver commands.log');
      writeExecutable(executable, readFileSync(recordingDriver), 0o755);
      writeFileSync(
        path.join(sandbox.root, 'NATIVE-INSTALL.md'),
        JSON.stringify({ executable: '/bin/sh', log })
      );
      const script =
        'printf "%s\\n" "$1" "$2" "$PWD" "$PATH"; /bin/cat; printf "delegate stderr\\n" >&2; exit 23';
      const result = await runCli(
        {
          ...sandbox,
          cli: { executable, args: [] },
          env: { ...sandbox.env, PATH: '' },
        },
        ['-c', script, 'fixture argv zero', 'argument with spaces', 'quote\'and"double'],
        { stdin: 'input\0bytes\n' }
      );
      expect(result.status).toBe(23);
      expect(result.signal).toBeNull();
      expect(result.stdout).toBe(
        `argument with spaces\nquote'and"double\n${realpathSync(sandbox.cwd)}\n\ninput\0bytes\n`
      );
      expect(result.stderr).toBe('delegate stderr\n');
      expect(readFileSync(log, 'utf8')).toBe(`-c ${script}\n`);
    });
  });
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
  {
    previous,
    candidate,
    driver,
    product = 'squad',
  }: Record<'previous' | 'candidate' | 'driver', Artifact> & { product?: string }
) {
  return runCli(
    { ...sandbox, cli: { executable: process.execPath, args: [] } },
    [
      verifier,
      '--product',
      product,
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
