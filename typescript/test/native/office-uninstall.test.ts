import { existsSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import { parseWholeStdout, runCli, withSandbox } from '../support/cli-process.js';
import { createArtifact } from '../support/native-artifact.js';
import { INSTALL_BUDGET_MS, paths, setUpMachine, tree } from '../support/native-uninstall.js';

describe('tmt uninstall on a disposable HOME and prefix', () => {
  it('stops a running Office service before removing its files', async () => {
    await withSandbox(async (sandbox) => {
      const tmt = await setUpMachine(sandbox);
      const { prefix } = paths(sandbox);
      const office = (args: string[]) =>
        runCli(tmt, ['office', '--prefix', prefix, ...args, '--json'], {
          deadlineMs: INSTALL_BUDGET_MS,
        });
      const artifact = await createArtifact(sandbox, '0.1.0-alpha.4', new Uint8Array(), 'office');
      try {
        const installedOffice = await runCli(
          tmt,
          [
            '__native-install',
            '--product',
            'office',
            '--channel',
            'alpha',
            '--prefix',
            prefix,
            '--json',
            '--archive',
            artifact.archive,
            '--manifest',
            artifact.manifest,
          ],
          { deadlineMs: INSTALL_BUDGET_MS }
        );
        expect(installedOffice.status, installedOffice.stdout + installedOffice.stderr).toBe(0);
        const started = await office(['start']);
        expect(started.status, started.stdout + started.stderr).toBe(0);
        const receipt = path.join(sandbox.globalDir, 'office', 'runtime', 'service-v1.json');
        expect(existsSync(receipt)).toBe(true);

        const result = await runCli(tmt, ['uninstall', '--yes', '--json'], {
          deadlineMs: INSTALL_BUDGET_MS,
        });
        expect(result.status, result.stdout + result.stderr).toBe(0);
        expect(parseWholeStdout(result).officeStopped).toBe(true);
        expect(existsSync(receipt)).toBe(false);
        expect(tree(prefix)).toEqual({});
      } finally {
        // The in-process stop path needs no installed companion.
        await runCli(sandbox, ['office', 'stop', '--json']);
      }
    });
  });
});
