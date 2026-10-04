import { writeExecutable } from '../support/executable-fixture.mjs';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import Database from 'better-sqlite3';
import { expect, it } from 'vite-plus/test';
import { withE2EFixture } from './harness.js';
import type { E2EFixture } from './harness.js';

const session = '33333333-3333-4333-8333-333333333333';
const foreign = '44444444-4444-4444-8444-444444444444';
const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;
// Codex documents `model` (the active model slug) on every hook input.
const hook = (source: string, id = session, model = 'gpt-5.2-codex') => ({
  args: ['__hook', 'codex'],
  input: { hook_event_name: 'SessionStart', source, session_id: id, model },
});

async function waitForRuntimeExit(
  fixture: E2EFixture,
  runtimePid: number,
  status: string,
  timeoutMs = 5000
): Promise<void> {
  await fixture.waitFor(
    () => {
      try {
        process.kill(runtimePid, 0);
        return false;
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== 'ESRCH') throw error;
      }
      return fs.existsSync(status) && fs.readFileSync(status, 'utf8') !== '';
    },
    timeoutMs,
    'runtime fixture reaped by its pane shell'
  );
  expect(fs.readFileSync(status, 'utf8')).toBe('0');
}

it('maps independent Codex then shared exact-thread hooks without using the server pane', async () => {
  await withE2EFixture(
    async (fixture) => {
      const home = path.join(fixture.root, 'home');
      fs.mkdirSync(home);
      expect((await fixture.runJsonCli(['name', 'Owner', '-s'])).code).toBe(0);
      expect(
        (await fixture.runJsonCli(['config', 'set', 'ui.paneBadge', 'on', '--global'])).code
      ).toBe(0);
      const scenario = path.join(fixture.root, 'codex-independent.json');
      const report = path.join(fixture.root, 'codex-independent-report.json');
      const independentStatus = path.join(fixture.root, 'codex-independent.status');
      fs.writeFileSync(
        scenario,
        JSON.stringify([
          { args: ['name', 'Codex Reader', '-s', '--json'] },
          hook('startup'),
          hook('compact'),
          { args: ['whoami', '--json'] },
          {
            args: ['__hook', 'codex'],
            input: { hook_event_name: 'SessionEnd', reason: 'other', session_id: session },
          },
        ])
      );
      const command = [
        'env',
        `HOME=${home}`,
        `TMUX_TEAM_HOME=${fixture.globalDir}`,
        '/opt/tmt-tests/hook-runtime/codex',
        fixture.executables.cli.executable,
        scenario,
        report,
      ]
        .map(quote)
        .join(' ');
      // Retain a real shell for the later exact shared resume and client exit.
      const targetPane = fixture.createShellPane('codex-target').pane;
      fixture.tmux([
        'send-keys',
        '-t',
        targetPane,
        '-l',
        `${command}; printf '%s' "$?" > ${quote(independentStatus)}`,
      ]);
      fixture.tmux(['send-keys', '-t', targetPane, 'Enter']);
      await fixture.waitFor(() => fs.existsSync(report), 15000, 'independent Codex hook report');
      const results = JSON.parse(fs.readFileSync(report, 'utf8'));
      expect(
        results.every(
          (item: { code: number; stderr: string }) => item.code === 0 && item.stderr === ''
        )
      ).toBe(true);
      const identity = JSON.parse(results[0].stdout);
      const badge = () =>
        fixture
          .tmux(['-u', 'show-options', '-p', '-qv', '-t', targetPane, '@tmux-team.badge'])
          .trim();
      expect(results[0].badge).toBe('Codex Reader (tmt)');
      expect(results[1].badge).toBe(
        '#[push-default]#[fg=green]●#[default]#[pop-default] Codex Reader (tmt)'
      );
      expect(results[4].badge).toBe('Codex Reader (tmt)');
      for (const index of [1, 2])
        expect(JSON.parse(results[index].stdout).hookSpecificOutput.additionalContext).toContain(
          identity.id
        );
      expect(JSON.parse(results[3].stdout)).toMatchObject({
        id: identity.id,
        sessionState: 'running',
      });
      const db = new Database(path.join(fixture.globalDir, 'tmux-team.db'), { readonly: true });
      try {
        const read = () =>
          db
            .prepare(
              'SELECT b.pane_id, b.observed_provider_session_id, b.runtime_state, p.runtime_mode FROM bindings b JOIN identity_session_preferences p ON b.identity_id = p.identity_id WHERE b.identity_id = ?'
            )
            .get(identity.id) as {
            pane_id: string;
            observed_provider_session_id: string;
            runtime_state: string;
            runtime_mode: string;
          };
        const driverState = () =>
          db
            .prepare(
              'SELECT driver_state_version, driver_state FROM identity_session_preferences WHERE identity_id = ?'
            )
            .get(identity.id) as { driver_state_version: number; driver_state: string };
        expect(driverState()).toEqual({
          driver_state_version: 1,
          driver_state: '{"model":"gpt-5.2-codex"}',
        });
        const before = read();
        const runtimePid = () =>
          (
            db
              .prepare('SELECT runtime_pid FROM bindings WHERE identity_id = ?')
              .get(identity.id) as {
              runtime_pid: number;
            }
          ).runtime_pid;
        expect(before).toMatchObject({
          runtime_state: 'unknown',
          runtime_mode: 'embedded',
          observed_provider_session_id: session,
        });
        // The old process must be gone, not merely done writing its report.
        await waitForRuntimeExit(fixture, runtimePid(), independentStatus);
        const sharedScenario = path.join(fixture.root, 'codex-shared.json');
        const sharedReport = path.join(fixture.root, 'codex-shared-report.json');
        const sharedStatus = path.join(fixture.root, 'codex-shared.status');
        fs.writeFileSync(
          sharedScenario,
          JSON.stringify([
            hook('resume', foreign, 'gpt-5.3-codex'),
            hook('resume', session, 'gpt-5.3-codex'),
            hook('compact', session, 'gpt-5.3-codex'),
          ])
        );
        // A retained shell owns wait/reaping. A direct tmux child can remain a
        // zombie after reporting; kill(pid, 0) still succeeds until tmux reaps it.
        const serverPane = fixture.createShellPane('codex-server').pane;
        const sharedCommand = [
          'env',
          `HOME=${home}`,
          `TMUX_TEAM_HOME=${fixture.globalDir}`,
          '/opt/tmt-tests/hook-runtime/codex',
          'app-server',
          fixture.executables.cli.executable,
          sharedScenario,
          sharedReport,
        ]
          .map(quote)
          .join(' ');
        fixture.tmux([
          'send-keys',
          '-t',
          serverPane,
          '-l',
          `${sharedCommand}; printf '%s' "$?" > ${quote(sharedStatus)}`,
        ]);
        fixture.tmux(['send-keys', '-t', serverPane, 'Enter']);
        await fixture.waitFor(() => fs.existsSync(sharedReport), 15000, 'shared Codex hook report');
        const shared = JSON.parse(fs.readFileSync(sharedReport, 'utf8'));
        expect(shared[0]).toEqual({ code: 0, stdout: '', stderr: '', badge: '' });
        for (const index of [1, 2]) {
          expect(shared[index].code).toBe(0);
          expect(shared[index].stderr).toBe('');
          expect(JSON.parse(shared[index].stdout).hookSpecificOutput.additionalContext).toContain(
            identity.id
          );
        }
        expect(read()).toEqual({ ...before, runtime_state: 'running', runtime_mode: 'shared' });
        // A reported model replaces the stored one.
        expect(driverState()).toEqual({
          driver_state_version: 1,
          driver_state: '{"model":"gpt-5.3-codex"}',
        });
        expect(badge()).toBe(
          '#[push-default]#[fg=green]●#[default]#[pop-default] Codex Reader (tmt)'
        );
        await waitForRuntimeExit(fixture, runtimePid(), sharedStatus);

        const resumed = path.join(fixture.root, 'resumed-client.json');
        const release = path.join(fixture.root, 'release-client');
        const status = path.join(fixture.root, 'client-exit.status');
        writeExecutable(
          path.join(fixture.wrapperDir, 'codex'),
          `#!${process.execPath}\nconst fs = require('node:fs');\nfs.writeFileSync(${JSON.stringify(resumed)}, JSON.stringify(process.argv.slice(2)));\nsetInterval(() => { if (fs.existsSync(${JSON.stringify(release)})) process.exit(0); }, 20);\n`,
          0o700
        );
        const resumeCommand = [
          'env',
          `HOME=${home}`,
          fixture.executables.cli.executable,
          ...fixture.executables.cli.args,
          'run',
          '--resume',
          // This hook-only fixture exercises plain exact resume, not app-server attachment.
          '--no-channel',
          'Codex Reader',
        ]
          .map(quote)
          .join(' ');
        fixture.tmux([
          'send-keys',
          '-t',
          targetPane,
          '-l',
          `${resumeCommand}; printf '%s' "$?" > ${quote(status)}`,
        ]);
        fixture.tmux(['send-keys', '-t', targetPane, 'Enter']);
        await fixture.waitFor(
          () => fs.existsSync(resumed) && read().runtime_state === 'running',
          5000,
          'owned exact shared resume admitted'
        );
        // Resume replays the exact session with the model its hooks reported.
        expect(JSON.parse(fs.readFileSync(resumed, 'utf8'))).toEqual([
          'resume',
          '-m',
          'gpt-5.3-codex',
          session,
        ]);
        expect(read()).toEqual({ ...before, runtime_state: 'running', runtime_mode: 'shared' });
        fs.writeFileSync(release, 'exit');
        await fixture.waitFor(
          () => fs.existsSync(status) && fs.readFileSync(status, 'utf8') !== '',
          5000,
          'shared client reaped'
        );
        expect(fs.readFileSync(status, 'utf8')).toBe('0');
        expect(read()).toEqual({ ...before, runtime_state: 'unknown', runtime_mode: 'shared' });
        expect(badge()).toBe('Codex Reader (tmt)');
      } finally {
        db.close();
      }
    },
    { mode: 'input-log' }
  );
});

