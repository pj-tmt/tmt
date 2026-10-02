import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { E2EFixture } from '../e2e/harness.js';
import { installTmuxTrace } from '../e2e/tmux-trace.js';

describe('fixture tmux tracing guards', () => {
  it('refuses a second installation without replacing either wrapper', () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'tmt-tmux-trace-'));
    try {
      const wrapperDir = path.join(root, 'bin');
      fs.mkdirSync(wrapperDir);
      const wrapper = path.join(wrapperDir, 'tmux');
      const inner = path.join(wrapperDir, 'tmux-inner');
      fs.writeFileSync(wrapper, '#!/bin/sh\nexit 0\n', { mode: 0o755 });
      const fixture = { root, wrapperDir } as E2EFixture;
      const trace = installTmuxTrace(fixture);
      execFileSync(wrapper, ['display-message', '-p', 'literal value'], {
        timeout: 5_000,
        killSignal: 'SIGKILL',
      });
      expect(trace.commands()).toEqual(['display-message']);
      const wrappers = [fs.readFileSync(wrapper), fs.readFileSync(inner)];
      expect(() => installTmuxTrace(fixture)).toThrow('already installed');
      expect([fs.readFileSync(wrapper), fs.readFileSync(inner)]).toEqual(wrappers);
      expect(trace.commands()).toEqual(['display-message']);
      trace.clear();
      expect(trace.invocations()).toEqual([]);
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  });

  it('kills a nonresponsive synchronous fixture command so the scenario timer can fire', () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'tmt-tmux-trace-'));
    try {
      const pidFile = path.join(root, 'pid');
      fs.writeFileSync(
        path.join(root, 'tmux'),
        `#!${process.execPath}\nrequire('node:fs').writeFileSync(${JSON.stringify(pidFile)}, String(process.pid));\nsetTimeout(() => process.exit(42), 15000);\n`,
        { mode: 0o755 }
      );
      // Exercise the fixture method without discovering or starting host tmux.
      const fixture = Object.assign(Object.create(E2EFixture.prototype), {
        wrapperDir: root,
        env: {},
      }) as E2EFixture;
      let failure: unknown;
      try {
        fixture.tmux(['display-message']);
      } catch (error) {
        failure = error;
      }
      expect(failure).toMatchObject({ code: 'ETIMEDOUT', signal: 'SIGKILL' });
      const pid = Number(fs.readFileSync(pidFile, 'utf8'));
      expect(pid).toBeGreaterThan(0);
      expect(() => process.kill(pid, 0)).toThrow(expect.objectContaining({ code: 'ESRCH' }));
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  }, 20_000);
});
