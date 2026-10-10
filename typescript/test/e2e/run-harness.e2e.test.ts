import {
  closeSync,
  constants,
  existsSync,
  mkdirSync,
  openSync,
  readFileSync,
  writeFileSync,
  writeSync,
} from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import Database from 'better-sqlite3';
import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';
import { expectJsonResult } from './cli-assertions.js';
import { withE2EFixture, type E2EFixture } from './harness.js';
import { durableState } from './identity-state-oracle.js';
import { waitForFileContent } from './wait-for-file.js';
import { processTreeHasOpenFile } from './process-file-oracle.js';

function quote(value: string): string {
  return `'${value.replaceAll("'", "'\\''")}'`;
}

function submit(
  fixture: E2EFixture,
  pane: string,
  args: string[],
  statusFile: string,
  env: Record<string, string> = {}
): void {
  const command = [
    ...(Object.keys(env).length === 0
      ? []
      : ['env', ...Object.entries(env).map(([key, value]) => `${key}=${value}`)]),
    fixture.executables.cli.executable,
    ...fixture.executables.cli.args,
    ...args,
  ]
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

async function suspendAfterLaunchStorageCloses(
  fixture: E2EFixture,
  pane: string,
  owner: number
): Promise<void> {
  // Running is committed before SQLite's final WAL close, which can take an
  // exclusive file lock. Suspending that close would freeze the next CLI's open.
  await fixture.waitFor(
    () => {
      if (!existsSync(`/proc/${owner}`)) throw new Error('Launcher exited before suspension.');
      return !processTreeHasOpenFile(owner, path.join(fixture.globalDir, 'tmux-team.db'));
    },
    5000,
    'launcher to close launch storage before suspension'
  );
  fixture.tmux(['send-keys', '-t', pane, 'C-z']);
}

function compileLaunchGate(fixture: E2EFixture): string {
  const library = path.join(fixture.root, 'sqlite-close-gate.so');
  execFileSync(
    'gcc',
    [
      '-shared',
      '-fPIC',
      '-Wall',
      '-Wextra',
      fileURLToPath(new URL('./fixtures/sqlite-close-gate.c', import.meta.url)),
      '-o',
      library,
      '-ldl',
    ],
    { timeout: 5000, encoding: 'utf8' }
  );
  return library;
}

function signalOwnedHarness(pid: number, executable: string, signal: NodeJS.Signals): void {
  try {
    const argv = readFileSync(`/proc/${pid}/cmdline`, 'utf8').split('\0');
    if (!argv.includes(executable))
      throw new Error(`Process ${pid} is no longer the owned harness.`);
    process.kill(pid, signal);
  } catch (error) {
    if (
      error instanceof Error &&
      'code' in error &&
      ['ENOENT', 'ESRCH'].includes(String(error.code))
    )
      return;
    throw error;
  }
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

function identityId(fixture: E2EFixture, name: string): string {
  const db = new Database(path.join(fixture.globalDir, 'tmux-team.db'), { readonly: true });
  try {
    return (db.prepare('SELECT id FROM identities WHERE name = ?').get(name) as { id: string }).id;
  } finally {
    db.close();
  }
}

// Assert the new provider-owned suffix before comparing the user's argv. This
// keeps the original byte-preservation checks sensitive to drops or additions.
function userArgsWithLaunchHooks(argv: string[]): string[] {
  expect(argv.at(-2)).toBe('--settings');
  const settings = JSON.parse(argv.at(-1)!);
  const digest = settings.hooks.Stop.flatMap((entry: { hooks: { command: string }[] }) =>
    entry.hooks.filter((hook) => hook.command.includes(' __digest-hook claude --launch '))
  );
  expect(digest).toHaveLength(1);
  return argv.slice(0, -2);
}

describe('foreground identity launch', { concurrent: false }, () => {
  it('auto-names a launch, names the same live identity, and resumes its hook-recorded session', async () => {
    await withE2EFixture(async (fixture) => {
      const pane = fixture.createShellPane('auto-run').pane;
      const scenario = path.join(fixture.root, 'auto-scenario.json');
      const report = path.join(fixture.root, 'auto-report.json');
      const checkpoint = path.join(fixture.root, 'before-name');
      const resumedScenario = path.join(fixture.root, 'resumed-scenario.json');
      const resumedReport = path.join(fixture.root, 'resumed-report.json');
      const resumedArgs = path.join(fixture.root, 'resumed-args');
      const session = '12345678-1234-4234-8234-123456789abc';
      const start = (source: string) => ({
        args: ['__hook', 'claude'],
        input: {
          hook_event_name: 'SessionStart',
          session_id: session,
          source,
        },
      });
      writeFileSync(
        scenario,
        JSON.stringify([
          start('startup'),
          { args: ['whoami', '--json'] },
          { checkpoint, args: ['this', 'Reviewer', '--json'] },
          { args: ['whoami', '--json'] },
        ])
      );
      writeFileSync(
        resumedScenario,
        JSON.stringify([start('resume'), { args: ['whoami', '--json'] }])
      );
      const fake = path.join(fixture.wrapperDir, 'claude');
      const script = `#!/bin/sh
if [ "$1" = "--resume" ]; then
  printf '%s\\0' "$@" > ${quote(resumedArgs)}
  exec /opt/tmt-tests/claude ${quote(fixture.executables.cli.executable)} ${quote(resumedScenario)} ${quote(resumedReport)}
fi
exec /opt/tmt-tests/claude "$@"
`;
      writeExecutable(fake, script, 0o700);
      const home = path.join(fixture.root, 'auto-home');
      mkdirSync(home);
      const status = path.join(fixture.root, 'auto.status');
      submit(
        fixture,
        pane,
        ['run', 'claude', fixture.executables.cli.executable, scenario, report],
        status,
        { HOME: home }
      );
      await waitForFileContent(checkpoint, { description: 'provider reached naming checkpoint' });
      const db = new Database(path.join(fixture.globalDir, 'tmux-team.db'), { readonly: true });
      try {
        const before = db.prepare('SELECT * FROM identities WHERE auto_named = 1').get() as {
          id: string;
          name: string;
          lifetime: string;
        };
        expect(before.name).toMatch(/^claude-[0-9a-f]{12}$/);
        expect(before.lifetime).toBe('temporary');
        const binding = db.prepare('SELECT id FROM bindings WHERE identity_id = ?').get(before.id);
        expect(preferences(fixture).find((row) => row.identity_id === before.id)).toMatchObject({
          preferred_harness: 'claude',
          provider_session_id: session,
        });
        writeFileSync(checkpoint, 'continue');
        expect(await waitForFileContent(status, { description: 'named launch exit' })).toBe('0');
        const results = JSON.parse(readFileSync(report, 'utf8')) as Array<{
          code: number;
          stdout: string;
        }>;
        expect(results.every((result) => result.code === 0)).toBe(true);
        expect(JSON.parse(results[1].stdout)).toMatchObject({ id: before.id, name: before.name });
        expect(JSON.parse(results[3].stdout)).toMatchObject({
          id: before.id,
          name: 'Reviewer',
          sessionState: 'running',
        });
        expect(
          db
            .prepare('SELECT id, name, lifetime, auto_named FROM identities WHERE id = ?')
            .get(before.id)
        ).toEqual({ id: before.id, name: 'Reviewer', lifetime: 'temporary', auto_named: 0 });
        expect(db.prepare('SELECT id FROM bindings WHERE identity_id = ?').get(before.id)).toEqual(
          binding
        );
        const resumeStatus = path.join(fixture.root, 'auto-resume.status');
        submit(fixture, pane, ['resume', 'Reviewer'], resumeStatus, { HOME: home });
        expect(
          await waitForFileContent(resumeStatus, { description: 'renamed identity resumed' })
        ).toBe('0');
        expect(
          userArgsWithLaunchHooks(readFileSync(resumedArgs, 'utf8').split('\0').slice(0, -1))
        ).toEqual(['--resume', session]);
        const resumed = JSON.parse(readFileSync(resumedReport, 'utf8')) as Array<{
          code: number;
          stdout: string;
        }>;
        expect(resumed.every((result) => result.code === 0)).toBe(true);
        expect(JSON.parse(resumed[1].stdout)).toMatchObject({ id: before.id, name: 'Reviewer' });
        expect(db.prepare('SELECT id FROM bindings WHERE identity_id = ?').get(before.id)).toEqual(
          binding
        );
      } finally {
        db.close();
      }
    });
  });

  it('resumes a verified bound seat through its recorded wrapper, observed model and bounded preset', async () => {
    await withE2EFixture(async (fixture) => {
      const pane = fixture.createShellPane('preset-seat').pane;
      const empty = fixture.createShellPane('unbound-resume').pane;
      const session = '12345678-1234-4234-8234-123456789abc';
      const scenario = path.join(fixture.root, 'preset-scenario.json');
      const report = path.join(fixture.root, 'preset-report.json');
      const calls = path.join(fixture.root, 'preset-calls.jsonl');
      writeFileSync(
        scenario,
        JSON.stringify([
          {
            args: ['__hook', 'claude'],
            input: {
              hook_event_name: 'SessionStart',
              session_id: session,
              source: 'startup',
              model: 'sonnet',
            },
          },
        ])
      );
      const record = `require('node:fs').appendFileSync(${JSON.stringify(calls)}, JSON.stringify({args:process.argv.slice(1), env:{pct:process.env.CLAUDE_AUTOCOMPACT_PCT_OVERRIDE,mini:process.env.CLAUDE_MINI_PCT}})+String.fromCharCode(10));`;
      const wrapper = path.join(fixture.wrapperDir, 'claude_mini');
      writeExecutable(
        wrapper,
        `#!/bin/sh\n${quote(process.execPath)} -e ${quote(record)} -- "$@" || exit "$?"\nexec /opt/tmt-tests/claude ${quote(fixture.executables.cli.executable)} ${quote(scenario)} ${quote(report)}\n`,
        0o700
      );
      const home = path.join(fixture.root, 'preset-home');
      mkdirSync(home);
      const run = async (label: string, args: string[], env: Record<string, string> = {}) => {
        const status = path.join(fixture.root, `${label}.status`);
        submit(fixture, pane, args, status, { HOME: home, ...env });
        expect(await waitForFileContent(status, { description: label })).toBe('0');
      };
      await run(
        'preset-initial',
        [
          'run',
          '--save',
          'Preset',
          wrapper,
          '--model',
          'opus',
          '--effort',
          'high',
          'private prompt',
        ],
        {
          CLAUDE_AUTOCOMPACT_PCT_OVERRIDE: '55',
          CLAUDE_MINI_PCT: '60',
          ANTHROPIC_API_KEY: 'fixture-secret',
        }
      );
      const id = identityId(fixture, 'Preset');
      const db = new Database(path.join(fixture.globalDir, 'tmux-team.db'), { readonly: true });
      try {
        const row = db
          .prepare(
            "SELECT value FROM identity_metadata WHERE identity_id = ? AND key = 'resume.launch'"
          )
          .get(id) as { value: string };
        expect(row.value).not.toContain('private prompt');
        expect(row.value).not.toContain('fixture-secret');
        expect(JSON.parse(row.value)).toMatchObject({
          executable: wrapper,
          harness: 'claude',
          session,
          model: 'sonnet',
          effort: 'high',
          env: { CLAUDE_AUTOCOMPACT_PCT_OVERRIDE: '55', CLAUDE_MINI_PCT: '60' },
        });
        await run('preset-unnamed', ['resume']);
        await run(
          'preset-explicit',
          ['resume', '--model', 'explicit-model', '--effort', 'low', 'Preset'],
          {
            CLAUDE_AUTOCOMPACT_PCT_OVERRIDE: '40',
            CLAUDE_MINI_PCT: '',
          }
        );
        const recorded = readFileSync(calls, 'utf8')
          .trim()
          .split('\n')
          .map(
            (line) =>
              JSON.parse(line) as {
                args: string[];
                env: { pct?: string; mini?: string };
              }
          );
        expect(recorded).toHaveLength(3);
        expect(userArgsWithLaunchHooks(recorded[1].args)).toEqual([
          '--resume',
          session,
          '--model',
          'sonnet',
          '--effort',
          'high',
        ]);
        expect(recorded[1].env).toEqual({ pct: '55', mini: '60' });
        expect(userArgsWithLaunchHooks(recorded[2].args)).toEqual([
          '--resume',
          session,
          '--model',
          'explicit-model',
          '--effort',
          'low',
        ]);
        expect(recorded[2].env).toEqual({ pct: '40', mini: '' });
        const unbound = await fixture.runCli(['resume'], { pane: empty });
        expect(unbound.code).toBe(1);
        expect(unbound.stderr).toContain('No identity is bound');
        expect(unbound.stderr).toContain('Preset');
        const shown = await fixture.runCli(['resume', '--show', 'Preset']);
        expect(shown.code).toBe(0);
        expect(JSON.parse(shown.stdout)).toMatchObject({
          identity: 'Preset',
          launch: { executable: wrapper },
        });
        const cleared = await fixture.runCli(['resume', '--forget-launch', 'Preset']);
        expect(cleared.code).toBe(0);
        expect(
          db
            .prepare(
              "SELECT value FROM identity_metadata WHERE identity_id = ? AND key = 'resume.launch'"
            )
            .get(id)
        ).toBeUndefined();
        expect(
          preferences(fixture).find((row) => row.identity_id === id)?.provider_session_id
        ).toBe(session);
        const shownCleared = await fixture.runCli(['resume', '--show', 'Preset']);
        expect(JSON.parse(shownCleared.stdout)).toEqual({ identity: 'Preset', launch: null });
        // Clearing preferences preserves exact-session resume via the named driver.
        const named = path.join(fixture.wrapperDir, 'claude');
        writeExecutable(
          named,
          `#!/bin/sh\n${quote(process.execPath)} -e ${quote(record)} -- "$@" || exit "$?"\nexec /opt/tmt-tests/claude ${quote(fixture.executables.cli.executable)} ${quote(scenario)} ${quote(report)}\n`,
          0o700
        );
        await run('preset-cleared-resume', ['resume', 'Preset']);
        const final = JSON.parse(readFileSync(calls, 'utf8').trim().split('\n').at(-1)!) as {
          args: string[];
          env: Record<string, string>;
        };
        expect(userArgsWithLaunchHooks(final.args)).toEqual([
          '--resume',
          session,
          '--model',
          'sonnet',
        ]);
        expect(final.env).toEqual({});
      } finally {
        db.close();
      }
    });
  });

  it('cleans up a failed temporary automatic spawn but retains an explicitly saved identity', async () => {
    await withE2EFixture(async (fixture) => {
      const pane = fixture.createShellPane('auto-spawn-failure').pane;
      const fake = path.join(fixture.wrapperDir, 'claude');
      writeExecutable(fake, '#!/nonexistent-tmt-fixture-interpreter\n', 0o700);
      const failed = await fixture.runCli(['run', 'claude'], { pane });
      expect(failed.code).toBe(1);
      expect(failed.stderr).toContain('Could not start');
      expect(
        failed.stderr.split('\n').filter((line) => line.startsWith('tmt: Temporary name:'))
      ).toHaveLength(1);
      const db = new Database(path.join(fixture.globalDir, 'tmux-team.db'), { readonly: true });
      try {
        expect(
          db.prepare('SELECT lifetime, retired_at_ms FROM identities WHERE auto_named = 1').all()
        ).toEqual([{ lifetime: 'temporary', retired_at_ms: expect.any(Number) }]);
        expect(db.prepare('SELECT * FROM bindings WHERE pane_id = ?').all(pane)).toEqual([]);
        const saved = await fixture.runCli(['run', '--save', 'claude'], { pane });
        expect(saved.code).toBe(1);
        const row = db
          .prepare(
            'SELECT id, name, lifetime FROM identities WHERE auto_named = 1 AND retired_at_ms IS NULL'
          )
          .get() as { id: string; name: string; lifetime: string };
        expect(row).toMatchObject({
          lifetime: 'saved',
          name: expect.stringMatching(/^claude-[0-9a-f]{12}$/),
        });
        const named = expectJsonResult(
          await fixture.runJsonCli<{ id: string; lifetime: string }>(['this', 'Saved'], { pane })
        );
        expect(named).toMatchObject({ id: row.id, lifetime: 'saved' });
      } finally {
        db.close();
      }
    });
  });

  it('refuses runtime-name collisions and accepts the explicit identity-command form', async () => {
    await withE2EFixture(async (fixture) => {
      const pane = fixture.createShellPane('auto-collision').pane;
      expectJsonResult(await fixture.runJsonCli(['identity', 'create', 'claude']));
      const fake = path.join(fixture.wrapperDir, 'claude');
      writeExecutable(fake, '#!/bin/sh\nexit 17\n', 0o700);
      for (const [label, args, code] of [
        ['bare', ['run', 'claude'], '5'],
        ['flags', ['run', 'claude', '--model', 'anything'], '5'],
        ['explicit', ['run', 'claude', 'claude'], '17'],
      ] as const) {
        const status = path.join(fixture.root, `${label}.status`);
        submit(fixture, pane, [...args], status);
        expect(await waitForFileContent(status, { description: label })).toBe(code);
      }
      const text = fixture.tmux(['capture-pane', '-p', '-t', pane]);
      expect(text).toContain('tmt run claude claude');
      expect(text).toContain('tmt run <new-name> claude');
    });
  });

  it('resumes only the exact remembered session, never starts fresh and marks only trustworthy failures stale', async () => {
    await withE2EFixture(async (fixture) => {
      const pane = fixture.createShellPane('run-resume').pane;
      const callsFile = path.join(fixture.root, 'resume-calls.jsonl');
      const fake = path.join(fixture.wrapperDir, 'claude');
      writeExecutable(
        fake,
        `#!${process.execPath}\nrequire('node:fs').appendFileSync(${JSON.stringify(callsFile)}, JSON.stringify(process.argv.slice(2)) + String.fromCharCode(10));\nprocess.exit(process.argv.includes('--resume') ? Number(process.env.RESUME_EXIT ?? 31) : 0);\n`,
        0o700
      );
      // A private HOME for the provider settings the stale check reads, so the
      // shared container home never gains hooks.
      const home = path.join(fixture.root, 'home');
      mkdirSync(path.join(home, '.claude'), { recursive: true });
      const run = async (label: string, args: string[], expected: number, exit = 31) => {
        const status = path.join(fixture.root, `${label}.status`);
        submit(fixture, pane, args, status, { HOME: home, RESUME_EXIT: String(exit) });
        expect(await waitForFileContent(status, { description: `${label} completed` })).toBe(
          String(expected)
        );
      };
      const calls = () =>
        readFileSync(callsFile, 'utf8')
          .trim()
          .split('\n')
          .map((line) => userArgsWithLaunchHooks(JSON.parse(line) as string[]));
      await run('initial', ['run', '-s', 'Resume', fake], 0);
      // Nothing remembered: both forms refuse without launching anything.
      await run('no-session', ['resume', 'Resume'], 1);
      await run('no-session-alias', ['run', '--resume', 'Resume'], 1);
      expect(calls()).toEqual([[]]);
      const id = identityId(fixture, 'Resume');
      const session = '12345678-1234-4234-8234-123456789abc';
      const write = (sql: string, ...values: unknown[]) => {
        const db = new Database(path.join(fixture.globalDir, 'tmux-team.db'));
        try {
          expect(db.prepare(sql).run(...values, id).changes).toBe(1);
        } finally {
          db.close();
        }
      };
      const seed = (harness: string, mode: string) =>
        write(
          'UPDATE identity_session_preferences SET remembered_harness = ?, runtime_mode = ?, provider_session_id = ?, stale_at_ms = NULL, resume_pending_at_ms = NULL WHERE identity_id = ?',
          harness,
          mode,
          session
        );
      const row = (overrides: Record<string, unknown> = {}) => ({
        identity_id: id,
        preferred_harness: 'claude',
        remembered_harness: 'claude',
        runtime_mode: 'default',
        provider_session_id: session,
        driver_state: null,
        driver_state_version: null,
        stale_at_ms: null,
        resume_pending_at_ms: null,
        channel: 0,
        ...overrides,
      });
      const staleAt = () => preferences(fixture)[0]?.stale_at_ms;

      // Read-only projections show the remembered session; nothing else is exposed.
      seed('claude', 'default');
      const resume = {
        driver: 'claude',
        mode: 'default',
        session,
        model: null,
        staleAtMs: null,
      };
      const shown = expectJsonResult(
        await fixture.runJsonCli<{ resume?: unknown }>(['identity', 'show', 'Resume'], {
          withoutTmux: true,
        })
      );
      expect(shown.resume).toEqual(resume);
      const listed = expectJsonResult(
        await fixture.runJsonCli<{ identities: Array<{ name: string; resume?: unknown }> }>(['ls'])
      );
      expect(listed.identities.find((row) => row.name === 'Resume')?.resume).toEqual(resume);

      // No persistent setup is needed now: per-launch start hooks make an
      // unconfirmed failed resume trustworthy enough to mark stale.
      await run('hooks-absent', ['resume', 'Resume'], 31);
      expect(calls()).toEqual([[], ['--resume', session]]);
      expect(staleAt()).toEqual(expect.any(Number));
      expect(preferences(fixture)).toEqual([row({ stale_at_ms: staleAt() })]);
      seed('claude', 'default');

      // With it installed, an unconfirmed non-zero exit marks the session stale;
      // a signal exit (128 + n) does not.
      writeFileSync(
        path.join(home, '.claude', 'settings.json'),
        JSON.stringify({
          hooks: Object.fromEntries(
            ['SessionStart', 'SessionEnd'].map((event) => [
              event,
              [
                {
                  hooks: [{ type: 'command', command: `'/opt/tmt/tmt' __hook claude`, timeout: 3 }],
                },
              ],
            ])
          ),
        })
      );
      await run('signal-exit', ['resume', 'Resume'], 130, 130);
      expect(preferences(fixture)).toEqual([row()]);
      await run('unconfirmed', ['resume', 'Resume'], 31);
      expect(staleAt()).toEqual(expect.any(Number));
      expect(preferences(fixture)).toEqual([row({ stale_at_ms: staleAt() })]);

      // A stale session is refused until the user retries; a retry that also
      // fails stays stale.
      await run('stale-refused', ['resume', 'Resume'], 1);
      expect(calls()).toHaveLength(4);
      await run('retry', ['resume', '--retry', 'Resume'], 31);
      expect(calls()).toHaveLength(5);
      expect(staleAt()).toEqual(expect.any(Number));

      // A crashed launcher's leftover pending mark is replaced by the next
      // resume, and a clean exit leaves nothing behind.
      seed('claude', 'default');
      write(
        'UPDATE identity_session_preferences SET resume_pending_at_ms = 1 WHERE identity_id = ?'
      );
      await run('after-crash', ['resume', 'Resume'], 0, 0);
      expect(preferences(fixture)).toEqual([row()]);

      // An unsupported session is reported, never replaced by a fresh start.
      seed('claude', 'unsupported-fixture-mode');
      await run('unsupported-mode', ['resume', 'Resume'], 1);
      // An unregistered driver's session is purged on the resume path.
      seed('unregistered-fixture-harness', 'default');
      await run('unregistered', ['run', '--resume', 'Resume'], 1);
      expect(calls()).toHaveLength(6);
      expect(preferences(fixture)).toEqual([
        row({
          remembered_harness: null,
          runtime_mode: null,
          provider_session_id: null,
          channel: null,
        }),
      ]);

      seed('claude', 'default');
      await run('forget', ['resume', '--forget', 'Resume'], 0);
      await run('forget-again', ['resume', '--forget', 'Resume'], 0);
      expect(preferences(fixture)).toEqual([
        row({
          remembered_harness: null,
          runtime_mode: null,
          provider_session_id: null,
          channel: null,
        }),
      ]);
      expect(calls()).toHaveLength(6);
    });
  });

  it('preserves stop/fg job control, refuses a second stopped runtime and records interrupt and forwarded termination', async () => {
    await withE2EFixture(async (fixture) => {
      const pane = fixture.createShellPane('run-signals').pane;
      const ready = path.join(fixture.root, 'signal-ready.json');
      const fake = path.join(fixture.wrapperDir, 'signal-harness');
      writeExecutable(
        fake,
        `#!${process.execPath}\nrequire('node:fs').writeFileSync(${JSON.stringify(ready)}, JSON.stringify({ child: process.pid, owner: process.ppid }));\nsetInterval(() => {}, 1000);\n`,
        0o700
      );
      const firstStatus = path.join(fixture.root, 'interrupt.status');
      submit(fixture, pane, ['run', '-s', 'Signals', fake], firstStatus);
      const first = JSON.parse(
        await waitForFileContent(ready, { description: 'signal harness start' })
      ) as { child: number; owner: number };
      const state = () =>
        durableState(fixture).bindings.find((row) => row.pane_id === pane)?.runtime_state;
      await fixture.waitFor(() => state() === 'running', 5000, 'durable running admission');
      const stopped = (pid: number) =>
        execFileSync('ps', ['-o', 'stat=', '-p', String(pid)], { encoding: 'utf8' })
          .trim()
          .startsWith('T');
      await suspendAfterLaunchStorageCloses(fixture, pane, first.owner);
      await fixture.waitFor(
        () => stopped(first.child) && stopped(first.owner),
        5000,
        'wrapper and child suspended together'
      );
      // Stopped processes do not prove Bash has reclaimed the terminal. Its
      // continuation writes this status only after returning from the stopped job.
      await waitForFileContent(firstStatus, {
        description: 'shell continuation after suspension',
      });
      const rejectedStatus = path.join(fixture.root, 'stopped-conflict.status');
      const forbidden = path.join(fixture.root, 'must-not-launch');
      submit(
        fixture,
        pane,
        ['run', 'Signals', '/bin/sh', '-c', `touch ${quote(forbidden)}`],
        rejectedStatus
      );
      const rejectedCode = await waitForFileContent(rejectedStatus, {
        description: 'stopped runtime conflict',
      });
      expect(rejectedCode, fixture.capture(60, pane)).toBe('5');
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
      expect(
        await waitForFileContent(interruptResult, { description: 'foreground interrupt status' })
      ).toBe('130');

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
      expect(
        await waitForFileContent(termStatus, { description: 'forwarded termination return' })
      ).toBe('143');
      expect(state()).toBe('ended');
      await fixture.waitFor(
        () => !existsSync(`/proc/${second.child}`) && !existsSync(`/proc/${second.owner}`),
        5000,
        'terminated direct child reaped'
      );
    });
  });

  it('defers immediate terminal suspension until launch storage releases its close lock', async () => {
    await withE2EFixture(async (fixture) => {
      const pane = fixture.createShellPane('run-close-lock').pane;
      const ready = path.join(fixture.root, 'close-ready.json');
      const fake = path.join(fixture.wrapperDir, 'close-harness');
      writeExecutable(
        fake,
        `#!${process.execPath}\nrequire('node:fs').writeFileSync(${JSON.stringify(ready)}, JSON.stringify({ child: process.pid, owner: process.ppid }));\nsetInterval(() => {}, 1000);\n`,
        0o700
      );
      const library = compileLaunchGate(fixture);
      const before = path.join(fixture.root, 'close-before');
      const held = path.join(fixture.root, 'close-held');
      const acquire = path.join(fixture.root, 'close-acquire.fifo');
      const release = path.join(fixture.root, 'close-release.fifo');
      execFileSync('mkfifo', [acquire, release], { timeout: 5000 });
      // Owned read/write descriptors let release tokens be queued even on a
      // failed readiness wait; cleanup cannot block opening an orphaned FIFO.
      const acquireFd = openSync(acquire, constants.O_RDWR | constants.O_NONBLOCK);
      const releaseFd = openSync(release, constants.O_RDWR | constants.O_NONBLOCK);
      const firstStatus = path.join(fixture.root, 'close-launch.status');
      let first: { child: number; owner: number } | undefined;
      let acquireReleased = false;
      let closeReleased = false;
      const state = () =>
        durableState(fixture).bindings.find((row) => row.pane_id === pane)?.runtime_state;
      const stopped = (pid: number) =>
        execFileSync('ps', ['-o', 'stat=', '-p', String(pid)], { encoding: 'utf8', timeout: 5000 })
          .trim()
          .startsWith('T');
      try {
        submit(fixture, pane, ['run', '-s', 'CloseLock', fake], firstStatus, {
          LD_PRELOAD: library,
          TMT_CLOSE_DATABASE: path.join(fixture.globalDir, 'tmux-team.db'),
          TMT_CLOSE_BEFORE: before,
          TMT_CLOSE_HELD: held,
          TMT_CLOSE_ACQUIRE: acquire,
          TMT_CLOSE_RELEASE: release,
        });
        first = JSON.parse(await waitForFileContent(ready)) as { child: number; owner: number };
        await waitForFileContent(before, { description: 'launcher entering SQLite close' });
        expect(state()).toBe('running');
        expect(
          processTreeHasOpenFile(first.owner, path.join(fixture.globalDir, 'tmux-team.db'))
        ).toBe(true);
        writeSync(acquireFd, Buffer.from('go'));
        acquireReleased = true;
        await waitForFileContent(held, { description: 'launcher holding SQLite close lock' });
        const locked = new Database(path.join(fixture.globalDir, 'tmux-team.db'), { timeout: 0 });
        try {
          expect(() => locked.prepare('SELECT * FROM bindings').all()).toThrow(/locked/);
        } finally {
          locked.close();
        }
        const signalBit = (pid: number, field: string, signal: number) => {
          const value = readFileSync(`/proc/${pid}/status`, 'utf8').match(
            new RegExp(`^${field}:\\s*([0-9a-f]+)$`, 'm')
          )?.[1];
          if (!value) throw new Error(`Missing ${field} for owned process ${pid}.`);
          return (BigInt(`0x${value}`) & (1n << BigInt(signal - 1))) !== 0n;
        };
        // Linux SIGTSTP is 20. Only the launcher blocks it, with no ignored or
        // caught disposition; the already-spawned provider retains defaults.
        expect(signalBit(first.child, 'SigBlk', 20)).toBe(false);
        for (const pid of [first.owner, first.child]) {
          expect(signalBit(pid, 'SigIgn', 20)).toBe(false);
          expect(signalBit(pid, 'SigCgt', 20)).toBe(false);
        }
        fixture.tmux(['send-keys', '-t', pane, 'C-z']);
        await fixture.waitFor(
          () => stopped(first!.child),
          5000,
          'provider receives immediate Ctrl-Z'
        );
        // Child stop proves terminal delivery happened. The launcher remains
        // runnable until its real close finishes; old unmasked code stops here.
        expect(stopped(first.owner)).toBe(false);
        expect(signalBit(first.owner, 'SigBlk', 20)).toBe(true);
        expect(signalBit(first.owner, 'ShdPnd', 20) || signalBit(first.owner, 'SigPnd', 20)).toBe(
          true
        );
        expect(
          processTreeHasOpenFile(first.owner, path.join(fixture.globalDir, 'tmux-team.db'))
        ).toBe(true);
        writeSync(releaseFd, Buffer.from('go'));
        closeReleased = true;
        await fixture.waitFor(
          () => stopped(first!.owner) && stopped(first!.child),
          5000,
          'closed-storage launcher and child suspended'
        );
        expect(
          processTreeHasOpenFile(first.owner, path.join(fixture.globalDir, 'tmux-team.db'))
        ).toBe(false);
        expect(signalBit(first.owner, 'SigBlk', 20)).toBe(false);
        expect(signalBit(first.owner, 'SigIgn', 20)).toBe(false);
        expect(signalBit(first.owner, 'SigCgt', 20)).toBe(false);
        await waitForFileContent(firstStatus, {
          description: 'shell continuation after suspension',
        });
        const conflict = path.join(fixture.root, 'close-conflict.status');
        const forbidden = path.join(fixture.root, 'close-must-not-launch');
        submit(
          fixture,
          pane,
          ['run', 'CloseLock', '/bin/sh', '-c', `touch ${quote(forbidden)}`],
          conflict
        );
        expect(
          await waitForFileContent(conflict, { description: 'post-close runtime conflict' })
        ).toBe('5');
        expect(existsSync(forbidden)).toBe(false);
        expect(stopped(first.child)).toBe(true);
      } finally {
        try {
          if (!acquireReleased) writeSync(acquireFd, Buffer.from('go'));
          if (!closeReleased) writeSync(releaseFd, Buffer.from('go'));
          if (first) {
            signalOwnedHarness(first.owner, fake, 'SIGCONT');
            signalOwnedHarness(first.child, fake, 'SIGCONT');
            // The close release can deliver the pending stop after our first
            // SIGCONT. Wait for descriptors to close, then resume once more.
            await fixture.waitFor(
              () =>
                !processTreeHasOpenFile(first!.owner, path.join(fixture.globalDir, 'tmux-team.db')),
              5000,
              'launch close gate released for cleanup'
            );
            signalOwnedHarness(first.owner, fake, 'SIGCONT');
            signalOwnedHarness(first.child, fake, 'SIGCONT');
            signalOwnedHarness(first.owner, fake, 'SIGTERM');
            await fixture.waitFor(
              () =>
                state() === 'ended' &&
                !existsSync(`/proc/${first!.owner}`) &&
                !existsSync(`/proc/${first!.child}`),
              5000,
              'close-lock runtime ended and both processes reaped'
            );
            const cleanupStatus = path.join(fixture.root, 'close-cleanup.status');
            fixture.tmux([
              'send-keys',
              '-t',
              pane,
              '-l',
              `printf '%s' "$?" > ${quote(cleanupStatus)}`,
            ]);
            fixture.tmux(['send-keys', '-t', pane, 'Enter']);
            expect(
              await waitForFileContent(cleanupStatus, { description: 'post-exit shell status' })
            ).toMatch(/^\d+$/);
          }
        } finally {
          closeSync(acquireFd);
          closeSync(releaseFd);
        }
      }
    });
  });

  it('retains a child stopped before admission as Unknown and refuses a duplicate launch', async () => {
    await withE2EFixture(async (fixture) => {
      const pane = fixture.createShellPane('run-pre-admission-stop').pane;
      const ready = path.join(fixture.root, 'stopped-ready.json');
      const fake = path.join(fixture.wrapperDir, 'stopped-before-admission');
      writeExecutable(
        fake,
        `#!${process.execPath}\nrequire('node:fs').writeFileSync(${JSON.stringify(ready)}, JSON.stringify({ child: process.pid, owner: process.ppid }));\nprocess.kill(process.pid, 'SIGSTOP');\nprocess.exit(23);\n`,
        0o700
      );
      const library = compileLaunchGate(fixture);
      const admission = path.join(fixture.root, 'admission-ready');
      const release = path.join(fixture.root, 'admission-release.fifo');
      execFileSync('mkfifo', [release], { timeout: 5000 });
      const releaseFd = openSync(release, constants.O_RDWR | constants.O_NONBLOCK);
      const status = path.join(fixture.root, 'stopped-launch.status');
      let first: { child: number; owner: number } | undefined;
      let released = false;
      const row = () => durableState(fixture).bindings.find((binding) => binding.pane_id === pane);
      try {
        submit(fixture, pane, ['run', '-s', 'StoppedBeforeAdmission', fake], status, {
          LD_PRELOAD: library,
          TMT_ADMISSION_READY: admission,
          TMT_ADMISSION_RELEASE: release,
        });
        first = JSON.parse(await waitForFileContent(ready)) as { child: number; owner: number };
        await waitForFileContent(admission, {
          description: 'real owned-child ps exec held before admission',
        });
        await fixture.waitFor(
          () => readFileSync(`/proc/${first!.child}/status`, 'utf8').includes('State:\tT'),
          5000,
          'provider stopped before native admission probe'
        );
        writeSync(releaseFd, Buffer.from('go'));
        released = true;
        await fixture.waitFor(
          () => !processTreeHasOpenFile(first!.owner, path.join(fixture.globalDir, 'tmux-team.db')),
          5000,
          'stopped-child attachment committed and storage closed'
        );
        expect(row()).toMatchObject({
          runtime_state: 'unknown',
          runtime_pid: first.child,
          launch_owner_pid: first.owner,
          last_transition: 'started',
          observed_provider_session_id: null,
        });
        expect(row()?.runtime_start_identity).toMatch(/^ps-v1:/);
        expect(row()?.launch_owner_start_identity).toMatch(/^ps-v1:/);
        // Stop the foreground launcher now that its descriptors are closed so
        // the shell can submit a second real command in this same pane.
        fixture.tmux(['send-keys', '-t', pane, 'C-z']);
        await waitForFileContent(status, {
          description: 'shell continuation for pre-admission stopped launch',
        });
        const forbidden = path.join(fixture.root, 'stopped-must-not-launch');
        const conflict = path.join(fixture.root, 'stopped-conflict.status');
        submit(
          fixture,
          pane,
          ['run', 'StoppedBeforeAdmission', '/bin/sh', '-c', `touch ${quote(forbidden)}`],
          conflict
        );
        expect(await waitForFileContent(conflict)).toBe('5');
        expect(existsSync(forbidden)).toBe(false);
        expect(row()?.runtime_state).toBe('unknown');
      } finally {
        try {
          if (!released) writeSync(releaseFd, Buffer.from('go'));
          if (first) {
            signalOwnedHarness(first.owner, fake, 'SIGCONT');
            signalOwnedHarness(first.child, fake, 'SIGCONT');
            await fixture.waitFor(
              () =>
                row()?.runtime_state === 'ended' &&
                !existsSync(`/proc/${first!.owner}`) &&
                !existsSync(`/proc/${first!.child}`),
              5000,
              'stopped launch exited and exact owner and child reaped'
            );
          }
        } finally {
          closeSync(releaseFd);
        }
      }
    });
  });

  it('preserves argv and TTY, remembers only the claim, and records fast exit before returning to the shell', async () => {
    await withE2EFixture(async (fixture) => {
      const shell = fixture.createShellPane('run-fast');
      const log = path.join(fixture.root, 'harness.jsonl');
      const fake = path.join(fixture.wrapperDir, 'claude');
      // A deterministic executable named claude exercises recognition, not a
      // provider installation. The container never invokes a real model.
      writeExecutable(
        fake,
        `#!${process.execPath}
const fs = require('node:fs');
const descriptors = fs.readdirSync('/proc/self/fd').flatMap(fd => { try { return [fs.readlinkSync('/proc/self/fd/' + fd)]; } catch { return []; } });
fs.appendFileSync(${JSON.stringify(log)}, JSON.stringify({ pid: process.pid, owner: process.ppid, argv: process.argv.slice(2), tty: [!!process.stdin.isTTY, !!process.stdout.isTTY, !!process.stderr.isTTY], descriptors }) + String.fromCharCode(10));
process.exit(23);
`,
        0o700
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
      expect(
        await waitForFileContent(firstStatus, { description: 'foreground command exit' })
      ).toBe('23');
      const first = JSON.parse(readFileSync(log, 'utf8').trim()) as {
        pid: number;
        owner: number;
        argv: string[];
        tty: boolean[];
        descriptors: string[];
      };
      expect(userArgsWithLaunchHooks(first.argv)).toEqual(args);
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
          driver_state: null,
          driver_state_version: null,
          stale_at_ms: null,
          resume_pending_at_ms: null,
          channel: 0,
        },
      ]);

      const secondStatus = path.join(fixture.root, 'second.status');
      submit(fixture, shell.pane, ['run', '-s', 'Runner'], secondStatus);
      expect(
        await waitForFileContent(secondStatus, { description: 'remembered bare relaunch exit' })
      ).toBe('23');
      const calls = readFileSync(log, 'utf8')
        .trim()
        .split('\n')
        .map((line) => JSON.parse(line) as { argv: string[] });
      expect(calls).toHaveLength(2);
      expect(userArgsWithLaunchHooks(calls[1]!.argv)).toEqual([]);
      expect(
        durableState(fixture).identities.find((row) => row.id === binding?.identity_id)?.lifetime
      ).toBe('saved');

      const genericStatus = path.join(fixture.root, 'generic.status');
      submit(fixture, shell.pane, ['run', 'Runner', '/bin/sh', '-c', 'exit 7'], genericStatus);
      expect(await waitForFileContent(genericStatus, { description: 'generic fast exit' })).toBe(
        '7'
      );
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
