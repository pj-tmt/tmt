import fs from 'node:fs';
import path from 'node:path';
import Database from 'better-sqlite3';
import { expect, it } from 'vite-plus/test';
import { withE2EFixture } from './harness.js';

it.each(['claude', 'codex'] as const)(
  '%s commits authoritative activity before the synchronous hook returns',
  async (provider) => {
    await withE2EFixture(
      async (fixture) => {
        const session = '11111111-1111-4111-8111-111111111111';
        const hook = (event: string, turn = 'a', id = session) => ({
          args: ['__hook', provider],
          input: {
            hook_event_name: event,
            session_id: id,
            turn_id: turn,
            source: 'startup',
            reason: 'other',
            notification_type: 'permission_prompt',
          },
        });
        const list = { args: ['ls', '--json'] };
        const steps = [
          { args: ['name', 'Activity Reader', '-s', '--json'] },
          hook('SessionStart'),
          list,
          hook('UserPromptSubmit'),
          list,
          hook('UserPromptSubmit'),
          list,
          hook('Stop'),
          list,
          hook('Stop'),
          list,
          hook('UserPromptSubmit', 'b'),
          list,
          hook('Stop', 'a', '22222222-2222-4222-8222-222222222222'),
          list,
          ...(provider === 'codex'
            ? [
                hook('Stop', 'a'),
                list,
                // An interrupt ends only its own turn (#3207): a late one is ignored,
                // a repeated one and the turn's own Stop are duplicates.
                hook('Interrupt', 'a'),
                list,
                hook('Interrupt', 'b'),
                list,
                hook('Interrupt', 'b'),
                list,
                hook('Stop', 'b'),
                list,
              ]
            : []),
          // Manual denial emits no clearing hook. An unrelated notification
          // must leave the last waiting observation unchanged.
          ...(provider === 'claude'
            ? [
                hook('Notification', 'b'),
                list,
                hook('Notification', 'b'),
                list,
                {
                  ...hook('Notification', 'b'),
                  input: { ...hook('Notification', 'b').input, notification_type: 'idle_prompt' },
                },
                list,
                hook('PostToolUseFailure', 'b'),
                list,
                hook('Notification', 'b'),
                list,
                { ...hook('PostToolUse', 'b'), measure: true },
                list,
                { ...hook('PostToolUse', 'b'), measure: true },
                list,
                hook('Notification', 'b'),
                list,
                hook('UserPromptSubmit', 'b'),
                list,
                hook('Notification', 'b'),
                list,
                hook('Stop', 'b'),
                list,
                hook('PostToolUse', 'b'),
                list,
              ]
            : []),
        ];
        const scenario = path.join(fixture.root, 'activity-scenario.json');
        const report = path.join(fixture.root, 'activity-report.json');
        fs.writeFileSync(scenario, JSON.stringify(steps));
        const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;
        const command = [
          'env',
          `TMT_HOME=${fixture.globalDir}`,
          provider === 'claude' ? '/opt/tmt-tests/claude' : '/opt/tmt-tests/hook-runtime/codex',
          fixture.executables.cli.executable,
          scenario,
          report,
        ]
          .map(quote)
          .join(' ');
        const pane = fixture.createShellPane('activity').pane;
        fixture.tmux(['send-keys', '-t', pane, '-l', command]);
        fixture.tmux(['send-keys', '-t', pane, 'Enter']);
        await fixture.waitFor(() => fs.existsSync(report), 15000, 'activity report');
        const results = JSON.parse(fs.readFileSync(report, 'utf8')) as Array<{
          code: number;
          stdout: string;
          stderr: string;
          elapsedMs?: number;
        }>;
        expect(results).toHaveLength(steps.length);
        expect(results.every((result) => result.code === 0 && result.stderr === '')).toBe(true);
        const row = (value: string) =>
          JSON.parse(value).identities.find(
            (item: { name: string }) => item.name === 'Activity Reader'
          );
        const activity = (index: number) => row(results[index].stdout).session.activity;
        expect(activity(2)).toEqual({
          state: 'unknown',
          sinceMs: null,
          lastActivityMs: null,
          providers: {},
        });
        // The remembered harness names a full driver; the level is additive and advisory.
        expect(row(results[2].stdout).driverSupport).toBe('full');
        expect(activity(4)).toEqual({
          state: 'working',
          sinceMs: expect.any(Number),
          lastActivityMs: expect.any(Number),
          providers: {},
        });
        expect(activity(6)).toEqual(activity(4));
        expect(activity(8).state).toBe('idle');
        expect(activity(8).lastActivityMs).toBeGreaterThanOrEqual(activity(4).lastActivityMs);
        expect(activity(10)).toEqual(activity(8));
        expect(activity(12).state).toBe('working');
        expect(activity(14)).toEqual(activity(12));
        if (provider === 'codex') {
          expect(activity(16)).toEqual(activity(12));
          expect(activity(18)).toEqual(activity(12));
          expect(activity(20).state).toBe('idle');
          expect(activity(20).lastActivityMs).toBeGreaterThanOrEqual(activity(12).lastActivityMs);
          expect(activity(22)).toEqual(activity(20));
          expect(activity(24)).toEqual(activity(20));
        }
        if (provider === 'claude') {
          expect(activity(16)).toEqual({
            state: 'waiting',
            sinceMs: expect.any(Number),
            lastActivityMs: expect.any(Number),
            providers: {},
          });
          expect(activity(18)).toEqual(activity(16));
          expect(activity(20)).toEqual(activity(16));
          expect(activity(22).state).toBe('working');
          expect(activity(24).state).toBe('waiting');
          expect(activity(26).state).toBe('working');
          expect(activity(28)).toEqual(activity(26));
          expect(activity(30).state).toBe('waiting');
          expect(activity(32).state).toBe('working');
          expect(activity(34).state).toBe('waiting');
          expect(activity(36).state).toBe('idle');
          expect(activity(38)).toEqual(activity(36));
          expect(results[25].elapsedMs).toEqual(expect.any(Number));
          expect(results[27].elapsedMs).toEqual(expect.any(Number));
          // Observations, not a timing assertion: the fixture measures the
          // actual CLI child from spawn through reap, before its badge read.
          console.info(
            'Claude ToolFinished wall time (ms):',
            JSON.stringify({
              full: results[25].elapsedMs,
              noOp: results[27].elapsedMs,
            })
          );
        }
        const last = provider === 'codex' ? 20 : 36;
        // The fixture runtime has exited, while its shell pane remains. A process
        // observation overrides the last provider working event without renewing it.
        await fixture.waitFor(
          () =>
            fixture
              .tmux(['display-message', '-p', '-t', pane, '#{pane_current_command}'])
              .trim() !== provider,
          5000,
          'fixture runtime exited'
        );
        const current = await fixture.runCli(['ls', '--json']);
        expect(current.code).toBe(0);
        const ended = row(current.stdout).session.activity;
        expect(ended.state).toBe('ended');
        expect(row(current.stdout).driverSupport).toBe('full');
        expect(ended.sinceMs).toBeNull();
        expect(ended.lastActivityMs).toBe(activity(last).lastActivityMs);
      },
      { mode: 'input-log' }
    );
  }
);

