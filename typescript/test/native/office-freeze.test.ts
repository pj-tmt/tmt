import {
  existsSync,
  mkdirSync,
  readFileSync,
  readlinkSync,
  unlinkSync,
  writeFileSync,
} from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import { expectError, parseWholeStdout, runCli } from '../support/cli-process.js';
import { createArtifact } from '../support/native-artifact.js';
import { withReleaseSandbox } from '../support/native-installation.js';
import { workspaceVersion } from '../support/workspace-version.js';

describe('frozen Office extension lifecycle', () => {
  it.each(['absent', 'installed', 'partiallyRemoved'] as const)(
    'refuses acquisition, preserves listing/removal and skips root upgrade with %s Office',
    { timeout: 60_000 },
    async (state) => {
      await withReleaseSandbox(async (sandbox) => {
        const prefix = path.join(sandbox.root, 'managed prefix');
        const version = (await runCli(sandbox, ['--version'])).stdout.trim();
        const cli = await createArtifact(sandbox, version);
        const installedCli = await runCli(
          sandbox,
          [
            '__native-install',
            '--product',
            'cli',
            '--channel',
            'alpha',
            '--archive',
            cli.archive,
            '--manifest',
            cli.manifest,
            '--prefix',
            prefix,
            '--pin',
            '--json',
          ],
          { deadlineMs: 15_000 }
        );
        expect(installedCli.status, installedCli.stdout + installedCli.stderr).toBe(0);
        const managed = {
          ...sandbox,
          cli: { executable: parseWholeStdout(installedCli).executable as string, args: [] },
        };
        const officeRoot = path.join(prefix, 'lib/tmt-office');
        let receipt: Buffer | undefined;
        let release: string | undefined;
        if (state !== 'absent') {
          // Existing receipts predate the freeze. The offline internal installer
          // constructs them without offering a new public Office installation.
          const office = await createArtifact(
            sandbox,
            workspaceVersion('tmt-office'),
            new Uint8Array(),
            'office'
          );
          const installed = await runCli(
            sandbox,
            [
              '__native-install',
              '--product',
              'office',
              '--channel',
              'alpha',
              '--archive',
              office.archive,
              '--manifest',
              office.manifest,
              '--prefix',
              prefix,
              '--json',
            ],
            { deadlineMs: 15_000 }
          );
          expect(installed.status, installed.stdout + installed.stderr).toBe(0);
          release = path.resolve(officeRoot, readlinkSync(path.join(officeRoot, 'current')));
          receipt = readFileSync(path.join(release, 'receipt.json'));
          if (state === 'partiallyRemoved') unlinkSync(path.join(prefix, 'bin/tmt-office'));
        }
        const data = path.join(sandbox.globalDir, 'office', 'office.db');
        mkdirSync(path.dirname(data), { recursive: true });
        writeFileSync(data, 'retained Office data');

        let frozenMessage: string | undefined;
        let frozenJson: string | undefined;
        for (const operation of ['install', 'upgrade']) {
          for (const yes of [[], ['--yes']]) {
            const result = await runCli(managed, [
              'extension',
              operation,
              'office',
              '--prefix',
              prefix,
              ...yes,
              '--json',
            ]);
            expectError(result, 'EXTENSION_FROZEN');
            const message = (parseWholeStdout(result).error as { message: string }).message;
            expect(message).toContain('office is frozen');
            frozenMessage ??= message;
            frozenJson ??= result.stdout;
            expect(message).toBe(frozenMessage);
          }
        }
        for (const [operation, flags] of [
          ['install', []],
          ['install', ['--yes']],
          ['install', ['--yes', '--force']],
          [
            'install',
            [
              '--yes',
              '--archive',
              path.join(sandbox.root, 'missing.tar.gz'),
              '--manifest',
              path.join(sandbox.root, 'missing-manifest.json'),
            ],
          ],
          ['upgrade', []],
          ['upgrade', ['--force']],
        ] as const) {
          const result = await runCli(managed, [
            'office',
            operation,
            '--prefix',
            prefix,
            ...flags,
            '--json',
          ]);
          expectError(result, 'EXTENSION_FROZEN');
          expect((parseWholeStdout(result).error as { message: string }).message).toBe(
            frozenMessage
          );
          expect(result.stdout).toBe(frozenJson);
        }
        const officialHuman = await runCli(managed, [
          'extension',
          'install',
          'office',
          '--prefix',
          prefix,
        ]);
        const facadeHuman = await runCli(managed, ['office', 'install', '--prefix', prefix]);
        expect(facadeHuman).toEqual(officialHuman);
        const repair = await runCli(managed, [
          'extension',
          'install',
          'office',
          '--repair',
          '--prefix',
          prefix,
          '--json',
        ]);
        expectError(repair, 'EXTENSION_FROZEN');
        expect((parseWholeStdout(repair).error as { message: string }).message).toBe(frozenMessage);

        // Failed acquisition would be visible immediately through these proxies;
        // Office's --check result must instead be a local frozen observation.
        managed.env.HTTPS_PROXY = managed.env.HTTP_PROXY = 'http://127.0.0.1:1';
        managed.env.NO_PROXY = '';
        for (const check of [[], ['--check']]) {
          const listed = await runCli(managed, [
            'extension',
            'ls',
            '--prefix',
            prefix,
            ...check,
            '--json',
          ]);
          expect(listed.status, listed.stdout + listed.stderr).toBe(0);
          const rows = parseWholeStdout(listed).extensions as Array<{
            name: string;
            update?: string;
          }>;
          const office = rows.find((row: { name: string }) => row.name === 'office');
          if (state === 'absent') expect(office).toBeUndefined();
          else {
            expect(office).toMatchObject({
              name: 'office',
              frozen: true,
              installed: state === 'installed',
            });
            if (check.length) expect(office?.update).toBe('frozen');
            if (state === 'partiallyRemoved') {
              expect(office).toMatchObject({
                status: 'partiallyRemoved',
                hint: expect.stringContaining('extension rm office --yes'),
              });
            }
          }
        }
        const human = await runCli(managed, ['extension', 'ls', '--prefix', prefix]);
        expect(human.status).toBe(0);
        if (state !== 'absent') expect(human.stdout).toContain('(frozen)');

        const upgraded = await runCli(managed, ['upgrade', '--yes', '--json'], {
          deadlineMs: 15_000,
        });
        expect(upgraded.status, upgraded.stdout + upgraded.stderr).toBe(0);
        expect(parseWholeStdout(upgraded).products).toMatchObject([
          { product: 'cli', status: 'skippedPinned' },
        ]);
        expect(parseWholeStdout(upgraded).products).toHaveLength(1);
        if (release)
          expect(readFileSync(path.join(release, 'receipt.json')).equals(receipt!)).toBe(true);
        else expect(existsSync(officeRoot)).toBe(false);

        const removed = await runCli(managed, [
          'extension',
          'rm',
          'office',
          '--yes',
          '--prefix',
          prefix,
          '--json',
        ]);
        expect(removed.status, removed.stdout + removed.stderr).toBe(0);
        expect(parseWholeStdout(removed)).toMatchObject({
          installed: false,
          changed: state !== 'absent',
        });
        expect(existsSync(path.join(prefix, 'bin/tmt-office'))).toBe(false);
        expect(existsSync(path.join(officeRoot, 'current'))).toBe(false);
        if (release)
          expect(readFileSync(path.join(release, 'receipt.json')).equals(receipt!)).toBe(true);
        expect(readFileSync(data, 'utf8')).toBe('retained Office data');
        const after = await runCli(managed, ['extension', 'ls', '--prefix', prefix, '--json']);
        expect(after.status).toBe(0);
        expect(
          (parseWholeStdout(after).extensions as Array<{ name: string }>).map((row) => row.name)
        ).toEqual(['ops', 'remote', 'colab']);
      });
    }
  );
});
