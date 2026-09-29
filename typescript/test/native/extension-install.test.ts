import {
  existsSync,
  readlinkSync,
  realpathSync,
  writeFileSync,
  chmodSync,
  mkdirSync,
} from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { expectError, parseWholeStdout, runCli, withSandbox } from '../support/cli-process.js';
import { createArtifact } from '../support/native-artifact.js';

const INSTALL_PROCESS_BUDGET_MS = 15_000;

describe('tmt extension install surface', () => {
  it('installs, lists offline, refuses without consent, and uninstalls squad keeping its releases', async () => {
    await withSandbox(async (sandbox) => {
      const prefix = path.join(sandbox.root, 'extension prefix');
      const squad = await createArtifact(sandbox, '0.1.0-alpha.1', new Uint8Array(), 'squad');
      const cli = (args: string[]) =>
        runCli(sandbox, [...args, '--json'], { deadlineMs: INSTALL_PROCESS_BUDGET_MS });

      // Consent is never assumed in a non-interactive run.
      const refused = await cli([
        'extension',
        'install',
        'squad',
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
        'squad',
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
        extension: 'squad',
        installed: true,
        changed: true,
        version: '0.1.0-alpha.1',
        executable: path.join(realpathSync(prefix), 'bin/tmt-squad'),
      });
      for (const link of ['tmt-squad', 'tmt-sq'])
        expect(readlinkSync(path.join(prefix, 'bin', link))).toBe(
          '../lib/tmt-squad/current/tmt-squad'
        );
      expect(parseWholeStdout(await cli(install))).toMatchObject({ changed: false });

      // A foreign same-named command elsewhere on PATH is reported, never run.
      const foreign = path.join(sandbox.root, 'foreign bin');
      mkdirSync(foreign);
      writeFileSync(path.join(foreign, 'tmt-sq'), '#!/bin/sh\nexit 7\n');
      chmodSync(path.join(foreign, 'tmt-sq'), 0o755);
      // A controlled PATH: never the user's own installed commands.
      sandbox.env.PATH = [path.join(prefix, 'bin'), foreign, '/usr/bin', '/bin'].join(':');
      const listed = parseWholeStdout(await cli(['extension', 'list', '--prefix', prefix]));
      expect(listed).toEqual({
        extensions: [
          {
            name: 'office',
            installed: false,
            version: null,
            channel: null,
            pinned: null,
            commands: ['tmt-office'],
            shadowedBy: [],
          },
          {
            name: 'squad',
            installed: true,
            version: '0.1.0-alpha.1',
            channel: 'alpha',
            pinned: null,
            commands: ['tmt-squad', 'tmt-sq'],
            shadowedBy: [path.join(foreign, 'tmt-sq')],
          },
        ],
      });

      // Root help groups the two names that resolve to one file.
      const help = await runCli(sandbox, ['--help']);
      expect(help.stdout).toContain('Extension squad (also: sq): ');

      expectError(
        await cli(['extension', 'uninstall', 'squad', '--prefix', prefix]),
        'EXTENSION_CONSENT_REQUIRED'
      );
      const removed = await cli(['extension', 'uninstall', 'squad', '--yes', '--prefix', prefix]);
      expect(parseWholeStdout(removed)).toEqual({
        extension: 'squad',
        installed: false,
        changed: true,
        kept: ['releases', 'skills', 'hookConsent'],
      });
      for (const link of ['tmt-squad', 'tmt-sq'])
        expect(existsSync(path.join(prefix, 'bin', link))).toBe(false);
      expect(existsSync(path.join(prefix, 'lib/tmt-squad/releases'))).toBe(true);
      expectError(
        await cli(['extension', 'upgrade', 'squad', '--yes', '--prefix', prefix]),
        'EXTENSION_NOT_INSTALLED'
      );
    });
  }, 60_000);
});