it('a failed synchronous activity write is never deferred past hook return', async () => {
  await withE2EFixture(
    async (fixture) => {
      const checkpoint = path.join(fixture.root, 'before-stop');
      const scenario = path.join(fixture.root, 'locked-activity.json');
      const report = path.join(fixture.root, 'locked-report.json');
      const hook = (event: string) => ({
        args: ['__hook', 'claude'],
        input: {
          hook_event_name: event,
          session_id: 'locked-session',
          source: 'startup',
        },
      });
      fs.writeFileSync(
        scenario,
        JSON.stringify([
          { args: ['name', 'Locked Activity', '-s'] },
          hook('SessionStart'),
          hook('UserPromptSubmit'),
          { ...hook('Stop'), checkpoint },
        ])
      );
      const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;
      const command = [
        'env',
        `TMT_HOME=${fixture.globalDir}`,
        '/opt/tmt-tests/claude',
        fixture.executables.cli.executable,
        scenario,
        report,
      ]
        .map(quote)
        .join(' ');
      const pane = fixture.createShellPane('locked-activity').pane;
      fixture.tmux(['send-keys', '-t', pane, '-l', command]);
      fixture.tmux(['send-keys', '-t', pane, 'Enter']);
      await fixture.waitFor(() => fs.existsSync(checkpoint), 15000, 'before stop checkpoint');
      const db = new Database(path.join(fixture.globalDir, 'tmt.db'));
      try {
        const read = () =>
          db.prepare('SELECT driver_state FROM identity_session_preferences').all();
        const before = read();
        expect(JSON.stringify(before)).toContain('working');
        db.exec('BEGIN IMMEDIATE');
        fs.writeFileSync(checkpoint, 'continue');
        await fixture.waitFor(() => fs.existsSync(report), 5000, 'failed hook returned');
        const results = JSON.parse(fs.readFileSync(report, 'utf8'));
        expect(results[3].code).toBe(0);
        expect(results[3].stdout).toBe('');
        expect(results[3].stderr).toBe('');
        db.exec('ROLLBACK');
        // No detached writer remains to use the now-available database.
        expect(read()).toEqual(before);
      } finally {
        if (db.inTransaction) db.exec('ROLLBACK');
        db.close();
      }
    },
    { mode: 'input-log' }
  );
});