it.each([
  { driver: 'codex', executable: '/opt/tmt-tests/hook-runtime/codex', reason: 'other' },
  { driver: 'claude', executable: '/opt/tmt-tests/claude', reason: 'logout' },
  { driver: 'claude', executable: '/opt/tmt-tests/claude', reason: 'prompt_input_exit' },
  { driver: 'claude', executable: '/opt/tmt-tests/claude', reason: 'other' },
])(
  '$driver $reason session end permits same-process turnover',
  async ({ driver, executable, reason }) => {
    await withE2EFixture(
      async (fixture) => {
        expect((await fixture.runJsonCli(['name', 'Turnover sender', '-s'])).code).toBe(0);
        const home = path.join(fixture.root, 'home');
        fs.mkdirSync(home);
        const scenario = path.join(fixture.root, 'turnover.json');
        const report = path.join(fixture.root, 'turnover-report.json');
        const checkpoint = path.join(fixture.root, 'turnover-checkpoint');
        const status = path.join(fixture.root, 'turnover.status');
        const event = (name: string, id: string, transition: string) => ({
          args: ['__hook', driver],
          input: {
            hook_event_name: name,
            session_id: id,
            [name === 'SessionStart' ? 'source' : 'reason']: transition,
          },
        });
        fs.writeFileSync(
          scenario,
          JSON.stringify([
            { args: ['name', 'Turnover recipient', '-s', '--json'] },
            event('SessionStart', session, 'startup'),
            event('SessionEnd', session, reason),
            { ...event('SessionStart', foreign, 'startup'), checkpoint },
            // A delayed end for the old session cannot undo the new start.
            event('SessionEnd', session, reason),
            { args: ['whoami', '--json'] },
          ])
        );
        const pane = fixture.createShellPane('turnover-target').pane;
        const command = [
          'env',
          `HOME=${home}`,
          `TMUX_TEAM_HOME=${fixture.globalDir}`,
          executable,
          fixture.executables.cli.executable,
          scenario,
          report,
          '--listen',
        ]
          .map(quote)
          .join(' ');
        fixture.tmux([
          'send-keys',
          '-t',
          pane,
          '-l',
          `${command}; printf '%s' "$?" > ${quote(status)}`,
        ]);
        fixture.tmux(['send-keys', '-t', pane, 'Enter']);
        await fixture.waitFor(
          () => fs.existsSync(checkpoint),
          15000,
          'provider end before same-process start'
        );
        const database = path.join(fixture.globalDir, 'tmux-team.db');
        const db = new Database(database, { readonly: true });
        try {
          const binding = () =>
            db.prepare('SELECT * FROM bindings WHERE pane_id = ?').get(pane) as {
              identity_id: string;
              runtime_pid: number;
              runtime_start_identity: string;
              runtime_state: string;
              last_transition: string;
              observed_provider_session_id: string;
              last_verified_at: string;
            };
          const ended = binding();
          expect(ended).toMatchObject({
            runtime_state: 'unknown',
            last_transition: 'ended',
            observed_provider_session_id: session,
          });
          expect(process.kill(ended.runtime_pid, 0)).toBe(true);
          fs.writeFileSync(checkpoint, 'continue');
          await fixture.waitFor(
            () => fs.existsSync(report),
            15000,
            'new session admitted by real hook'
          );
          const results = JSON.parse(fs.readFileSync(report, 'utf8'));
          expect(results[3].stderr).toBe('');
          expect(JSON.parse(results[3].stdout).hookSpecificOutput.additionalContext).toContain(
            ended.identity_id
          );
          expect(results[4].stderr).toContain('continuing without context');
          expect(JSON.parse(results[5].stdout)).toMatchObject({
            id: ended.identity_id,
            sessionState: 'running',
          });
          expect(binding()).toMatchObject({
            runtime_pid: ended.runtime_pid,
            runtime_start_identity: ended.runtime_start_identity,
            runtime_state: 'running',
            observed_provider_session_id: foreign,
          });
          const sent = await fixture.runJsonCli(
            ['talk', 'Turnover recipient', 'turnover delivered once', '--detach', '--force'],
            { transportTrace: true }
          );
          expect(sent.code).toBe(0);
          expect(sent.json).toMatchObject({ status: 'sent', pane });
          expect(sent.json?.offline).toBeUndefined();
          const received = report.replace(/\.json$/, '.input.json');
          await fixture.waitFor(
            () => fs.existsSync(received),
            5000,
            'runtime-produced request input'
          );
          const input = JSON.parse(fs.readFileSync(received, 'utf8')) as string[];
          expect(input).toContain('turnover delivered once');
          expect(
            input.filter((line) => line.startsWith(`tmt reply ${sent.json?.requestId} `))
          ).toHaveLength(1);
          expect(
            db
              .prepare('SELECT wake_state FROM request_attempts WHERE request_id = ?')
              .get(sent.json?.requestId)
          ).toEqual({ wake_state: 'sent' });
          await waitForRuntimeExit(fixture, ended.runtime_pid, status);
          const trace = fixture.transportTrace();
          const offline = await fixture.runJsonCli(
            ['talk', 'Turnover recipient', 'after process exit', '--detach', '--force'],
            { transportTrace: true }
          );
          expect(offline.code).toBe(0);
          expect(offline.json).toMatchObject({
            status: 'queued',
            offline: true,
            notification: 'not_attempted',
            waitingFor: 'recipient_inbox_pull',
          });
          expect(fixture.transportTrace()).toEqual(trace);
        } finally {
          db.close();
        }
      },
      { mode: 'input-log' }
    );
  }
);

