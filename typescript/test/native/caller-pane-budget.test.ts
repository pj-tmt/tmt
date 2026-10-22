import { chmodSync, existsSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it, vi } from 'vite-plus/test';
import { runCli, withSandbox, type Sandbox } from '../support/cli-process.js';
import { locateTmux, privateTmux } from '../support/private-tmux.js';

// #4260: the check that the caller's own pane exists is one tmux command. It had a one second
// budget for every caller, and a timeout was folded into "no pane": `tmt name` answered
// PANE_NOT_FOUND after about 1.06 s whenever tmux (or whatever stands in front of it) was slow.
// The four commands that cannot do without the pane now wait up to 5 s for that one command and
// report a timeout as a timeout. Every other caller keeps its one second.
//
// The private server lives in the sandbox; a PATH `tmux` forwards to it and delays only the first
// `display-message -t`, which is the caller check. No host tmux is addressed.
async function withDelayedCallerCheck(
  run: (world: {
    sandbox: Sandbox;
    pane: string;
    delay: (seconds: number) => void;
  }) => Promise<void>
) {
  await withSandbox(async (sandbox) => {
    const real = locateTmux();
    const scratch = privateTmux({
      root: sandbox.root,
      name: 'budget',
      env: sandbox.env,
      executable: real,
    });
    scratch.tmux(['new-session', '-d', '-s', 'budget', '-x', '80', '-y', '24', '/bin/sh']);
    const server = Number(scratch.tmux(['display-message', '-p', '#{pid}']));
    const session = scratch.tmux(['display-message', '-p', '#{session_id}']).replace(/^\$/, '');
    const pane = scratch.tmux(['display-message', '-p', '#{pane_id}']);
    const wrap = path.join(sandbox.root, 'wrap');
    mkdirSync(wrap);
    const delayFile = path.join(wrap, 'delay');
    const doneFile = path.join(wrap, 'done');
    writeFileSync(delayFile, '0');
    writeFileSync(
      path.join(wrap, 'tmux'),
      [
        '#!/bin/sh',
        `if [ ! -e '${doneFile}' ]; then case " $* " in *" display-message "*" -t "*) : > '${doneFile}'; sleep "$(cat '${delayFile}')" ;; esac; fi`,
        'while [ "$#" -gt 0 ]; do case "$1" in -u|-2) shift ;; -S) shift 2 ;; *) break ;; esac; done',
        `exec '${real}' -f /dev/null -S '${scratch.socket}' -u "$@"`,
        '',
      ].join('\n')
    );
    chmodSync(path.join(wrap, 'tmux'), 0o700);
    sandbox.env.PATH = [wrap, sandbox.env.PATH].join(path.delimiter);
    sandbox.env.TMUX = `${scratch.socket},${server},${session}`;
    sandbox.env.TMUX_PANE = pane;
    try {
      await run({
        sandbox,
        pane,
        delay: (seconds) => {
          writeFileSync(delayFile, String(seconds));
          rmSync(doneFile, { force: true });
        },
      });
    } finally {
      scratch.stop();
      await vi.waitFor(() => expect(() => process.kill(server, 0)).toThrow(), {
        timeout: 10_000,
        interval: 25,
      });
      expect(existsSync(scratch.socket) ? 'socket file may remain' : 'gone').toBeTruthy();
    }
  });
}

const slow = { deadlineMs: 30_000 };

describe('the caller pane check of the commands that need a pane', () => {
  it('waits for a slow tmux instead of claiming there is no pane', async () => {
    await withDelayedCallerCheck(async ({ sandbox, pane, delay }) => {
      delay(1.5);
      const named = await runCli(sandbox, ['name', 'worker', '--save', '--json'], slow);
      expect(named.status, named.stdout + named.stderr).toBe(0);
      expect(JSON.parse(named.stdout)).toMatchObject({ bound: true, name: 'worker', pane });

      for (const command of [['whoami'], ['ls', '--here'], ['unbind']]) {
        delay(1.5);
        const result = await runCli(sandbox, [...command, '--json'], slow);
        expect(result.status, `${command.join(' ')}: ${result.stdout}${result.stderr}`).toBe(0);
      }
    });
  }, 60_000);

  it('reports a tmux that outlasts the budget as slow, not as a missing pane', async () => {
    await withDelayedCallerCheck(async ({ sandbox, delay }) => {
      delay(6.5);
      const named = await runCli(sandbox, ['name', 'worker', '--save', '--json'], slow);
      expect(named.status, named.stdout + named.stderr).toBe(1);
      const error = JSON.parse(named.stdout).error;
      expect(error.code).toBe('RECONCILIATION_FAILED');
      expect(error.message).toContain('did not answer within 5 s');
      expect(error.message).toContain('ETIMEDOUT');
      expect(named.stdout).not.toContain('PANE_NOT_FOUND');
    });
  }, 60_000);
});
