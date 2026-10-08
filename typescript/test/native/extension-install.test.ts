import {
  chmodSync,
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
import { writeExecutable } from '../support/executable-fixture.mjs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { describe, expect, it } from 'vite-plus/test';
import {
  expectError,
  parseWholeStdout,
  runCli,
  withSandbox,
  type Sandbox,
} from '../support/cli-process.js';
import { install, installFormerSquad, withReleaseSandbox } from '../support/native-installation.js';
import { createArtifact, type ArtifactFixture } from '../support/native-artifact.js';

const INSTALL_PROCESS_BUDGET_MS = 15_000;

function replacementArgs(artifact: ArtifactFixture, prefix: string): string[] {
  return [
    'extension',
    'install',
    'ops',
    '--yes',
    '--archive',
    artifact.archive,
    '--manifest',
    artifact.manifest,
    '--prefix',
    prefix,
    '--channel',
    'alpha',
  ];
}

async function enableHooks(sandbox: Sandbox, name: string): Promise<unknown> {
  const result = await runCli(sandbox, ['extension', 'hooks', 'enable', name, '--json']);
  expect(result.status, result.stdout + result.stderr).toBe(0);
  return parseWholeStdout(result).enabled;
}

// Replacement now delegates migration before removing the former install.
// This test-owned server has no boards and every command names its socket.
async function withReplacementTmux<T>(sandbox: Sandbox, body: () => Promise<T>): Promise<T> {
  const found = spawnSync('/usr/bin/which', ['tmux'], { encoding: 'utf8' });
  if (found.status !== 0) throw new Error('tmux is required for native retirement checks');
  const executable = found.stdout.trim();
  const socket = path.join(sandbox.root, 'replacement.sock');
  const invoke = (args: string[]) => {
    const result = spawnSync(executable, ['-S', socket, ...args], {
      env: sandbox.env,
      encoding: 'utf8',
      timeout: 5000,
    });
    if (result.status !== 0) throw new Error(result.stderr);
    return result.stdout.trim();
  };
  sandbox.env.PATH = [path.dirname(executable), sandbox.env.PATH].join(path.delimiter);
  invoke(['-f', '/dev/null', 'new-session', '-d', '-s', 'replacement', '/bin/sh']);
  sandbox.env.TMUX = `${socket},${invoke(['display-message', '-p', '#{pid}'])},0`;
  try {
    return await body();
  } finally {
    invoke(['kill-server']);
  }
}

describe('former Squad hook consent during Ops replacement', () => {
  const hint =
    'Removed former squad hook consent; Ops hooks require separate consent. Enable with: tmt extension hooks enable ops';

  it.each([
    { consent: true, json: true, independentOps: false },
    { consent: false, json: true, independentOps: false },
    { consent: true, json: false, independentOps: false },
    { consent: false, json: false, independentOps: false },
    { consent: true, json: true, independentOps: true },
  ])(
    'replaces verified Squad with $consent consent, JSON=$json, independent Ops=$independentOps',
    async ({ consent, json, independentOps }) => {
      await withSandbox(async (sandbox) => {
        const prefix = path.join(sandbox.root, 'replacement prefix');
        await installFormerSquad(sandbox, prefix);
        sandbox.env.PATH = [path.join(prefix, 'bin'), '/usr/bin', '/bin'].join(':');
        return withReplacementTmux(sandbox, async () => {
          // A separately consented observer proves removal is scoped to the former name.
          symlinkSync(path.join(prefix, 'bin/tmt-squad'), path.join(prefix, 'bin/tmt-observer'));
          const observer = consent ? await enableHooks(sandbox, 'observer') : undefined;
          if (consent) await enableHooks(sandbox, 'squad');
          const settings = path.join(sandbox.globalDir, 'extension-hooks.json');
          const sentinel = path.join(sandbox.globalDir, 'ops.toml');
          mkdirSync(sandbox.globalDir, { recursive: true });
          writeFileSync(sentinel, 'retained user configuration\n');
          const candidate = await createArtifact(sandbox, '0.1.0-alpha.2', new Uint8Array(), 'ops');
          let ops: unknown;
          if (independentOps) {
            await install(sandbox, candidate, prefix, ['--product', 'ops']);
            ops = await enableHooks(sandbox, 'ops');
          }
          const result = await runCli(
            sandbox,
            [...replacementArgs(candidate, prefix), ...(json ? ['--json'] : [])],
            { deadlineMs: INSTALL_PROCESS_BUDGET_MS }
          );
          expect(result.status, result.stdout + result.stderr).toBe(0);
          expect(result.stderr).toBe('');
          if (json) {
            const report = parseWholeStdout(result);
            expect(report.replaced).toBe('squad');
            expect(report.removed).toEqual([
              path.join(realpathSync(prefix), 'bin/tmt-squad'),
              path.join(realpathSync(prefix), 'bin/tmt-sq'),
              path.join(realpathSync(prefix), 'lib/tmt-squad'),
            ]);
            if (consent)
              expect(report.hooks).toEqual({
                disabled: ['squad'],
                enableCommand: 'tmt extension hooks enable ops',
              });
            else expect(report).not.toHaveProperty('hooks');
          } else {
            expect(result.stdout.includes(hint)).toBe(consent);
          }
          expect(existsSync(path.join(prefix, 'lib/tmt-squad'))).toBe(false);
          expect(existsSync(path.join(prefix, 'bin/tmt-squad'))).toBe(false);
          expect(readlinkSync(path.join(prefix, 'bin/tmt-ops'))).toBe(
            '../lib/tmt-ops/current/tmt-ops'
          );
          expect(
            JSON.parse(readFileSync(path.join(prefix, 'lib/tmt-ops/current/receipt.json'), 'utf8'))
              .version
          ).toBe(candidate.version);
          const listed = await runCli(sandbox, ['extension', 'hooks', 'ls', '--json']);
          expect(parseWholeStdout(listed).extensions).toEqual(
            independentOps ? [observer, ops] : consent ? [observer] : []
          );
          const stored = existsSync(settings)
            ? JSON.parse(readFileSync(settings, 'utf8')).extensions
            : [];
          expect(stored.map((item: { name: string }) => item.name)).toEqual(
            independentOps ? ['observer', 'ops'] : consent ? ['observer'] : []
          );
          expect(readFileSync(sentinel, 'utf8')).toBe('retained user configuration\n');
          const before = existsSync(settings) ? readFileSync(settings) : undefined;
          const repeated = await runCli(
            sandbox,
            [...replacementArgs(candidate, prefix), '--json'],
            {
              deadlineMs: INSTALL_PROCESS_BUDGET_MS,
            }
          );
          expect(repeated.status, repeated.stdout + repeated.stderr).toBe(0);
          expect(parseWholeStdout(repeated)).not.toHaveProperty('hooks');
          if (before) expect(readFileSync(settings).equals(before)).toBe(true);
          else expect(existsSync(settings)).toBe(false);
          expect(existsSync(sandbox.database)).toBe(false);
        });
      });
    },
    60_000
  );

  it.each(['former', 'successor'] as const)(
    'retains former hook consent on a pinned %s upgrade no-op',
    async (selected) => {
      await withSandbox(async (sandbox) => {
        const prefix = path.join(sandbox.root, 'replacement prefix');
        await installFormerSquad(sandbox, prefix);
        sandbox.env.PATH = [path.join(prefix, 'bin'), '/usr/bin', '/bin'].join(':');
        await enableHooks(sandbox, 'squad');
        if (selected === 'successor') {
          const candidate = await createArtifact(sandbox, '0.1.0-alpha.2', new Uint8Array(), 'ops');
          await install(sandbox, candidate, prefix, ['--product', 'ops']);
        }
        const receiptFile = path.join(
          prefix,
          `lib/tmt-${selected === 'former' ? 'squad' : 'ops'}/current/receipt.json`
        );
        const receipt = JSON.parse(readFileSync(receiptFile, 'utf8'));
        receipt.pinned_version = receipt.version;
        writeFileSync(receiptFile, JSON.stringify(receipt));
        const settings = path.join(sandbox.globalDir, 'extension-hooks.json');
        const before = readFileSync(settings);
        const result = await runCli(
          sandbox,
          ['extension', 'upgrade', 'ops', '--yes', '--prefix', prefix, '--json'],
          { deadlineMs: INSTALL_PROCESS_BUDGET_MS }
        );
        expect(result.status, result.stdout + result.stderr).toBe(0);
        expect(parseWholeStdout(result)).toMatchObject({ changed: false, skippedPinned: true });
        expect(parseWholeStdout(result)).not.toHaveProperty('hooks');
        expect(readFileSync(settings).equals(before)).toBe(true);
        expect(existsSync(path.join(prefix, 'lib/tmt-squad'))).toBe(true);
        expect(existsSync(path.join(prefix, 'bin/tmt-squad'))).toBe(true);
      });
    },
    60_000
  );

  it.each(['invalid', 'unwritable'] as const)(
    'retains former installation on %s consent failure and retries at the active Ops version',
    async (kind) => {
      await withSandbox(async (sandbox) => {
        const prefix = path.join(sandbox.root, 'replacement prefix');
        await installFormerSquad(sandbox, prefix);
        sandbox.env.PATH = [path.join(prefix, 'bin'), '/usr/bin', '/bin'].join(':');
        return withReplacementTmux(sandbox, async () => {
          await enableHooks(sandbox, 'squad');
          const settings = path.join(sandbox.globalDir, 'extension-hooks.json');
          const original = readFileSync(settings);
          if (kind === 'invalid') writeFileSync(settings, '{invalid consent');
          else chmodSync(sandbox.globalDir, 0o500);
          const before = readFileSync(settings);
          const candidate = await createArtifact(sandbox, '0.1.0-alpha.2', new Uint8Array(), 'ops');
          try {
            const result = await runCli(
              sandbox,
              [...replacementArgs(candidate, prefix), '--json'],
              {
                deadlineMs: INSTALL_PROCESS_BUDGET_MS,
              }
            );
            const error = expectError(
              result,
              kind === 'invalid' ? 'EXTENSION_HOOKS_INVALID' : 'EXTENSION_HOOKS_UNAVAILABLE'
            ).error as { message: string };
            expect(error.message).toContain(
              'ops is installed, but former squad hook consent cleanup failed'
            );
            expect(error.message).toContain(settings);
            expect(error.message).toContain('former installation was retained');
            expect(readFileSync(settings).equals(before)).toBe(true);
            expect(existsSync(path.join(prefix, 'lib/tmt-squad'))).toBe(true);
            expect(existsSync(path.join(prefix, 'bin/tmt-squad'))).toBe(true);
            expect(readlinkSync(path.join(prefix, 'bin/tmt-ops'))).toBe(
              '../lib/tmt-ops/current/tmt-ops'
            );
          } finally {
            chmodSync(sandbox.globalDir, 0o700);
          }
          writeFileSync(settings, original);
          const recovered = await runCli(
            sandbox,
            [...replacementArgs(candidate, prefix), '--json'],
            {
              deadlineMs: INSTALL_PROCESS_BUDGET_MS,
            }
          );
          expect(recovered.status, recovered.stdout + recovered.stderr).toBe(0);
          expect(parseWholeStdout(recovered)).toMatchObject({
            changed: false,
            replaced: 'squad',
            hooks: { disabled: ['squad'], enableCommand: 'tmt extension hooks enable ops' },
          });
          expect(existsSync(path.join(prefix, 'lib/tmt-squad'))).toBe(false);
          expect(JSON.parse(readFileSync(settings, 'utf8')).extensions).toEqual([]);
        });
      });
    },
    60_000
  );

  it('keeps withdrawn consent on partial former removal and completes a retry without enabling Ops', async () => {
    await withSandbox(async (sandbox) => {
      const prefix = path.join(sandbox.root, 'replacement prefix');
      await installFormerSquad(sandbox, prefix);
      sandbox.env.PATH = [path.join(prefix, 'bin'), '/usr/bin', '/bin'].join(':');
      return withReplacementTmux(sandbox, async () => {
        await enableHooks(sandbox, 'squad');
        const candidate = await createArtifact(sandbox, '0.1.0-alpha.2', new Uint8Array(), 'ops');
        await install(sandbox, candidate, prefix, ['--product', 'ops']);
        const formerRoot = path.join(prefix, 'lib/tmt-squad');
        chmodSync(formerRoot, 0o500);
        try {
          const failed = await runCli(sandbox, [...replacementArgs(candidate, prefix), '--json'], {
            deadlineMs: INSTALL_PROCESS_BUDGET_MS,
          });
          const error = expectError(failed, 'EXTENSION_INSTALL_FAILED').error as {
            message: string;
          };
          expect(error.message).toContain(
            'ops is installed, but former squad replacement cleanup failed'
          );
          expect(error.message).toContain(hint);
          expect(
            JSON.parse(readFileSync(path.join(sandbox.globalDir, 'extension-hooks.json'), 'utf8'))
              .extensions
          ).toEqual([]);
          expect(existsSync(path.join(prefix, 'lib/tmt-squad'))).toBe(true);
          expect(existsSync(path.join(prefix, 'bin/tmt-squad'))).toBe(false);
          expect(existsSync(path.join(prefix, 'bin/tmt-sq'))).toBe(false);
          expect(existsSync(path.join(prefix, 'bin/tmt-ops'))).toBe(true);
        } finally {
          chmodSync(formerRoot, 0o700);
        }
        const recovered = await runCli(sandbox, [...replacementArgs(candidate, prefix), '--json'], {
          deadlineMs: INSTALL_PROCESS_BUDGET_MS,
        });
        expect(recovered.status, recovered.stdout + recovered.stderr).toBe(0);
        expect(parseWholeStdout(recovered)).toMatchObject({ changed: false, replaced: 'squad' });
        expect(parseWholeStdout(recovered)).not.toHaveProperty('hooks');
        expect(existsSync(path.join(prefix, 'lib/tmt-squad'))).toBe(false);
        expect(
          parseWholeStdout(await runCli(sandbox, ['extension', 'hooks', 'ls', '--json'])).extensions
        ).toEqual([]);
      });
    });
  }, 60_000);
});

describe('tmt extension install surface', () => {
  it('offers every registered official extension independently of release activation', async () => {
    await withSandbox(async (sandbox) => {
      const result = await runCli(sandbox, [
        'extension',
        'ls',
        '--prefix',
        path.join(sandbox.root, 'empty prefix'),
        '--json',
      ]);
      expect(result.status).toBe(0);
      expect(result.stderr).toBe('');
      const listed = parseWholeStdout(result) as {
        extensions: { name: string; installed: boolean }[];
      };
      expect(listed.extensions.every((extension) => !extension.installed)).toBe(true);
      expect(listed.extensions.map((extension) => extension.name).sort()).toEqual([
        'colab',
        'ops',
        'remote',
      ]);
      expectError(
        await runCli(sandbox, ['extension', 'install', 'driver-herdr', '--yes', '--json']),
        'EXTENSION_UNKNOWN'
      );
    });
  });

  it.each(['remote', 'colab'] as const)(
    'installs and upgrades %s without starting it or modifying its private state',
    async (name) => {
      await withSandbox(async (sandbox) => {
        const prefix = path.join(sandbox.root, `${name} prefix`);
        const extensionState = path.join(sandbox.globalDir, name);
        mkdirSync(extensionState, { recursive: true });
        writeFileSync(path.join(extensionState, 'machine.key'), 'retained key');
        writeFileSync(path.join(extensionState, `${name}.db`), 'retained grants');
        const first = await createArtifact(sandbox, '0.1.0-alpha.1', new Uint8Array(), name);
        const cli = (args: string[]) =>
          runCli(sandbox, [...args, '--json'], { deadlineMs: INSTALL_PROCESS_BUDGET_MS });
        const install = (artifact: ArtifactFixture, yes: boolean) =>
          cli([
            'extension',
            'install',
            name,
            '--archive',
            artifact.archive,
            '--manifest',
            artifact.manifest,
            '--prefix',
            prefix,
            ...(yes ? ['--yes'] : []),
          ]);
        expectError(await install(first, false), 'EXTENSION_CONSENT_REQUIRED');
        expect(existsSync(prefix)).toBe(false);
        const installed = await install(first, true);
        expect(installed.status, installed.stdout + installed.stderr).toBe(0);
        expect(parseWholeStdout(installed)).toEqual({
          extension: name,
          installed: true,
          changed: true,
          version: '0.1.0-alpha.1',
          executable: path.join(realpathSync(prefix), `bin/tmt-${name}`),
        });
        expect(readlinkSync(path.join(prefix, `bin/tmt-${name}`))).toBe(
          `../lib/tmt-${name}/current/tmt-${name}`
        );
        expect(parseWholeStdout(await install(first, true))).toMatchObject({ changed: false });
        const oldRelease = realpathSync(path.join(prefix, `lib/tmt-${name}/current`));
        const candidate = await createArtifact(sandbox, '0.1.0-alpha.2', new Uint8Array([1]), name);
        const upgraded = await install(candidate, true);
        expect(upgraded.status, upgraded.stdout + upgraded.stderr).toBe(0);
        expect(parseWholeStdout(upgraded)).toMatchObject({
          changed: true,
          version: '0.1.0-alpha.2',
        });
        expect(existsSync(oldRelease)).toBe(true);
        const listed = await cli(['extension', 'ls', '--prefix', prefix]);
        expect(parseWholeStdout(listed).extensions).toContainEqual({
          name,
          installed: true,
          version: '0.1.0-alpha.2',
          channel: 'alpha',
          pinned: null,
          commands: [`tmt-${name}`],
          shadowedBy: [],
        });
        expectError(await install(first, true), 'EXTENSION_INSTALL_FAILED');
        const removed = await cli(['extension', 'rm', name, '--yes', '--prefix', prefix]);
        expect(removed.status, removed.stdout + removed.stderr).toBe(0);
        expect(parseWholeStdout(removed)).toMatchObject({
          extension: name,
          installed: false,
          changed: true,
        });
        expect(existsSync(path.join(prefix, `bin/tmt-${name}`))).toBe(false);
        expect(existsSync(path.join(prefix, `lib/tmt-${name}/releases`))).toBe(true);
        expect(readFileSync(path.join(extensionState, 'machine.key'), 'utf8')).toBe('retained key');
        expect(readFileSync(path.join(extensionState, `${name}.db`), 'utf8')).toBe(
          'retained grants'
        );
        expect(existsSync(path.join(extensionState, 'control.sock'))).toBe(false);
        expect(existsSync(sandbox.database)).toBe(false);
      });
    },
    60_000
  );

  it('installs, lists offline, refuses without consent, and uninstalls squad keeping its releases', async () => {
    await withSandbox(async (sandbox) => {
      const prefix = path.join(sandbox.root, 'extension prefix');
      const squad = await createArtifact(sandbox, '0.1.0-alpha.1', new Uint8Array(), 'ops');
      const cli = (args: string[]) =>
        runCli(sandbox, [...args, '--json'], { deadlineMs: INSTALL_PROCESS_BUDGET_MS });

      // Consent is never assumed in a non-interactive run.
      const refused = await cli([
        'extension',
        'install',
        'ops',
        '--archive',
        squad.archive,
        '--manifest',
        squad.manifest,
        '--prefix',
        prefix,
      ]);
      expectError(refused, 'EXTENSION_CONSENT_REQUIRED');
      expect(existsSync(prefix)).toBe(false);
      expectError(await cli(['extension', 'install', 'cli', '--yes']), 'EXTENSION_UNKNOWN');

      const install = [
        'extension',
        'install',
        'ops',
        '--yes',
        '--archive',
        squad.archive,
        '--manifest',
        squad.manifest,
        '--prefix',
        prefix,
        '--channel',
        'alpha',
      ];
      const installed = await cli(install);
      expect(installed.stderr).toBe('');
      expect(parseWholeStdout(installed)).toEqual({
        extension: 'ops',
        installed: true,
        changed: true,
        version: '0.1.0-alpha.1',
        executable: path.join(realpathSync(prefix), 'bin/tmt-ops'),
      });
      for (const link of ['tmt-ops'])
        expect(readlinkSync(path.join(prefix, 'bin', link))).toBe('../lib/tmt-ops/current/tmt-ops');
      expect(parseWholeStdout(await cli(install))).toMatchObject({ changed: false });

      // A foreign same-named command elsewhere on PATH is reported, never run.
      const foreign = path.join(sandbox.root, 'foreign bin');
      mkdirSync(foreign);
      writeExecutable(path.join(foreign, 'tmt-ops'), '#!/bin/sh\nexit 7\n', 0o755);
      // A controlled PATH: never the user's own installed commands.
      sandbox.env.PATH = [path.join(prefix, 'bin'), foreign, '/usr/bin', '/bin'].join(':');
      const listed = parseWholeStdout(await cli(['extension', 'list', '--prefix', prefix]));
      expect(listed).toEqual({
        extensions: [
          {
            name: 'ops',
            installed: true,
            version: '0.1.0-alpha.1',
            channel: 'alpha',
            pinned: null,
            commands: ['tmt-ops'],
            shadowedBy: [path.join(foreign, 'tmt-ops')],
          },
          {
            name: 'remote',
            installed: false,
            version: null,
            channel: null,
            pinned: null,
            commands: ['tmt-remote'],
            shadowedBy: [],
          },
          {
            name: 'colab',
            installed: false,
            version: null,
            channel: null,
            pinned: null,
            commands: ['tmt-colab'],
            shadowedBy: [],
          },
        ],
      });

      // Root help discovers the sole Ops command.
      const help = await runCli(sandbox, ['--help']);
      expect(help.stdout).toMatch(/^ {2}ops +\S/m);

      expectError(
        await cli(['extension', 'uninstall', 'ops', '--prefix', prefix]),
        'EXTENSION_CONSENT_REQUIRED'
      );
      const removed = await cli(['extension', 'uninstall', 'ops', '--yes', '--prefix', prefix]);
      expect(parseWholeStdout(removed)).toEqual({
        extension: 'ops',
        installed: false,
        changed: true,
        skillsRemoved: [],
        skillsKept: [],
        kept: ['releases', 'hookConsent'],
      });
      for (const link of ['tmt-ops'])
        expect(existsSync(path.join(prefix, 'bin', link))).toBe(false);
      expect(existsSync(path.join(prefix, 'lib/tmt-ops/releases'))).toBe(true);
      expectError(
        await cli(['extension', 'upgrade', 'ops', '--yes', '--prefix', prefix]),
        'EXTENSION_NOT_INSTALLED'
      );
    });
  }, 60_000);

  it('names one exact repair command, refuses unsafe damage, and treats healthy repair as a no-op', async () => {
    await withSandbox(async (sandbox) => {
      const prefix = path.join(sandbox.root, "repair prefix with ' quote");
      const squad = await createArtifact(sandbox, '0.1.0-alpha.1', new Uint8Array(), 'ops');
      const cli = (args: string[]) =>
        runCli(sandbox, [...args, '--json'], { deadlineMs: INSTALL_PROCESS_BUDGET_MS });
      expectError(
        await cli(['extension', 'install', 'ops', '--repair', '--prefix', prefix]),
        'EXTENSION_CONSENT_REQUIRED'
      );
      expect(existsSync(prefix)).toBe(false);
      const installed = await cli([
        'extension',
        'install',
        'ops',
        '--yes',
        '--archive',
        squad.archive,
        '--manifest',
        squad.manifest,
        '--prefix',
        prefix,
        '--channel',
        'alpha',
      ]);
      expect(installed.status).toBe(0);
      const release = realpathSync(path.join(prefix, 'lib/tmt-ops/current'));
      const receiptPath = path.join(release, 'receipt.json');
      const receipt = JSON.parse(readFileSync(receiptPath, 'utf8'));
      // Test-only provenance lets read-only/healthy cases run without a live release or endpoint bypass.
      receipt.source = {
        kind: 'github-release',
        repository: 'wkh237/tmt',
        release_id: 42,
        manifest_sha256: 'a'.repeat(64),
      };
      writeFileSync(receiptPath, JSON.stringify(receipt));
      const healthyReceipt = readFileSync(receiptPath);
      const healthy = await cli([
        'extension',
        'install',
        'ops',
        '--repair',
        '--yes',
        '--prefix',
        prefix,
      ]);
      expect(healthy.status).toBe(0);
      expect(parseWholeStdout(healthy)).toMatchObject({ changed: false, retainedRelease: null });
      expect(realpathSync(path.join(prefix, 'lib/tmt-ops/current'))).toBe(release);
      expect(readFileSync(receiptPath).equals(healthyReceipt)).toBe(true);
      const tamperedPath = path.join(release, 'LICENSE');
      writeFileSync(tamperedPath, 'user edits\n');
      const before = readFileSync(tamperedPath);
      const quotedPrefix = `'${realpathSync(prefix).replace(/'/g, "'\\''")}'`;
      const repairCommand = `tmt extension install ops --repair --yes --prefix ${quotedPrefix}`;
      // Listing keeps going and reports the damaged entry with the same exact repair.
      const listed = await cli(['extension', 'list', '--prefix', prefix]);
      expect(listed.status).toBe(0);
      const squadRow = (
        parseWholeStdout(listed) as {
          extensions: { name: string; status?: string; hint?: string }[];
        }
      ).extensions.find((entry) => entry.name === 'ops');
      expect(squadRow?.status).toBe('repairRequired');
      expect(squadRow?.hint).toContain(repairCommand);
      expect(squadRow?.hint?.match(/tmt extension/g)).toHaveLength(1);
      for (const args of [
        ['extension', 'install', 'ops', '--yes', '--prefix', prefix],
        ['extension', 'upgrade', 'ops', '--yes', '--prefix', prefix],
      ]) {
        const error = expectError(await cli(args), 'EXTENSION_REPAIR_REQUIRED').error as {
          message: string;
        };
        expect(error.message).toContain(`Repair this release with: ${repairCommand}`);
        expect(error.message.match(/tmt extension/g)).toHaveLength(1);
        expect(error.message).not.toContain('Inspect with:');
        expect(readFileSync(tamperedPath).equals(before)).toBe(true);
      }
      const outside = path.join(sandbox.root, 'outside');
      writeFileSync(outside, 'outside sentinel');
      symlinkSync(outside, path.join(release, 'foreign-link'));
      const unsafe = await cli(['extension', 'list', '--prefix', prefix]);
      expect(unsafe.status).toBe(0);
      const unsafeRow = (
        parseWholeStdout(unsafe) as {
          extensions: { name: string; status?: string; hint?: string; detail?: string }[];
        }
      ).extensions.find((entry) => entry.name === 'ops');
      expect(unsafeRow?.status).toBe('invalid');
      expect(`${unsafeRow?.hint} ${unsafeRow?.detail}`).not.toContain('--repair');
      const error = expectError(
        await cli(['extension', 'install', 'ops', '--repair', '--yes', '--prefix', prefix]),
        'EXTENSION_INSTALLATION_INVALID'
      ).error as { message: string };
      expect(error.message).not.toContain('--repair');
      expect(readFileSync(outside, 'utf8')).toBe('outside sentinel');
      expect(readFileSync(tamperedPath).equals(before)).toBe(true);
      expect(readFileSync(receiptPath).equals(healthyReceipt)).toBe(true);
    });
  }, 60_000);

  it('repairs a local archive exactly and retains its damaged release and foreign files', async () => {
    await withSandbox(async (sandbox) => {
      const prefix = path.join(sandbox.root, 'local repair prefix');
      const squad = await createArtifact(sandbox, '0.1.0-alpha.1', new Uint8Array(), 'ops');
      const cli = (args: string[]) =>
        runCli(sandbox, [...args, '--json'], { deadlineMs: INSTALL_PROCESS_BUDGET_MS });
      const inputs = ['--archive', squad.archive, '--manifest', squad.manifest];
      expect(
        (
          await cli([
            'extension',
            'install',
            'ops',
            '--yes',
            '--prefix',
            prefix,
            '--channel',
            'alpha',
            ...inputs,
          ])
        ).status
      ).toBe(0);
      const current = path.join(prefix, 'lib/tmt-ops/current');
      const old = realpathSync(current);
      const receiptBytes = readFileSync(path.join(old, 'receipt.json'));
      const original = readFileSync(path.join(old, 'LICENSE'));
      writeFileSync(path.join(old, 'LICENSE'), 'user edits');
      writeFileSync(path.join(old, 'foreign.txt'), 'keep foreign content');
      const repair = ['extension', 'install', 'ops', '--repair', '--yes', '--prefix', prefix];
      // Listing reports the damaged entry with its exact repair instead of failing as a whole.
      const listed = await cli(['extension', 'list', '--prefix', prefix]);
      expect(listed.status).toBe(0);
      const row = (
        parseWholeStdout(listed) as {
          extensions: { name: string; status?: string; path?: string; hint?: string }[];
        }
      ).extensions.find((entry) => entry.name === 'ops');
      expect(row).toMatchObject({ status: 'repairRequired' });
      expect(row?.path).toBe(path.join(prefix, 'bin/tmt-ops'));
      expect(row?.hint).toContain(
        "--archive '<original-archive>' --manifest '<matching-manifest>'"
      );
      expect(row?.hint?.match(/tmt extension/g)).toHaveLength(1);
      const refused = await cli(repair);
      expect(refused.status).toBe(1);
      const error = expectError(refused, 'EXTENSION_REPAIR_REQUIRED').error as { message: string };
      expect(error.message).toContain(
        "--archive '<original-archive>' --manifest '<matching-manifest>'"
      );
      expect(error.message.match(/tmt extension/g)).toHaveLength(1);
      expect(realpathSync(current)).toBe(old);
      expectError(
        await cli(['extension', 'install', 'ops', '--repair', '--prefix', prefix, ...inputs]),
        'EXTENSION_CONSENT_REQUIRED'
      );
      const repaired = await cli([...repair, ...inputs]);
      expect(repaired.status).toBe(0);
      expect(parseWholeStdout(repaired)).toMatchObject({
        changed: true,
        retainedRelease: old,
        version: squad.version,
      });
      const active = realpathSync(current);
      expect(active).not.toBe(old);
      expect(readFileSync(path.join(active, 'LICENSE')).equals(original)).toBe(true);
      expect(existsSync(path.join(active, 'foreign.txt'))).toBe(false);
      expect(JSON.parse(readFileSync(path.join(active, 'receipt.json'), 'utf8')).source).toBe(
        'local-archive'
      );
      expect(readFileSync(path.join(old, 'LICENSE'), 'utf8')).toBe('user edits');
      expect(readFileSync(path.join(old, 'foreign.txt'), 'utf8')).toBe('keep foreign content');
      expect(readFileSync(path.join(old, 'receipt.json')).equals(receiptBytes)).toBe(true);
      expect(parseWholeStdout(await cli([...repair, ...inputs]))).toMatchObject({
        changed: false,
        retainedRelease: null,
      });
      expect(realpathSync(current)).toBe(active);
    });
  }, 60_000);

  it('declares optional uses, lists them, answers extensions.uses and warns before removing what they use', async () => {
    await withSandbox(async (sandbox) => {
      const cli = (args: string[]) =>
        runCli(sandbox, [...args, '--json'], { deadlineMs: INSTALL_PROCESS_BUDGET_MS });
      const uses = JSON.stringify({
        version: 1,
        uses: [
          {
            feature: 'browser-access',
            label: 'Browser access',
            extension: 'remote',
            requires: '>=0.1.0-alpha.1',
          },
        ],
      });
      const install = async (product: 'colab' | 'remote', version: string, declared?: string) => {
        const artifact = await createArtifact(
          sandbox,
          version,
          new Uint8Array(),
          product,
          undefined,
          {},
          undefined,
          declared
        );
        const result = await cli([
          'extension',
          'install',
          product,
          '--yes',
          '--channel',
          'alpha',
          '--archive',
          artifact.archive,
          '--manifest',
          artifact.manifest,
        ]);
        expect(result.status, result.stdout + result.stderr).toBe(0);
      };
      type UseRow = { available: boolean; reason: string | null; installed: string | null };
      const listed = async (): Promise<UseRow> => {
        const rows = parseWholeStdout(await cli(['extension', 'ls'])).extensions as {
          name: string;
          uses?: UseRow[];
        }[];
        return rows.find((row) => row.name === 'colab')!.uses![0];
      };
      const answer = async (extension: string) =>
        JSON.parse(
          (
            await runCli(sandbox, ['api'], {
              stdin: JSON.stringify({
                version: 1,
                operation: 'extensions.uses',
                input: { extension, feature: 'browser-access' },
              }),
            })
          ).stdout
        );

      await install('colab', '0.1.0-alpha.1', uses);
      expect(await listed()).toMatchObject({
        feature: 'browser-access',
        extension: 'remote',
        requires: '>=0.1.0-alpha.1',
        available: false,
        reason: 'missing',
        installed: null,
      });
      expect(await answer('colab')).toMatchObject({
        available: false,
        reason: 'missing',
        hint: 'Browser access needs the Remote extension: tmt extension install remote --yes',
      });
      const text = await runCli(sandbox, ['extension', 'ls']);
      expect(text.stdout).toContain('tmt extension install remote --yes');
      // The default prefix needs no --prefix in a printed command.
      expect(text.stdout).not.toContain('--prefix');

      await install('remote', '0.1.0-alpha.0');
      expect(await listed()).toMatchObject({ available: false, reason: 'tooOld' });
      expect((await answer('colab')).hint).toContain('tmt extension upgrade remote --yes');

      await install('remote', '0.1.0-alpha.2');
      expect(await listed()).toMatchObject({
        available: true,
        reason: null,
        installed: '0.1.0-alpha.2',
      });
      expect(await answer('colab')).toMatchObject({ available: true, reason: null, hint: '' });
      expect((await answer('remote')).error.code).toBe('EXTENSION_USE_UNDECLARED');

      // Removal warns about what stops working and still needs consent.
      const refused = await cli(['extension', 'rm', 'remote']);
      const message = (
        expectError(refused, 'EXTENSION_CONSENT_REQUIRED').error as { message: string }
      ).message;
      expect(message).toContain("colab's Browser access");
      const removed = await cli(['extension', 'rm', 'remote', '--yes']);
      expect(removed.status).toBe(0);
      expect(parseWholeStdout(removed)).toMatchObject({
        affects: [{ extension: 'colab', feature: 'browser-access', label: 'Browser access' }],
      });
      expect(await listed()).toMatchObject({ available: false, reason: 'missing' });
    });
  }, 120_000);

  it('rejects a release whose TMT-USES.json is malformed before anything is published', async () => {
    await withSandbox(async (sandbox) => {
      const cli = (args: string[]) =>
        runCli(sandbox, [...args, '--json'], { deadlineMs: INSTALL_PROCESS_BUDGET_MS });
      const artifact = await createArtifact(
        sandbox,
        '0.1.0-alpha.1',
        new Uint8Array(),
        'colab',
        undefined,
        {},
        undefined,
        '{"version":1,"uses":[{"feature":"Browser Access"}]}'
      );
      const refused = await cli([
        'extension',
        'install',
        'colab',
        '--yes',
        '--channel',
        'alpha',
        '--archive',
        artifact.archive,
        '--manifest',
        artifact.manifest,
      ]);
      expectError(refused, 'EXTENSION_INSTALL_FAILED');
      const rows = parseWholeStdout(await cli(['extension', 'ls'])).extensions as {
        name: string;
        installed: boolean;
      }[];
      expect(rows.find((row) => row.name === 'colab')?.installed).toBe(false);
    });
  }, 120_000);

  it('offers bundled skills, refreshes them by name on update, and uninstall removes every owned skill', async () => {
    await withSandbox(async (sandbox) => {
      const prefix = path.join(sandbox.root, 'extension prefix');
      const cli = (args: string[]) =>
        runCli(sandbox, [...args, '--json'], { deadlineMs: INSTALL_PROCESS_BUDGET_MS });
      const install = (artifact: { archive: string; manifest: string }, extra: string[] = []) =>
        cli([
          'extension',
          'install',
          'ops',
          '--yes',
          '--archive',
          artifact.archive,
          '--manifest',
          artifact.manifest,
          '--prefix',
          prefix,
          '--channel',
          'alpha',
          ...extra,
        ]);
      const squad = (version: string, skills: Record<string, string>) =>
        createArtifact(sandbox, version, new Uint8Array(), 'ops', undefined, skills);
      const first = await squad('0.1.0-alpha.1', {
        'tmt-ops/SKILL.md': 'lead v1\n',
        'tmt-ops/references/usage.md': 'usage\n',
        'tmt-ops-retired/SKILL.md': 'retired\n',
      });

      // --yes installs the extension but never publishes skills by itself.
      const offered = parseWholeStdout(await install(first)) as Record<string, unknown>;
      expect(offered.skills).toEqual({
        available: ['tmt-ops', 'tmt-ops-retired'],
        published: [],
        removed: [],
      });
      const human = await runCli(
        sandbox,
        [
          'extension',
          'install',
          'ops',
          '--yes',
          '--archive',
          first.archive,
          '--manifest',
          first.manifest,
          '--prefix',
          prefix,
        ],
        { deadlineMs: INSTALL_PROCESS_BUDGET_MS }
      );
      expect(human.stdout).toContain(
        '2 agent skills available (tmt-ops, tmt-ops-retired); publish with: tmt extension install ops --skills'
      );

      // --skills publishes the verified tree into every provider root.
      const accepted = parseWholeStdout(await install(first, ['--skills'])) as {
        skills: { published: Array<{ name: string; target: string }> };
      };
      const targets = accepted.skills.published.map((item) => item.target);
      expect(targets.length).toBeGreaterThan(0);
      // The provider roots this sandbox publishes into, from the report itself.
      const roots = [...new Set(targets.map((target) => path.dirname(target)))];
      const published = (name: string) =>
        roots
          .map((root) => path.join(root, name))
          .filter((target) => existsSync(target) && lstatSync(target).isSymbolicLink());
      for (const item of accepted.skills.published) {
        expect(item.target.startsWith(sandbox.home)).toBe(true);
        expect(lstatSync(item.target).isSymbolicLink()).toBe(true);
      }
      const lead = published('tmt-ops');
      expect(lead.length).toBeGreaterThan(0);
      for (const target of lead) {
        expect(readFileSync(path.join(target, 'SKILL.md'), 'utf8')).toBe('lead v1\n');
        expect(readFileSync(path.join(target, 'references/usage.md'), 'utf8')).toBe('usage\n');
      }

      // A playbook the same owner holds, outside the release tree.
      const playbook = await runCli(sandbox, ['api'], {
        stdin: JSON.stringify({
          version: 1,
          operation: 'skills.install',
          input: {
            owner: 'ops',
            consent: true,
            skills: [{ name: 'squad-playbook', files: [{ path: 'SKILL.md', content: 'play' }] }],
          },
        }),
      });
      expect(playbook.status, playbook.stdout).toBe(0);

      // An update refreshes held tree skills, removes the one it dropped by
      // name, and leaves the playbook alone.
      const second = await squad('0.2.0-alpha.1', { 'tmt-ops/SKILL.md': 'lead v2\n' });
      const updated = parseWholeStdout(await install(second)) as {
        version: string;
        skills: { available: string[]; removed: string[] };
      };
      expect(updated.version).toBe('0.2.0-alpha.1');
      expect(updated.skills.available).toEqual(['tmt-ops']);
      expect(updated.skills.removed.length).toBeGreaterThan(0);
      for (const target of published('tmt-ops'))
        expect(readFileSync(path.join(target, 'SKILL.md'), 'utf8')).toBe('lead v2\n');
      expect(published('tmt-ops-retired')).toEqual([]);
      const plays = published('squad-playbook');
      expect(plays.length).toBeGreaterThan(0);

      // A folder nobody manages is never touched.
      const foreign = path.join(path.dirname(plays[0]!), 'someone-elses-skill');
      mkdirSync(foreign);
      writeFileSync(path.join(foreign, 'SKILL.md'), 'mine');

      // Uninstall removes every skill the owner holds, playbooks included.
      const removed = parseWholeStdout(
        await cli(['extension', 'uninstall', 'ops', '--yes', '--prefix', prefix])
      ) as { skillsRemoved: string[]; skillsKept: string[]; kept: string[] };
      expect(removed.skillsRemoved.length).toBe(lead.length + plays.length);
      expect(removed.skillsKept).toEqual([]);
      expect(removed.kept).toEqual(['releases', 'hookConsent']);
      expect(published('tmt-ops')).toEqual([]);
      expect(published('squad-playbook')).toEqual([]);
      expect(readFileSync(path.join(foreign, 'SKILL.md'), 'utf8')).toBe('mine');
      expect(existsSync(path.join(prefix, 'lib/tmt-ops/releases'))).toBe(true);
    });
  }, 90_000);

  it('installs canonical Colab skill bytes, refreshes managed links and preserves unmanaged conflicts', async () => {
    await withSandbox(async (sandbox) => {
      const canonical = readFileSync(
        new URL('../../../extensions/tmt-colab/skills/tmt-colab/SKILL.md', import.meta.url),
        'utf8'
      );
      const prefix = path.join(sandbox.root, 'Colab skill prefix');
      const artifact = (version: string, content: string) =>
        createArtifact(sandbox, version, new Uint8Array(), 'colab', undefined, {
          'tmt-colab/SKILL.md': content,
        });
      const install = (input: ArtifactFixture, skills = false) =>
        runCli(
          sandbox,
          [
            'extension',
            'install',
            'colab',
            '--yes',
            '--archive',
            input.archive,
            '--manifest',
            input.manifest,
            '--prefix',
            prefix,
            ...(skills ? ['--skills'] : []),
            '--json',
          ],
          { deadlineMs: INSTALL_PROCESS_BUDGET_MS }
        );
      const first = await artifact('0.1.0-alpha.1', canonical);
      const offered = await install(first);
      expect(offered.status, offered.stdout + offered.stderr).toBe(0);
      expect(parseWholeStdout(offered).skills).toEqual({
        available: ['tmt-colab'],
        published: [],
        removed: [],
      });
      const accepted = await install(first, true);
      expect(accepted.status, accepted.stdout + accepted.stderr).toBe(0);
      const {
        skills: { published },
      } = parseWholeStdout(accepted) as {
        skills: { published: Array<{ name: string; target: string }> };
      };
      expect(published.length).toBeGreaterThan(0);
      for (const item of published) {
        expect(item.name).toBe('tmt-colab');
        expect(item.target.startsWith(sandbox.home)).toBe(true);
        expect(lstatSync(item.target).isSymbolicLink()).toBe(true);
        expect(readFileSync(path.join(item.target, 'SKILL.md'), 'utf8')).toBe(canonical);
      }
      const readout = await runCli(
        {
          ...sandbox,
          cli: {
            executable: path.join(prefix, 'bin/tmt-colab'),
            args: [],
          },
        },
        ['skill']
      );
      expect(readout.status, readout.stdout + readout.stderr).toBe(0);
      expect(readout.stdout).toBe(canonical);
      expect(readout.stderr).toBe('');
      const updatedBytes = canonical + '\nUpdated installation fixture.\n';
      const updated = await install(await artifact('0.1.0-alpha.2', updatedBytes));
      expect(updated.status, updated.stdout + updated.stderr).toBe(0);
      for (const item of published)
        expect(readFileSync(path.join(item.target, 'SKILL.md'), 'utf8')).toBe(updatedBytes);
      // Modified content is user-owned: a newer extension stays installed without overwriting it.
      const conflict = published[0]!.target;
      unlinkSync(conflict);
      mkdirSync(conflict);
      writeFileSync(path.join(conflict, 'SKILL.md'), 'User-owned Colab instructions.\n');
      const refused = await install(await artifact('0.1.0-alpha.3', canonical), true);
      expectError(refused, 'EXTENSION_SKILLS_FAILED');
      expect(readFileSync(path.join(conflict, 'SKILL.md'), 'utf8')).toBe(
        'User-owned Colab instructions.\n'
      );
      expect(readlinkSync(path.join(prefix, 'bin/tmt-colab'))).toBe(
        '../lib/tmt-colab/current/tmt-colab'
      );
      const listed = await runCli(sandbox, ['extension', 'ls', '--prefix', prefix, '--json']);
      expect(listed.status, listed.stdout + listed.stderr).toBe(0);
      expect(parseWholeStdout(listed)).toMatchObject({
        extensions: expect.arrayContaining([
          expect.objectContaining({ name: 'colab', installed: true, version: '0.1.0-alpha.3' }),
        ]),
      });
      expect(existsSync(path.join(sandbox.globalDir, 'colab', 'door.sock'))).toBe(false);
      expect(existsSync(sandbox.database)).toBe(false);
    });
  }, 90_000);

  it('keeps the extension installed when its skills cannot be published', async () => {
    await withSandbox(async (sandbox) => {
      const prefix = path.join(sandbox.root, 'extension prefix');
      const artifact = await createArtifact(
        sandbox,
        '0.1.0-alpha.1',
        new Uint8Array(),
        'ops',
        undefined,
        { 'tmt-ops/SKILL.md': 'lead\n' }
      );
      // An unmanaged folder already holds the name in the Claude root.
      const conflict = path.join(sandbox.home, '.claude/skills/tmt-ops');
      mkdirSync(conflict, { recursive: true });
      writeFileSync(path.join(conflict, 'SKILL.md'), 'hand-written');
      const result = await runCli(
        sandbox,
        [
          'extension',
          'install',
          'ops',
          '--yes',
          '--skills',
          '--archive',
          artifact.archive,
          '--manifest',
          artifact.manifest,
          '--prefix',
          prefix,
          '--channel',
          'alpha',
          '--json',
        ],
        { deadlineMs: INSTALL_PROCESS_BUDGET_MS }
      );
      expectError(result, 'EXTENSION_SKILLS_FAILED');
      expect(readFileSync(path.join(conflict, 'SKILL.md'), 'utf8')).toBe('hand-written');
      expect(readlinkSync(path.join(prefix, 'bin/tmt-ops'))).toBe('../lib/tmt-ops/current/tmt-ops');
    });
  }, 60_000);
});

describe('aggregate official upgrades', () => {
  it(
    'keeps independent CLI/Squad/Remote/Colab pins and skips frozen Office even with invalid state',
    { timeout: 60000 },
    async () => {
      await withReleaseSandbox(async (sandbox) => {
        const prefix = path.join(sandbox.root, 'native install prefix with spaces');
        const install = async (
          artifact: ArtifactFixture,
          product: 'cli' | 'ops' | 'remote' | 'colab'
        ) => {
          const result = await runCli(
            sandbox,
            [
              '__native-install',
              '--archive',
              artifact.archive,
              '--manifest',
              artifact.manifest,
              '--prefix',
              prefix,
              '--channel',
              'alpha',
              '--product',
              product,
              '--pin',
              '--json',
            ],
            { deadlineMs: INSTALL_PROCESS_BUDGET_MS }
          );
          expect(result.status, result.stdout + result.stderr).toBe(0);
          expect(result.stderr).toBe('');
          return parseWholeStdout(result) as { executable: string };
        };
        const version = (await runCli(sandbox, ['--version'])).stdout.trim();
        const cliArtifact = await createArtifact(sandbox, version);
        const cli = await install(cliArtifact, 'cli');
        const extensionReceipts = new Map<string, Buffer>();
        for (const product of ['ops', 'remote', 'colab'] as const) {
          const artifact = await createArtifact(
            sandbox,
            '0.1.0-alpha.1',
            new Uint8Array(),
            product
          );
          await install(artifact, product);
          const receipt = path.join(prefix, `lib/tmt-${product}/current/receipt.json`);
          extensionReceipts.set(receipt, readFileSync(receipt));
        }
        const managed = { ...sandbox, cli: { executable: cli.executable, args: [] } };
        const first = await runCli(managed, ['upgrade', '--yes', '--json'], {
          deadlineMs: INSTALL_PROCESS_BUDGET_MS,
        });
        expect(first.status, first.stderr + first.stdout).toBe(0);
        expect(parseWholeStdout(first).products).toMatchObject([
          { product: 'cli', status: 'skippedPinned' },
          {
            product: 'ops',
            status: 'skippedPinned',
            hint: 'tmt extension upgrade ops --unpin',
          },
          {
            product: 'remote',
            status: 'skippedPinned',
            hint: 'tmt extension upgrade remote --unpin',
          },
          {
            product: 'colab',
            status: 'skippedPinned',
            hint: 'tmt extension upgrade colab --unpin',
          },
        ]);
        writeFileSync(path.join(prefix, 'bin/tmt-office'), 'not a managed installation');
        const skipped = await runCli(managed, ['upgrade', '--yes', '--json'], {
          deadlineMs: INSTALL_PROCESS_BUDGET_MS,
        });
        expect(skipped.status, skipped.stderr + skipped.stdout).toBe(0);
        expect(parseWholeStdout(skipped).products).toMatchObject([
          { product: 'cli', status: 'skippedPinned' },
          { product: 'ops', status: 'skippedPinned' },
          { product: 'remote', status: 'skippedPinned' },
          { product: 'colab', status: 'skippedPinned' },
        ]);
        for (const [receipt, before] of extensionReceipts) {
          expect(readFileSync(receipt).equals(before)).toBe(true);
        }
        expect(existsSync(sandbox.database)).toBe(false);
      });
    }
  );
});
