import { existsSync } from 'node:fs';
import { describe, expect, it } from 'vite-plus/test';
import { runCli, withSandbox } from '../support/cli-process.js';

describe('focus outside a tmux client', () => {
  it('refuses with HOST_UNSUPPORTED before touching storage or any tmux server', async () => {
    await withSandbox(async (sandbox) => {
      // The process harness always clears TMUX and TMUX_PANE; malformed and
      // pane-less tmux contexts are covered where a real server exists (E2E).
      for (const args of [
        ['focus', 'auth-fix', '--json'],
        ['focus', '%2', '--json'],
        ['focus', '--client', '--json'],
      ]) {
        const result = await runCli(sandbox, args);
        expect(result.status, args.join(' ')).toBe(1);
        expect(JSON.parse(result.stdout).error.code).toBe('HOST_UNSUPPORTED');
      }
      expect(existsSync(sandbox.database)).toBe(false);
    });
  });

  it('requires exactly one identity or pane target, or --client alone', async () => {
    await withSandbox(async (sandbox) => {
      for (const args of [
        ['focus', '--json'],
        ['focus', 'auth-fix', '--client', '--json'],
      ]) {
        const result = await runCli(sandbox, args);
        expect(result.status, args.join(' ')).not.toBe(0);
        expect(JSON.parse(result.stdout).error.code, args.join(' ')).toBe('USAGE_ERROR');
      }
    });
  });
});
