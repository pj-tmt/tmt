import {
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  readlinkSync,
  realpathSync,
  symlinkSync,
  writeFileSync,
  unlinkSync,
} from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import path from 'node:path';
import { createHash, randomUUID } from 'node:crypto';
import { describe, expect, it } from 'vite-plus/test';
import { expectError, parseWholeStdout, runCli, withSandbox } from '../support/cli-process.js';
import { createArtifact, type ArtifactFixture } from '../support/native-artifact.js';

import {
  INSTALL_PROCESS_BUDGET_MS,
  installPrefix,
  install,
  currentPointer,
  receiptPath,
} from '../support/native-installation.js';

function currentReleaseId(prefix: string): string {
  return readlinkSync(currentPointer(prefix)).replace(/^releases\//, '');
}

function releaseIds(prefix: string): string[] {
  return readdirSync(path.join(prefix, 'lib', 'tmux-team', 'releases')).sort();
}

function receipt(prefix: string): Record<string, unknown> {
  return JSON.parse(readFileSync(receiptPath(prefix), 'utf8')) as Record<string, unknown>;
}

function artifactChecksum(fixture: ArtifactFixture): string {
  const manifest = JSON.parse(readFileSync(fixture.manifest, 'utf8')) as {
    artifacts: Record<string, { checksums: { sha256: string } }>;
  };
  return manifest.artifacts[path.basename(fixture.archive)].checksums.sha256;
}

describe('native installation process contract', () => {
  it(
    'versioned candidate handoff fences publication and retains provenance and same-version repair',
    { timeout: 60_000 },
    async () => {
      await withSandbox(async (sandbox) => {
        const probe = await runCli(sandbox, [
          '__native-install',
          '--handoff-version',
          '1',
          '--probe',
          '--json',
        ]);
        expect(probe.status).toBe(0);
        expect(probe.stderr).toBe('');
        expect(parseWholeStdout(probe)).toEqual({ protocol: 1 });
        expect(existsSync(sandbox.database)).toBe(false);
        const version = (await runCli(sandbox, ['--version'])).stdout.trim();
        const old = await createArtifact(sandbox, '0.0.1-alpha.1');
        const prefix = installPrefix(sandbox);
        await install(sandbox, old, prefix);
        const oldId = currentReleaseId(prefix);
        const oldPath = realpathSync(path.join(prefix, 'bin', 'tmt'));
        const oldBytes = readFileSync(oldPath);
        const oldReceipt = readFileSync(receiptPath(prefix));
        const candidate = await createArtifact(sandbox, version);
        const request = {
          protocol: 1,
          archive: candidate.archive,
          manifest: candidate.manifest,
          prefix: realpathSync(prefix),
          target: candidate.target,
          version,
          archive_sha256: artifactChecksum(candidate),
          manifest_sha256: createHash('sha256')
            .update(readFileSync(candidate.manifest))
            .digest('hex'),
          channel: 'alpha',
          pin: 'preserve',
          expected_current: oldId,
          release_id: 1454,
        };
        const handoff = (input: unknown) =>
          runCli(sandbox, ['__native-install', '--handoff-version', '1', '--json'], {
            stdin: JSON.stringify(input),
            deadlineMs: INSTALL_PROCESS_BUDGET_MS,
          });
        for (const change of [
          { protocol: 2 },
          { archive: 'relative.tar.gz' },
          { manifest: 'relative.json' },
          { prefix: 'relative-prefix' },
          { manifest_sha256: '0'.repeat(64) },
          { archive_sha256: '0'.repeat(64) },
          { target: 'unsupported-target' },
          { version: '0.0.0' },
          { expected_current: randomUUID() },
          { extra: true },
        ]) {
          const rejected = await handoff({ ...request, ...change });
          expect(rejected.status, rejected.stdout + rejected.stderr).toBe(1);
          expect(parseWholeStdout(rejected).installation).toBeNull();
          expect(currentReleaseId(prefix)).toBe(oldId);
          expect(readFileSync(receiptPath(prefix))).toEqual(oldReceipt);
          expect(readFileSync(oldPath).equals(oldBytes)).toBe(true);
        }
        const accepted = await handoff(request);
        expect(accepted.status, accepted.stdout + accepted.stderr).toBe(0);
        expect(accepted.stderr).toBe('');
        const report = parseWholeStdout(accepted);
        expect(report).toMatchObject({
          protocol: 1,
          error: null,
          installation: { version, changed: true },
        });
        expect(currentReleaseId(prefix)).not.toBe(oldId);
        expect(receipt(prefix).source).toEqual({
          kind: 'github-release',
          repository: 'pj-tmt/tmt',
          release_id: 1454,
          manifest_sha256: request.manifest_sha256,
        });
        expect(readFileSync(oldPath).equals(oldBytes)).toBe(true);
        const refreshed = await runCli(
          { ...sandbox, cli: { executable: path.join(prefix, 'bin', 'tmt'), args: [] } },
          ['__native-refresh-skills', '--managed', '--json']
        );
        expect(refreshed.status, refreshed.stdout + refreshed.stderr).toBe(0);
        expect(parseWholeStdout(refreshed)).toEqual({ refreshed: [], skipped: [], conflicts: [] });
        expect(existsSync(sandbox.database)).toBe(false);
        const activeId = currentReleaseId(prefix);
        unlinkSync(path.join(prefix, 'bin', 'tmux-team'));
        const repeated = await handoff({ ...request, expected_current: activeId });
        expect(repeated.status, repeated.stdout + repeated.stderr).toBe(0);
        expect(parseWholeStdout(repeated)).toMatchObject({ installation: { changed: false } });
        expect(currentReleaseId(prefix)).toBe(activeId);
        expect(realpathSync(path.join(prefix, 'bin', 'tmux-team'))).toBe(
          realpathSync(path.join(prefix, 'bin', 'tmt'))
        );
      });
    }
  );

  it('prints the requested command path even through an aliased prefix ancestor', async () => {
    await withSandbox(async (sandbox) => {
      const version = (await runCli(sandbox, ['--version'])).stdout.trim();
      const fixture = await createArtifact(sandbox, version);
      const physical = path.join(sandbox.root, 'physical');
      const alias = path.join(sandbox.root, 'alias');
      mkdirSync(physical);
      symlinkSync(physical, alias);
      const prefix = path.join(alias, 'prefix');
      const args = [
        '__native-install',
        '--archive',
        fixture.archive,
        '--manifest',
        fixture.manifest,
        '--prefix',
        prefix,
        '--channel',
        'alpha',
      ];
      for (const verb of ['Installed', 'Current']) {
        const result = await runCli(sandbox, args, { deadlineMs: INSTALL_PROCESS_BUDGET_MS });
        expect(result.status).toBe(0);
        expect(result.stdout).toContain(
          `${verb} tmt ${version} at ${path.join(prefix, 'bin', 'tmt')}`
        );
        expect(result.stdout).not.toContain(realpathSync(physical));
        expect(result.stderr).toBe('');
      }
    });
  });

  it('retains a generated operation ID across a companion storage uncertainty and replay', async () => {
    await withSandbox(async (sandbox) => {
      const prefix = installPrefix(sandbox);
      const companion = path.join(sandbox.root, 'storage-uncertain-office');
      writeExecutable(
        companion,
        `#!/bin/sh
control="\${0%/lib/tmt-office/releases/*}/storage-uncertain-operation"
case "$3" in
probe) printf 'TMT-OFFICE/1\\n0.1.0-alpha.2\\n' ;;
capabilities) printf 'TMT-OFFICE-CAPABILITIES/1\\noffice_board_v1\\n' ;;
board-post)
  input="$(cat)"
  operation_id="$(printf '%s' "$input" | sed -n 's/.*"operationId":"\\([^"]*\\)".*/\\1/p')"
  if [ ! -f "$control" ]; then
    printf '%s' "$operation_id" > "$control"
    printf '{"error":"STORAGE_ERROR"}'
  elif [ "$(cat "$control")" = "$operation_id" ]; then
    printf '{"entryId":"11111111-1111-4111-8111-111111111111","threadId":"11111111-1111-4111-8111-111111111111","revision":1,"created":true,"operationId":"%s"}' "$operation_id"
  else
    printf '{"error":"BOARD_IDEMPOTENCY_CONFLICT"}'
  fi
  ;;
*) exit 1 ;;
esac
`,
        0o755
      );
      const fixture = await createArtifact(
        sandbox,
        '0.1.0-alpha.2',
        new Uint8Array(),
        'office',
        companion
      );
      const office = (args: string[]) =>
        runCli(sandbox, ['office', '--prefix', prefix, ...args, '--json'], {
          deadlineMs: 30_000,
        });
      expect(
        (
          await runCli(
            sandbox,
            [
              '__native-install',
              '--product',
              'office',
              '--channel',
              'alpha',
              '--prefix',
              prefix,
              '--archive',
              fixture.archive,
              '--manifest',
              fixture.manifest,
              '--json',
            ],
            { deadlineMs: 30_000 }
          )
        ).status
      ).toBe(0);

      const mutation = [
        'board',
        'post',
        '--general',
        '--owner',
        '--title',
        'possibly committed',
        '--body',
        'exact body',
      ];
      const firstAttempt = await office(mutation);
      expect(firstAttempt.status).toBe(1);
      const uncertain = expectError(firstAttempt, 'OFFICE_LOCAL_UNCERTAIN');
      const message = (uncertain.error as { message: string }).message;
      const match = message.match(
        /--operation-id ([0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12})\.$/
      );
      expect(match).not.toBeNull();
      const operationId = match![1];
      expect(readFileSync(path.join(prefix, 'storage-uncertain-operation'), 'utf8')).toBe(
        operationId
      );

      const replay = parseWholeStdout(await office([...mutation, '--operation-id', operationId]));
      expect(replay).toEqual({
        entryId: '11111111-1111-4111-8111-111111111111',
        threadId: '11111111-1111-4111-8111-111111111111',
        revision: 1,
        created: true,
        operationId,
      });
      expect(parseWholeStdout(await office([...mutation, '--operation-id', operationId]))).toEqual(
        replay
      );
    });
  });

  it(
    'installs a real copied native executable from spaced paths and runs it with an empty PATH',
    { timeout: 60_000 },
    async () => {
      await withSandbox(async (sandbox) => {
        const versionResult = await runCli(sandbox, ['--version']);
        expect(versionResult.status).toBe(0);
        expect(versionResult.stderr).toBe('');
        const version = versionResult.stdout.trim();
        expect(version).toMatch(/^\d+\.\d+\.\d+-alpha(?:\.[0-9A-Za-z-]+)?$/);
        const fixture = await createArtifact(sandbox, version);
        const prefix = installPrefix(sandbox);
        const installed = await install(sandbox, fixture, prefix);
        expect(installed).toEqual({
          executable: path.join(realpathSync(prefix), 'bin', 'tmt'),
          version,
          changed: true,
        });
        expect(readlinkSync(installed.executable)).toBe('../lib/tmux-team/current/tmt');
        expect(existsSync(sandbox.database)).toBe(false);

        sandbox.env.PATH = '';
        const moved = await runCli(
          { ...sandbox, cli: { executable: installed.executable, args: [] } },
          ['--version']
        );
        expect(moved.status).toBe(0);
        expect(moved.stdout).toBe(`${version}\n`);
        expect(moved.stderr).toBe('');
        expect(existsSync(sandbox.database)).toBe(false);
      });
    }
  );

  it(
    'repeats as a no-op while application-state selectors change',
    { timeout: 60_000 },
    async () => {
      await withSandbox(async (sandbox) => {
        const versionResult = await runCli(sandbox, ['--version']);
        const fixture = await createArtifact(sandbox, versionResult.stdout.trim());
        const prefix = installPrefix(sandbox);
        const first = await install(sandbox, fixture, prefix);
        const pointer = readlinkSync(currentPointer(prefix));
        const before = readFileSync(path.join(prefix, 'lib', 'tmux-team', pointer, 'tmt'));

        const firstHome = path.join(sandbox.root, 'unrelated tmux state one');
        const second = await install(sandbox, fixture, prefix, [], firstHome);
        const secondHome = path.join(sandbox.root, 'unrelated tmux state two');
        const third = await install(sandbox, fixture, prefix, [], secondHome);

        expect(first.changed).toBe(true);
        expect(second).toEqual({ ...first, changed: false });
        expect(third).toEqual({ ...first, changed: false });
        expect(readlinkSync(currentPointer(prefix))).toBe(pointer);
        expect(
          readFileSync(path.join(prefix, 'lib', 'tmux-team', pointer, 'tmt')).equals(before)
        ).toBe(true);
        expect(releaseIds(prefix)).toHaveLength(1);
        expect(existsSync(firstHome)).toBe(false);
        expect(existsSync(secondHome)).toBe(false);
        expect(existsSync(sandbox.database)).toBe(false);
      });
    }
  );

  it(
    'records pin and unpin transitions while retaining the old release bytes',
    { timeout: 60_000 },
    async () => {
      await withSandbox(async (sandbox) => {
        const versionResult = await runCli(sandbox, ['--version']);
        const fixture = await createArtifact(sandbox, versionResult.stdout.trim());
        const prefix = installPrefix(sandbox);
        await install(sandbox, fixture, prefix);
        const originalId = currentReleaseId(prefix);
        const originalBytes = readFileSync(
          path.join(prefix, 'lib', 'tmux-team', 'releases', originalId, 'tmt')
        );

        const pinned = await install(sandbox, fixture, prefix, ['--pin']);
        const pinnedId = currentReleaseId(prefix);
        expect(pinned.changed).toBe(true);
        expect(pinnedId).not.toBe(originalId);
        expect(receipt(prefix)).toMatchObject({
          version: fixture.version,
          channel: 'alpha',
          pinned_version: fixture.version,
        });
        const managed = {
          ...sandbox,
          cli: { executable: pinned.executable, args: [] },
          env: { ...sandbox.env, PATH: path.dirname(pinned.executable) },
        };
        const pinnedReceipt = readFileSync(receiptPath(prefix));
        for (const command of ['upgrade', 'update']) {
          const result = await runCli(managed, [command, '--json']);
          expect(result.status).toBe(0);
          expect(result.stderr).toBe('');
          expect(parseWholeStdout(result)).toMatchObject({
            version: fixture.version,
            changed: false,
            channel: 'alpha',
            pinned: true,
            pinnedVersion: fixture.version,
            skippedPinned: true,
            products: [
              { product: 'cli', status: 'skippedPinned', version: fixture.version, error: null },
            ],
            skills: null,
            pathWarning: null,
          });
        }
        const shadowed = await runCli({ ...managed, env: { ...managed.env, PATH: '' } }, [
          'upgrade',
          '--json',
        ]);
        expect(shadowed.status).toBe(0);
        expect(parseWholeStdout(shadowed).pathWarning).toContain('PATH does not select');
        const badChannel = await runCli(managed, ['upgrade', '--channel', 'stable', '--json']);
        expect(badChannel.status).toBe(1);
        expectError(
          badChannel,
          'NATIVE_UPGRADE_FAILED',
          'The installation is pinned; explicitly select a version or unpin it.'
        );
        expect(readFileSync(receiptPath(prefix))).toEqual(pinnedReceipt);
        expect(existsSync(sandbox.database)).toBe(false);
        expect(
          readFileSync(path.join(prefix, 'lib', 'tmux-team', 'releases', originalId, 'tmt')).equals(
            originalBytes
          )
        ).toBe(true);

        const unpinned = await install(sandbox, fixture, prefix, ['--unpin']);
        const unpinnedId = currentReleaseId(prefix);
        expect(unpinned.changed).toBe(true);
        const stale = await runCli(
          {
            ...sandbox,
            cli: {
              executable: path.join(prefix, 'lib', 'tmux-team', 'releases', pinnedId, 'tmt'),
              args: [],
            },
          },
          ['upgrade', '--json']
        );
        expect(stale.status).toBe(1);
        expectError(
          stale,
          'NATIVE_UPGRADE_FAILED',
          'This executable is not the active managed release. Run the current native installation, or update using its original package manager.'
        );
        expect(currentReleaseId(prefix)).toBe(unpinnedId);
        expect(unpinnedId).not.toBe(pinnedId);
        expect(receipt(prefix)).toMatchObject({
          version: fixture.version,
          channel: 'alpha',
          pinned_version: null,
        });
        expect(releaseIds(prefix)).toEqual([originalId, pinnedId, unpinnedId].sort());
        expect(
          readFileSync(path.join(prefix, 'lib', 'tmux-team', 'releases', originalId, 'tmt')).equals(
            originalBytes
          )
        ).toBe(true);
        expect(existsSync(sandbox.database)).toBe(false);
      });
    }
  );

  it(
    'rejects an equal-version artifact with changed file bytes even when its archive digest is forged into the receipt',
    { timeout: 60_000 },
    async () => {
      await withSandbox(async (sandbox) => {
        const versionResult = await runCli(sandbox, ['--version']);
        const version = versionResult.stdout.trim();
        const originalFixture = await createArtifact(sandbox, version);
        const prefix = installPrefix(sandbox);
        await install(sandbox, originalFixture, prefix);
        const pointer = readlinkSync(currentPointer(prefix));
        const originalExecutable = readFileSync(
          path.join(prefix, 'lib', 'tmux-team', pointer, 'tmt')
        );
        const originalReceipt = receipt(prefix);
        const candidate = await createArtifact(
          sandbox,
          version,
          Buffer.from('\nintentionally different candidate bytes\n')
        );
        const forgedReceipt = {
          ...originalReceipt,
          archive_sha256: artifactChecksum(candidate),
        };
        const forgedReceiptBytes = Buffer.from(`${JSON.stringify(forgedReceipt)}\n`);
        writeFileSync(receiptPath(prefix), forgedReceiptBytes);

        // This candidate is intentionally never executed; it only tests equal-version file integrity.
        const result = await runCli(
          sandbox,
          [
            '__native-install',
            '--archive',
            candidate.archive,
            '--manifest',
            candidate.manifest,
            '--prefix',
            prefix,
            '--channel',
            'alpha',
            '--json',
          ],
          { deadlineMs: INSTALL_PROCESS_BUDGET_MS }
        );
        expect(result.status).toBe(1);
        expectError(
          result,
          'NATIVE_INSTALL_FAILED',
          'Installed target or equal-version artifact integrity does not match.'
        );
        expect(readlinkSync(currentPointer(prefix))).toBe(pointer);
        expect(
          readFileSync(path.join(prefix, 'lib', 'tmux-team', pointer, 'tmt')).equals(
            originalExecutable
          )
        ).toBe(true);
        expect(readFileSync(receiptPath(prefix))).toEqual(forgedReceiptBytes);
        expect(receipt(prefix).file_sha256).toEqual(originalReceipt.file_sha256);
        expect(existsSync(sandbox.database)).toBe(false);
      });
    }
  );

  it(
    'refuses unmanaged collisions and a tampered receipt without changing owned bytes or pointer',
    { timeout: 60_000 },
    async () => {
      await withSandbox(async (sandbox) => {
        const versionResult = await runCli(sandbox, ['--version']);
        const fixture = await createArtifact(sandbox, versionResult.stdout.trim());
        const prefix = installPrefix(sandbox);
        mkdirSync(path.join(prefix, 'bin'), { recursive: true });
        const collision = path.join(prefix, 'bin', 'tmt');
        const collisionBytes = Buffer.from('user-owned command\n');
        writeFileSync(collision, collisionBytes);
        const collisionResult = await runCli(
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
          { deadlineMs: INSTALL_PROCESS_BUDGET_MS }
        );
        expect(collisionResult.status).toBe(1);
        expectError(collisionResult, 'NATIVE_INSTALL_FAILED');
        expect(readFileSync(collision)).toEqual(collisionBytes);
        expect(existsSync(currentPointer(prefix))).toBe(false);

        const ownedPrefix = path.join(sandbox.root, 'owned prefix');
        await install(sandbox, fixture, ownedPrefix);
        const pointer = readlinkSync(currentPointer(ownedPrefix));
        const executable = readFileSync(path.join(ownedPrefix, 'lib', 'tmux-team', pointer, 'tmt'));
        writeFileSync(receiptPath(ownedPrefix), '{ malformed receipt');
        const tampered = await runCli(
          sandbox,
          [
            '__native-install',
            '--archive',
            fixture.archive,
            '--manifest',
            fixture.manifest,
            '--prefix',
            ownedPrefix,
            '--channel',
            'alpha',
            '--json',
          ],
          { deadlineMs: INSTALL_PROCESS_BUDGET_MS }
        );
        expect(tampered.status).toBe(1);
        expectError(tampered, 'NATIVE_INSTALL_FAILED');
        expect(readlinkSync(currentPointer(ownedPrefix))).toBe(pointer);
        expect(
          readFileSync(path.join(ownedPrefix, 'lib', 'tmux-team', pointer, 'tmt')).equals(
            executable
          )
        ).toBe(true);
        expect(existsSync(sandbox.database)).toBe(false);
      });
    }
  );

  it('keeps the internal installer out of help and completion output', async () => {
    await withSandbox(async (sandbox) => {
      const help = await runCli(sandbox, ['help']);
      expect(help.status).toBe(0);
      expect(help.stdout).not.toContain('__native-install');
      expect(help.stdout).not.toContain('--archive');
      for (const shell of ['bash', 'zsh']) {
        const completion = await runCli(sandbox, ['__completion-script', shell]);
        expect(completion.status).toBe(0);
        expect(completion.stdout).not.toContain('__native-install');
        expect(completion.stdout).not.toContain('--product');
        // Offline Office installation is public; only the internal publisher is hidden.
        expect(completion.stdout).toContain('--archive');
      }
      expect(existsSync(sandbox.database)).toBe(false);
    });
  });
});
