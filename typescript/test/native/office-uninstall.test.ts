import { existsSync, readFileSync } from 'node:fs';
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
        const stopped = await runCli(sandbox, ['office', 'stop', '--json']);
        expect(stopped.status, stopped.stdout + stopped.stderr).toBe(0);
        expect(parseWholeStdout(stopped).running).toBe(false);
      }
    });
  });

  it('rejects and terminates a detached Office service left by the callback', async () => {
    let root = '';
    let receipt: { pid: number; port: number; controlToken: string; nonce: string } | undefined;
    const alive = () => {
      if (receipt === undefined) return false;
      try {
        process.kill(receipt.pid, 0);
        return true;
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code === 'ESRCH') return false;
        throw error;
      }
    };
    try {
      await expect(
        withSandbox(async (sandbox) => {
          root = sandbox.root;
          const prefix = path.join(root, 'office prefix');
          const artifact = await createArtifact(
            sandbox,
            '0.1.0-alpha.4',
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
          expect(installed.status, installed.stdout + installed.stderr).toBe(0);
          const started = await runCli(sandbox, ['office', '--prefix', prefix, 'start', '--json']);
          expect(started.status, started.stdout + started.stderr).toBe(0);
          receipt = JSON.parse(
            readFileSync(path.join(sandbox.globalDir, 'office/runtime/service-v1.json'), 'utf8')
          ) as { pid: number; port: number; controlToken: string; nonce: string };
          // Returning without stop recreates the callback cleanup gap.
          expect(alive()).toBe(true);
        })
      ).rejects.toThrow('Sandbox callback left live processes');
      expect(existsSync(root)).toBe(false);
      await expect.poll(alive, { timeout: 1000 }).toBe(false);
    } finally {
      // Keep the regression safe when run against the original, unguarded harness.
      if (receipt !== undefined && alive()) {
        const stopped = await fetch(`http://127.0.0.1:${receipt.port}/control/v1/stop`, {
          method: 'POST',
          headers: {
            authorization: `Bearer ${receipt.controlToken}`,
            'x-tmt-office-nonce': receipt.nonce,
          },
          signal: AbortSignal.timeout(1000),
        });
        expect(stopped.status).toBe(200);
        await stopped.arrayBuffer();
        await expect.poll(alive, { timeout: 1000 }).toBe(false);
      }
    }
  });
});