it.each([false, true])(
  'plain talk reaches an idle Codex after SessionEnd without another start (legacy=%s)',
  async (legacy) => {
    await withE2EFixture(
      async (fixture) => {
        expect((await fixture.runJsonCli(['name', 'Idle sender', '-s'])).code).toBe(0);
        const home = path.join(fixture.root, 'home');
        fs.mkdirSync(home);
        const scenario = path.join(fixture.root, 'idle.json');
        const report = path.join(fixture.root, 'idle-report.json');
        const checkpoint = path.join(fixture.root, 'idle-checkpoint');
        const status = path.join(fixture.root, 'idle.status');
        fs.writeFileSync(
          scenario,
          JSON.stringify([
            hook('startup'),
            {
              args: ['__hook', 'codex'],
              input: { hook_event_name: 'SessionEnd', session_id: session, reason: 'other' },
            },
            // This gate changes no provider state. No SessionStart or user input follows End.
            { args: ['whoami', '--json'], checkpoint },
          ])
        );
        const pane = fixture.createShellPane('idle-codex').pane;
        const command = [
          'env',
          `HOME=${home}`,
          `TMUX_TEAM_HOME=${fixture.globalDir}`,
          fixture.executables.cli.executable,
          ...fixture.executables.cli.args,
          'run',
          '--no-channel',
          '-s',
          'Idle recipient',
          '/opt/tmt-tests/hook-runtime/codex',
          fixture.executables.cli.executable,
          scenario,
          report,
          '--listen',
        ]
          .map(quote)
          .join(' ');
        fixture.tmux([
          'send-keys',
          '-t',
          pane,
          '-l',
          `${command}; printf '%s' "$?" > ${quote(status)}`,
        ]);
        fixture.tmux(['send-keys', '-t', pane, 'Enter']);
        await fixture.waitFor(
          () => fs.existsSync(checkpoint),
          15000,
          'idle provider-ended runtime'
        );
        const db = new Database(path.join(fixture.globalDir, 'tmux-team.db'));
        let runtimePid = 0;
        try {
          const binding = () =>
            db.prepare('SELECT * FROM bindings WHERE pane_id = ?').get(pane) as {
              identity_id: string;
              runtime_pid: number;
              runtime_state: string;
              last_verified_at: string;
              launch_owner_pid: number;
              launch_owner_start_identity: string;
            };
          const ended = binding();
          runtimePid = ended.runtime_pid;
          expect(ended).toMatchObject({
            runtime_state: 'unknown',
            last_transition: 'ended',
            observed_provider_session_id: session,
          });
          expect(ended.launch_owner_pid).toBeGreaterThan(0);
          expect(ended.launch_owner_start_identity).toBeTruthy();
          expect(process.kill(runtimePid, 0)).toBe(true);
          expect(process.kill(ended.launch_owner_pid, 0)).toBe(true);
          if (legacy)
            db.prepare("UPDATE bindings SET runtime_state = 'ended' WHERE pane_id = ?").run(pane);
          const recorded = binding();

          // Exact PID alone is insufficient: a stopped recipient cannot accept input.
          process.kill(runtimePid, 'SIGSTOP');
          await fixture.waitFor(
            () => {
              return execFileSync('ps', ['-p', String(runtimePid), '-o', 'stat='], {
                encoding: 'utf8',
              })
                .trim()
                .startsWith('T');
            },
            5000,
            'fixture runtime stopped'
          );
          const refused = await fixture.runJsonCli(
            ['talk', 'Idle recipient', 'stopped JSON refusal', '--detach', '--force'],
            { transportTrace: true }
          );
          expect(refused.code).toBe(1);
          expect(refused.json).toMatchObject({
            deliveryState: 'not_delivered',
            error: {
              code: 'DELIVERY_PREPARATION_FAILED',
              message: expect.stringContaining('not delivered live'),
              suggestion: expect.stringContaining(
                `tmt inbox --identity ${ended.identity_id} --json`
              ),
            },
          });
          const human = await fixture.runCli(
            ['talk', 'Idle recipient', 'stopped human refusal', '--detach', '--force'],
            { transportTrace: true }
          );
          expect(human.code).toBe(1);
          expect(human.stderr).toContain('not delivered live');
          expect(human.stderr).toContain('new turn or session');
          expect(human.stderr).toContain('tmt resume');
          expect(fixture.transportTrace()).toEqual([]);
          process.kill(runtimePid, 'SIGCONT');

          const sent = await fixture.runJsonCli(
            ['talk', 'Idle recipient', 'idle delivery without typing', '--detach', '--force'],
            { transportTrace: true }
          );
          expect(sent.code).toBe(0);
          expect(sent.json).toMatchObject({ status: 'sent', pane });
          expect(sent.json?.offline).toBeUndefined();
          const { last_verified_at: before, ...original } = recorded;
          const { last_verified_at: after, ...preserved } = binding();
          expect(Date.parse(after)).toBeGreaterThanOrEqual(Date.parse(before));
          expect(preserved).toEqual(original);
          expect(
            db
              .prepare('SELECT wake_state FROM request_attempts WHERE request_id = ?')
              .get(sent.json?.requestId)
          ).toEqual({ wake_state: 'sent' });
          fs.writeFileSync(checkpoint, 'continue');
          const received = report.replace(/\.json$/, '.input.json');
          await fixture.waitFor(() => fs.existsSync(received), 5000, 'idle runtime consumed talk');
          const input = JSON.parse(fs.readFileSync(received, 'utf8')) as string[];
          expect(input.filter((line) => line === 'idle delivery without typing')).toHaveLength(1);
          expect(
            input.filter((line) => line.startsWith(`tmt reply ${sent.json?.requestId} `))
          ).toHaveLength(1);
          const results = JSON.parse(fs.readFileSync(report, 'utf8'));
          expect(results).toHaveLength(3);
          expect(JSON.parse(results[2].stdout).sessionState).toBe(legacy ? 'ended' : 'unknown');
          await waitForRuntimeExit(fixture, runtimePid, status);
        } finally {
          if (runtimePid) {
            try {
              process.kill(runtimePid, 'SIGCONT');
            } catch (error) {
              if ((error as NodeJS.ErrnoException).code !== 'ESRCH') throw error;
            }
          }
          db.close();
        }
      },
      { mode: 'input-log' }
    );
  }
);

