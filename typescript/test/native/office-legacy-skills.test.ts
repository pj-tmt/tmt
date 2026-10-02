import { lstatSync, readFileSync, unlinkSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { parseWholeStdout, runCli, withSandbox, type Sandbox } from '../support/cli-process.js';
import { createArtifact } from '../support/native-artifact.js';
import { oldLayout } from '../support/legacy-extension-skills.js';

async function installOffice(sandbox: Sandbox, prefix: string) {
  const artifact = await createArtifact(sandbox, '0.1.0-alpha.4', new Uint8Array(), 'office');
  const args = [
    'extension',
    'install',
    'office',
    '--yes',
    '--prefix',
    prefix,
    '--archive',
    artifact.archive,
    '--manifest',
    artifact.manifest,
    '--json',
  ];
  const result = await runCli(sandbox, args, { deadlineMs: 15_000 });
  expect(result.status, result.stdout + result.stderr).toBe(0);
  return args;
}

describe('legacy extension skill lifecycle (#957)', () => {
  it('finishes half-removal, reports it in ls, and retires all twelve old links', async () => {
    await withSandbox(async (sandbox) => {
      const targets = oldLayout(sandbox);
      const prefix = path.join(sandbox.root, 'prefix');
      await installOffice(sandbox, prefix);
      unlinkSync(path.join(prefix, 'bin/tmt-office'));
      const listed = await runCli(sandbox, ['extension', 'ls', '--prefix', prefix, '--json']);
      expect(listed.status, listed.stdout).toBe(0);
      expect(parseWholeStdout(listed)).toMatchObject({
        extensions: [
          {
            name: 'office',
            installed: false,
            status: 'partiallyRemoved',
            hint: expect.stringContaining('extension rm office --yes'),
          },
          {},
        ],
      });
      const removed = await runCli(sandbox, [
        'extension',
        'rm',
        'office',
        '--yes',
        '--prefix',
        prefix,
        '--json',
      ]);
      expect(removed.status, removed.stdout).toBe(0);
      expect(parseWholeStdout(removed)).toMatchObject({
        skillsRemoved: expect.arrayContaining(targets),
        skillsKept: [],
      });
      for (const target of targets) expect(() => lstatSync(target)).toThrow();
      expect(
        JSON.parse(readFileSync(path.join(sandbox.globalDir, 'skill-installations.json'), 'utf8'))
          .targets
      ).toEqual([]);
      expect((await runCli(sandbox, ['__native-refresh-skills', '--json'])).status).toBe(0);
    });
  }, 60_000);
});
