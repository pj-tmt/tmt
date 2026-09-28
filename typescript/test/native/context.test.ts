import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { fileSnapshot, runCli, withSandbox } from '../support/cli-process.js';
import { calibrateTmuxTripwire } from './tmux-tripwire.js';

describe('read-only identity context process contract', () => {
  it('returns unavailable context without creating configuration, database or notes', async () => {
    await withSandbox(async (sandbox) => {
      const tripwire = await calibrateTmuxTripwire(sandbox);
      const snapshot = () => {
        const files = fileSnapshot(sandbox.root);
        delete files[path.relative(sandbox.root, tripwire)];
        return files;
      };
      const before = snapshot();
      const result = await runCli(sandbox, ['whoami', '--context', '--json']);
      expect(result.status, result.stderr).toBe(0);
      expect(result.stderr).toBe('');
      expect(JSON.parse(result.stdout)).toEqual({
        bound: false,
        status: 'unavailable',
      });
      expect(Buffer.byteLength(result.stdout)).toBeLessThanOrEqual(4096);
      expect(snapshot()).toEqual(before);
      expect(existsSync(sandbox.globalDir)).toBe(false);
      const human = await runCli(sandbox, ['whoami', '--context']);
      expect(human.status, human.stderr).toBe(0);
      expect(human.stdout).toBe('');
      expect(human.stderr).toBe('');
      expect(snapshot()).toEqual(before);
      expect(existsSync(sandbox.globalDir)).toBe(false);
    });
  });

  it('does not repair or initialize malformed existing configuration during hook discovery', async () => {
    await withSandbox(async (sandbox) => {
      await calibrateTmuxTripwire(sandbox);
      mkdirSync(sandbox.globalDir, { recursive: true });
      writeFileSync(sandbox.globalConfig, '{invalid configuration');
      const result = await runCli(sandbox, ['whoami', '--context', '--json']);
      expect(result.status, result.stderr).toBe(0);
      expect(result.stderr).toBe('');
      expect(JSON.parse(result.stdout).bound).toBe(false);
      expect(readFileSync(sandbox.globalConfig, 'utf8')).toBe('{invalid configuration');
      expect(existsSync(sandbox.database)).toBe(false);
      expect(existsSync(path.join(sandbox.globalDir, 'notes'))).toBe(false);
    });
  });
});
