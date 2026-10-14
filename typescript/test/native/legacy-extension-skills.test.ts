import {
  existsSync,
  mkdirSync,
  readFileSync,
  readlinkSync,
  symlinkSync,
  unlinkSync,
  writeFileSync,
} from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import { parseWholeStdout, runCli, withSandbox } from '../support/cli-process.js';
import { oldLayout } from '../support/legacy-extension-skills.js';

describe('retired extension guidance preservation', () => {
  it.each([false, true])(
    'preserves historical links on install, refresh and default uninstall (dangling=%s)',
    async (dangling) => {
      await withSandbox(async (sandbox) => {
        const targets = oldLayout(sandbox, dangling, !dangling);
        const prefix = path.join(sandbox.root, 'prefix');
        const retiredProgram = path.join(prefix, 'bin', 'tmt-office');
        const retiredData = path.join(sandbox.globalDir, 'office', 'office.db');
        const retiredNamespace = path.join(prefix, 'lib', 'tmt-office');
        const retiredRelease = path.join(retiredNamespace, 'releases', '0.1.0-alpha.1');
        mkdirSync(path.dirname(retiredProgram), { recursive: true });
        mkdirSync(retiredRelease, { recursive: true });
        mkdirSync(path.dirname(retiredData), { recursive: true });
        writeFileSync(path.join(retiredRelease, 'tmt-office'), 'opaque installed Office program');
        writeFileSync(path.join(retiredRelease, 'receipt.json'), 'opaque retained receipt');
        symlinkSync('releases/0.1.0-alpha.1', path.join(retiredNamespace, 'current'));
        symlinkSync('../lib/tmt-office/current/tmt-office', retiredProgram);
        writeFileSync(retiredData, 'opaque retained Office data');
        const prior = targets.map((target) => ({
          target,
          source: readlinkSync(target),
          bytes: dangling ? undefined : readFileSync(path.join(target, 'SKILL.md')),
        }));
        const installed = await runCli(sandbox, ['install', '--json']);
        expect(installed.status, installed.stdout + installed.stderr).toBe(0);
        expect(
          new Set(
            (parseWholeStdout(installed).installed as { skill: string }[]).map(
              (entry) => entry.skill
            )
          )
        ).toEqual(new Set(['tmt', 'tmt-inbox']));
        const refreshed = await runCli(sandbox, ['__native-refresh-skills', '--json']);
        expect(refreshed.status, refreshed.stdout + refreshed.stderr).toBe(0);
        expect(parseWholeStdout(refreshed)).toMatchObject({
          conflicts: [],
          skipped: expect.arrayContaining(targets),
        });
        if (!dangling) {
          for (const root of new Set(targets.map((target) => path.dirname(target)))) {
            expect(existsSync(path.join(root, 'tmux-team'))).toBe(false);
            expect(readFileSync(path.join(root, 'tmt', 'SKILL.md'), 'utf8')).not.toContain(
              'name: tmux-team'
            );
          }
        }
        const removed = await runCli(sandbox, ['uninstall', '--yes', '--prefix', prefix, '--json']);
        expect(removed.status, removed.stdout + removed.stderr).toBe(0);
        expect(readlinkSync(retiredProgram)).toBe('../lib/tmt-office/current/tmt-office');
        expect(readlinkSync(path.join(retiredNamespace, 'current'))).toBe('releases/0.1.0-alpha.1');
        expect(readFileSync(retiredProgram, 'utf8')).toBe('opaque installed Office program');
        expect(readFileSync(path.join(retiredRelease, 'receipt.json'), 'utf8')).toBe(
          'opaque retained receipt'
        );
        expect(readFileSync(retiredData, 'utf8')).toBe('opaque retained Office data');
        expect(parseWholeStdout(removed)).toMatchObject({
          data: { deleted: false },
          kept: expect.arrayContaining(
            targets.map((target) => ({ path: target, reason: expect.any(String) }))
          ),
        });
        for (const { target, source, bytes } of prior) {
          expect(readlinkSync(target)).toBe(source);
          if (bytes) expect(readFileSync(path.join(target, 'SKILL.md')).equals(bytes)).toBe(true);
        }
      });
    }
  );

  it('preserves edited historical sources, unmanaged targets and missing custom integrations', async () => {
    await withSandbox(async (sandbox) => {
      const targets = oldLayout(sandbox);
      const edited = readlinkSync(targets[0]);
      writeFileSync(path.join(edited, 'SKILL.md'), 'retained edited source');
      unlinkSync(targets[1]);
      mkdirSync(targets[1]);
      writeFileSync(path.join(targets[1], 'SKILL.md'), 'user owned');
      const outside = path.join(sandbox.root, 'outside');
      mkdirSync(outside);
      writeFileSync(path.join(outside, 'SKILL.md'), 'outside owned');
      unlinkSync(targets[2]);
      symlinkSync(outside, targets[2]);
      const missing = path.join(sandbox.root, 'custom', 'tmt-office');
      const record = path.join(sandbox.globalDir, 'skill-installations.json');
      writeFileSync(record, JSON.stringify({ version: 1, targets: [...targets, missing] }));
      for (const args of [
        ['install', '--json'],
        ['__native-refresh-skills', '--json'],
        ['uninstall', '--yes', '--prefix', path.join(sandbox.root, 'prefix'), '--json'],
      ]) {
        const result = await runCli(sandbox, args);
        expect(result.status, result.stdout + result.stderr).toBe(0);
      }
      expect(readFileSync(path.join(edited, 'SKILL.md'), 'utf8')).toBe('retained edited source');
      expect(readFileSync(path.join(targets[1], 'SKILL.md'), 'utf8')).toBe('user owned');
      expect(readlinkSync(targets[2])).toBe(outside);
      expect(readFileSync(path.join(outside, 'SKILL.md'), 'utf8')).toBe('outside owned');
      expect(JSON.parse(readFileSync(record, 'utf8')).targets).toContain(missing);
    });
  });
});
