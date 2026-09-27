import Database from 'better-sqlite3';
import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { withE2EFixture, type E2EFixture } from './harness.js';
import { durableState } from './identity-state-oracle.js';

function quote(value: string): string {
  return `'${value.replaceAll("'", "'\\''")}'`;
}

function submit(fixture: E2EFixture, pane: string, args: string[], statusFile: string): void {
  const command = [fixture.executables.cli.executable, ...fixture.executables.cli.args, ...args]
    .map(quote)
    .join(' ');
  fixture.tmux([
    'send-keys',
    '-t',
    pane,
    '-l',
    `${command}; printf '%s' "$?" > ${quote(statusFile)}`,
  ]);
  fixture.tmux(['send-keys', '-t', pane, 'Enter']);
}

function preferences(fixture: E2EFixture): Record<string, unknown>[] {
  const db = new Database(path.join(fixture.globalDir, 'tmux-team.db'), { readonly: true });
  try {
    return db.prepare('SELECT * FROM identity_session_preferences').all() as Record<
      string,
      unknown
    >[];
  } finally {
    db.close();
  }
}

describe.sequential('foreground identity launch', () => {
  it('resumes only the exact remembered session and never launches a fallback after a failed resume', async () => {
    await withE2EFixture(async (fixture) => {
      const pane = fixture.createShellPane('run-resume').pane;
      const callsFile = path.join(fixture.root, 'resume-calls.jsonl');
      const fake = path.join(fixture.wrapperDir, 'claude');
      writeFileSync(
        fake,
        `#!${process.execPath}\nrequire('node:fs').appendFileSync(${JSON.stringify(callsFile)}, JSON.stringify(process.argv.slice(2)) + String.fromCharCode(10));\nprocess.exit(process.argv.includes('--resume') ? 31 : 0);\n`,
        { mode: 0o700 }
      );
      const run = async (label: string, args: string[], expected: number) => {
        const status = path.join(fixture.root, `${label}.status`);
        submit(fixture, pane, args, status);
        await fixture.waitFor(() => existsSync(status), 5000, `${label} completed`);
        expect(readFileSync(status, 'utf8')).toBe(String(expected));
      };
      await run('initial', ['run', '-s', 'Resume', fake], 0);
      await run('no-session', ['run', '--resume', 'Resume'], 0);
      const id = durableState(fixture).bindings.find((row) => row.pane_id === pane)?.identity_id;
      const session = '12345678-1234-4234-8234-123456789abc';
      const seed = (harness: string, mode: string) => {
        const db = new Database(path.join(fixture.globalDir, 'tmux-team.db'));
        try {
          expect(
            db
              .prepare(
                'UPDATE identity_session_preferences SET remembered_harness = ?, runtime_mode = ?, provider_session_id = ? WHERE identity_id = ?'
              )
              .run(harness, mode, session, id).changes
          ).toBe(1);
        } finally {
          db.close();
        }
      };
      seed('claude', 'default');
      await run('failed-exact-resume', ['run', '--resume', 'Resume'], 31);
      const calls = () =>
        readFileSync(callsFile, 'utf8')
          .trim()
          .split('\n')
          .map((line) => JSON.parse(line) as string[]);
      expect(calls()).toEqual([[], [], ['--resume', session]]);
      expect(preferences(fixture)[0]).toMatchObject({
        remembered_harness: 'claude',
        runtime_mode: 'default',
        provider_session_id: session,
      });
      seed('claude', 'unsupported-fixture-mode');
      await run('unsupported-mode', ['run', '--resume', 'Resume'], 0);
      seed('unregistered-fixture-harness', 'default');
      await run('unsupported-harness', ['run', '--resume', 'Resume'], 0);
      expect(calls()).toEqual([[], [], ['--resume', session], [], []]);
      expect(preferences(fixture)[0]).toMatchObject({
        preferred_harness: 'claude',
        remembered_harness: 'unregistered-fixture-harness',
        provider_session_id: session,
      });
    });
  });

  it('preserves stop/fg job control, refuses a second stopped runtime and records interrupt and forwarded termination', async () => {
    await withE2EFixture(async (fixture) => {
      const pane = fixture.createShellPane('run-signals').pane;
      const ready = path.join(fixture.root, 'signal-ready.json');
      const fake = path.join(fixture.wrapperDir, 'signal-harness');
      writeFileSync(
        fake,
        `#!${process.execPath}\nrequire('node:fs').writeFileSync(${JSON.stringify(ready)}, JSON.stringify({ child: process.pid, owner: process.ppid }));\nsetInterval(() => {}, 1000);\n`,
        { mode: 0o700 }
      );
      const firstStatus = path.join(fixture.root, 'interrupt.status');
      submit(fixture, pane, ['run', '-s', 'Signals', fake], firstStatus);
      await fixture.waitFor(() => existsSync(ready), 5000, 'signal harness start');
      const first = JSON.parse(readFileSync(ready, 'utf8')) as { child: number; owner: number };
      const state = () =>
        durableState(fixture).bindings.find((row) => row.pane_id === pane)?.runtime_state;
      await fixture.waitFor(() => state() === 'running', 5000, 'durable running admission');
      const stopped = (pid: number) =>
        execFileSync('ps', ['-o', 'stat=', '-p', String(pid)], { encoding: 'utf8' })
          .trim()
          .startsWith('T');
      fixture.tmux(['send-keys', '-t', pane, 'C-z']);
      await fixture.waitFor(
        () => stopped(first.child) && stopped(first.owner),
        5000,
        'wrapper and child suspended together'
      );
      const rejectedStatus = path.join(fixture.root, 'stopped-conflict.status');
      const forbidden = path.join(fixture.root, 'must-not-launch');
      submit(
        fixture,
        pane,
        ['run', 'Signals', '/bin/sh', '-c', `touch ${quote(forbidden)}`],
        rejectedStatus
      );
      await fixture.waitFor(() => existsSync(rejectedStatus), 5000, 'stopped runtime conflict');
      expect(readFileSync(rejectedStatus, 'utf8')).toBe('5');
      expect(existsSync(forbidden)).toBe(false);
      expect(stopped(first.child)).toBe(true);
      fixture.tmux(['send-keys', '-t', pane, '-l', 'fg']);
      fixture.tmux(['send-keys', '-t', pane, 'Enter']);
      await fixture.waitFor(
        () => !stopped(first.child) && !stopped(first.owner),
        5000,
        'foreground resume'
      );
      fixture.tmux(['send-keys', '-t', pane, 'C-c']);
      await fixture.waitFor(
        () =>
          state() === 'ended' &&
          !existsSync(`/proc/${first.child}`) &&
          !existsSync(`/proc/${first.owner}`),
        5000,
        'interrupted child and wrapper reaped'
      );
      // The original shell continuation ran when the job was suspended; fg's
      // status is read separately after the foreground job actually finishes.
      const interruptResult = path.join(fixture.root, 'fg-interrupt.status');
      fixture.tmux(['send-keys', '-t', pane, '-l', `printf '%s' "$?" > ${quote(interruptResult)}`]);
      fixture.tmux(['send-keys', '-t', pane, 'Enter']);
      await fixture.waitFor(() => existsSync(interruptResult), 5000, 'foreground interrupt status');
      expect(readFileSync(interruptResult, 'utf8')).toBe('130');

      writeFileSync(ready, '');
      const termStatus = path.join(fixture.root, 'term.status');
      submit(fixture, pane, ['run', 'Signals', fake], termStatus);
      await fixture.waitFor(
        () => readFileSync(ready, 'utf8').length > 0 && state() === 'running',
        5000,
        'second live admission'
      );
      const second = JSON.parse(readFileSync(ready, 'utf8')) as { child: number; owner: number };
      process.kill(second.owner, 'SIGTERM');
      await fixture.waitFor(() => existsSync(termStatus), 5000, 'forwarded termination return');
      expect(readFileSync(termStatus, 'utf8')).toBe('143');
      expect(state()).toBe('ended');
      await fixture.waitFor(
        () => !existsSync(`/proc/${second.child}`) && !existsSync(`/proc/${second.owner}`),
        5000,
        'terminated direct child reaped'
      );
    });
  });

  it('preserves argv and TTY, remembers only the claim, and records fast exit before returning to the shell', async () => {
    await withE2EFixture(async (fixture) => {
      const shell = fixture.createShellPane('run-fast');
      const log = path.join(fixture.root, 'harness.jsonl');
      const fake = path.join(fixture.wrapperDir, 'claude');
      // A deterministic executable named claude exercises recognition, not a
      // provider installation. The container never invokes a real model.
      writeFileSync(
        fake,
        `#!${process.execPath}
const fs = require('node:fs');
const descriptors = fs.readdirSync('/proc/self/fd').flatMap(fd => { try { return [fs.readlinkSync('/proc/self/fd/' + fd)]; } catch { return []; } });
fs.appendFileSync(${JSON.stringify(log)}, JSON.stringify({ pid: process.pid, owner: process.ppid, argv: process.argv.slice(2), tty: [!!process.stdin.isTTY, !!process.stdout.isTTY, !!process.stderr.isTTY], descriptors }) + String.fromCharCode(10));
process.exit(23);
`,
        { mode: 0o700 }
      );
      const args = [
        '--resume',
        'exact-user-value',
        '--help',
        '--json',
        '',
        'two words\nnext line',
        '--secret=not-stored',
      ];
      const firstStatus = path.join(fixture.root, 'first.status');
      submit(fixture, shell.pane, ['run', 'Runner', fake, ...args], firstStatus);
      await fixture.waitFor(() => existsSync(firstStatus), 5000, 'foreground command exit');
      expect(readFileSync(firstStatus, 'utf8')).toBe('23');
      const first = JSON.parse(readFileSync(log, 'utf8').trim()) as {
        pid: number;
        owner: number;
        argv: string[];
        tty: boolean[];
        descriptors: string[];
      };
      expect(first.argv).toEqual(args);
      expect(first.tty).toEqual([true, true, true]);
      expect(first.descriptors.some((fd) => fd.includes('tmux-team.db'))).toBe(false);
      const binding = durableState(fixture).bindings.find((row) => row.pane_id === shell.pane);
      expect(binding).toMatchObject({
        runtime_state: 'ended',
        last_transition: 'ended',
        runtime_pid: first.pid,
        launch_owner_pid: first.owner,
        observed_provider_session_id: null,
      });
      expect(binding?.runtime_start_identity).toMatch(/^ps-v1:/);
      expect(binding?.launch_owner_start_identity).toMatch(/^ps-v1:/);
      expect(
        durableState(fixture).identities.find((row) => row.id === binding?.identity_id)?.lifetime
      ).toBe('temporary');
      expect(preferences(fixture)).toEqual([
        {
          identity_id: binding?.identity_id,
          preferred_harness: 'claude',
          remembered_harness: null,
          runtime_mode: null,
          provider_session_id: null,
        },
      ]);

      const secondStatus = path.join(fixture.root, 'second.status');
      submit(fixture, shell.pane, ['run', '-s', 'Runner'], secondStatus);
      await fixture.waitFor(() => existsSync(secondStatus), 5000, 'remembered bare relaunch exit');
      expect(readFileSync(secondStatus, 'utf8')).toBe('23');
      const calls = readFileSync(log, 'utf8')
        .trim()
        .split('\n')
        .map((line) => JSON.parse(line) as { argv: string[] });
      expect(calls).toHaveLength(2);
      expect(calls[1]?.argv).toEqual([]);
      expect(
        durableState(fixture).identities.find((row) => row.id === binding?.identity_id)?.lifetime
      ).toBe('saved');

      const genericStatus = path.join(fixture.root, 'generic.status');
      submit(fixture, shell.pane, ['run', 'Runner', '/bin/sh', '-c', 'exit 7'], genericStatus);
      await fixture.waitFor(() => existsSync(genericStatus), 5000, 'generic fast exit');
      expect(readFileSync(genericStatus, 'utf8')).toBe('7');
      expect(preferences(fixture)[0]?.preferred_harness).toBe('claude');
      expect(
        durableState(fixture).identities.find((row) => row.id === binding?.identity_id)?.lifetime
      ).toBe('saved');
      expect(
        durableState(fixture).bindings.find((row) => row.pane_id === shell.pane)?.runtime_state
      ).toBe('ended');
    });
  });
});
