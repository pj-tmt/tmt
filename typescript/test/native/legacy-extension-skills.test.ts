import {
  existsSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  readlinkSync,
  readdirSync,
  symlinkSync,
  unlinkSync,
  writeFileSync,
} from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import { expectError, parseWholeStdout, runCli, withSandbox } from '../support/cli-process.js';

import { oldLayout, officeNames } from '../support/legacy-extension-skills.js';

describe('legacy extension skill lifecycle (#957)', () => {
  it('removes legacy published links even after the extension commands are already gone', async () => {
    await withSandbox(async (sandbox) => {
      const targets = oldLayout(sandbox);
      const removed = await runCli(sandbox, ['extension', 'rm', 'office', '--yes', '--json']);
      expect(removed.status, removed.stdout).toBe(0);
      expect(parseWholeStdout(removed)).toMatchObject({
        changed: true,
        skillsRemoved: expect.arrayContaining(targets),
      });
      for (const target of targets) expect(() => lstatSync(target)).toThrow();
    });
  });

  it('adopts dangling legacy links through the existing extension skill owner API', async () => {
    await withSandbox(async (sandbox) => {
      const targets = oldLayout(sandbox, true);
      const adopted = await runCli(sandbox, ['api'], {
        stdin: JSON.stringify({
          version: 1,
          operation: 'skills.install',
          input: {
            owner: 'office',
            consent: true,
            skills: officeNames.map((name) => ({
              name,
              files: [{ path: 'SKILL.md', content: `---\nname: ${name}\n---\nOwner fixture.\n` }],
            })),
          },
        }),
      });
      expect(adopted.status, adopted.stdout).toBe(0);
      for (const target of targets) {
        expect(readlinkSync(target)).toContain('/skill-assets/owners/office/');
        expect(readFileSync(path.join(target, 'SKILL.md'), 'utf8')).toContain('Owner fixture.');
      }
      expect((await runCli(sandbox, ['__native-refresh-skills', '--json'])).status).toBe(0);
    });
  });

  it('removes old-layout Office links when both receipt and intent record are missing', async () => {
    await withSandbox(async (sandbox) => {
      const targets = oldLayout(sandbox, true);
      unlinkSync(path.join(sandbox.globalDir, 'skill-installations.json'));
      const removed = await runCli(sandbox, ['extension', 'rm', 'office', '--yes', '--json']);
      expect(removed.status, removed.stdout).toBe(0);
      expect(parseWholeStdout(removed)).toMatchObject({
        changed: true,
        skillsRemoved: expect.arrayContaining(targets),
      });
      for (const target of targets) expect(() => lstatSync(target)).toThrow();
    });
  });

  it('refreshes all twelve dangling TMT links without conflict or manual deletion', async () => {
    await withSandbox(async (sandbox) => {
      const targets = oldLayout(sandbox, true);
      const refreshed = await runCli(sandbox, ['__native-refresh-skills', '--json']);
      expect(refreshed.status, refreshed.stdout).toBe(0);
      expect(parseWholeStdout(refreshed)).toMatchObject({ conflicts: [], skipped: [] });
      for (const target of targets) {
        expect(readFileSync(path.join(target, 'SKILL.md'), 'utf8')).toContain(
          `name: ${path.basename(target)}`
        );
      }
    });
  });

  it('preserves user directories, outside links and modified sources on removal', async () => {
    await withSandbox(async (sandbox) => {
      const targets = oldLayout(sandbox);
      const directory = targets[0];
      unlinkSync(directory);
      mkdirSync(directory);
      writeFileSync(path.join(directory, 'SKILL.md'), 'user owned');
      const outside = path.join(sandbox.root, 'outside');
      mkdirSync(outside);
      writeFileSync(path.join(outside, 'SKILL.md'), 'outside owned');
      unlinkSync(targets[1]);
      symlinkSync(outside, targets[1]);
      const modified = readlinkSync(targets[2]);
      writeFileSync(path.join(modified, 'SKILL.md'), 'modified source');
      const removed = await runCli(sandbox, ['extension', 'rm', 'office', '--yes', '--json']);
      expect(removed.status, removed.stdout).toBe(0);
      expect(readFileSync(path.join(directory, 'SKILL.md'), 'utf8')).toBe('user owned');
      expect(readlinkSync(targets[1])).toBe(outside);
      expect(readFileSync(path.join(modified, 'SKILL.md'), 'utf8')).toBe('modified source');
      expect(readlinkSync(targets[2])).toBe(modified);
    });
  });

  it('reports the precise backup command for a remaining user conflict', async () => {
    await withSandbox(async (sandbox) => {
      const targets = oldLayout(sandbox, true);
      const target = targets[0];
      unlinkSync(target);
      mkdirSync(target);
      writeFileSync(path.join(target, 'SKILL.md'), 'user owned');
      const conflict = await runCli(sandbox, ['__native-refresh-skills']);
      expect(conflict.status).toBe(1);
      expect(conflict.stdout).toContain('backup=$(mktemp -d');
      expect(conflict.stdout).toContain('User-owned or modified skill preserved');
      expect(conflict.stdout).toContain(target);
      expect(readFileSync(path.join(target, 'SKILL.md'), 'utf8')).toBe('user owned');
      const line = conflict.stdout.split('\n').find((line) => line.includes(target));
      expect(line).toBeDefined();
      const command = line!.split('back up this entry with: ')[1].split('; then repeat')[0];
      const recovered = await runCli(
        { ...sandbox, cli: { executable: '/bin/sh', args: ['-c', command] } },
        []
      );
      expect(recovered.status, recovered.stderr).toBe(0);
      expect(existsSync(target)).toBe(false);
      const backupRoot = path.join(path.dirname(path.dirname(target)), '.tmt-skill-backups');
      const [backup] = readdirSync(backupRoot);
      expect(
        readFileSync(path.join(backupRoot, backup, path.basename(target), 'SKILL.md'), 'utf8')
      ).toBe('user owned');
      expect((await runCli(sandbox, ['__native-refresh-skills', '--json'])).status).toBe(0);
    });
  });

  it('retains a cause when upgrade fails before networking against old-layout state', async () => {
    await withSandbox(async (sandbox) => {
      oldLayout(sandbox);
      const failed = await runCli(sandbox, ['upgrade', '--json']);
      expectError(failed, 'NATIVE_UPGRADE_FAILED');
      expect(parseWholeStdout(failed)).toMatchObject({
        error: { cause: expect.stringContaining('managed') },
      });
    });
  });
});