it('requires the hook runtime to exit and be reaped after publishing its report', async () => {
  await withE2EFixture(
    async (fixture) => {
      const scenario = path.join(fixture.root, 'held-runtime.json');
      const report = path.join(fixture.root, 'held-runtime-report.json');
      const status = path.join(fixture.root, 'held-runtime.status');
      fs.mkdirSync(path.join(fixture.root, 'home'));
      fs.writeFileSync(
        scenario,
        JSON.stringify([{ args: ['name', 'Held Reader', '-s', '--json'] }, hook('startup')])
      );
      const pane = fixture.createShellPane('held-runtime').pane;
      const command = [
        'env',
        `HOME=${path.join(fixture.root, 'home')}`,
        `TMUX_TEAM_HOME=${fixture.globalDir}`,
        '/opt/tmt-tests/hook-runtime/codex',
        fixture.executables.cli.executable,
        scenario,
        report,
        '--listen',
      ]
        .map(quote)
        .join(' ');
      fixture.tmux([
        'send-keys',
        '-t',
        pane,
        '-l',
        `${command}; printf '%s' "$?" > ${quote(status)}`,
      ]);
      fixture.tmux(['send-keys', '-t', pane, 'Enter']);
      await fixture.waitFor(() => fs.existsSync(report), 15000, 'held runtime hook report');
      const results = JSON.parse(fs.readFileSync(report, 'utf8'));
      expect(
        results.every(
          (item: { code: number; stderr: string }) => item.code === 0 && item.stderr === ''
        )
      ).toBe(true);
      const db = new Database(path.join(fixture.globalDir, 'tmux-team.db'), { readonly: true });
      try {
        const identity = JSON.parse(results[0].stdout);
        expect(JSON.parse(results[1].stdout).hookSpecificOutput.additionalContext).toContain(
          identity.id
        );
        const { runtime_pid: pid } = db
          .prepare('SELECT runtime_pid FROM bindings WHERE identity_id = ?')
          .get(identity.id) as { runtime_pid: number };
        expect(process.kill(pid, 0)).toBe(true);
        expect(fs.existsSync(status)).toBe(false);
        await expect(waitForRuntimeExit(fixture, pid, status, 100)).rejects.toThrow(
          'Timed out waiting for runtime fixture reaped by its pane shell.'
        );
        fixture.tmux([
          'send-keys',
          '-t',
          pane,
          '-l',
          'Submit your response with the command above.',
        ]);
        fixture.tmux(['send-keys', '-t', pane, 'Enter']);
        await waitForRuntimeExit(fixture, pid, status);
      } finally {
        db.close();
      }
    },
    { mode: 'input-log' }
  );
});
