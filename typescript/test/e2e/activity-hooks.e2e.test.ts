import fs from 'node:fs';
import path from 'node:path';
import Database from 'better-sqlite3';
import { expect, it } from 'vitest';
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
          ...(provider === 'codex' ? [hook('Stop', 'a'), list] : []),
        ];
        const scenario = path.join(fixture.root, 'activity-scenario.json');
        const report = path.join(fixture.root, 'activity-report.json');
        fs.writeFileSync(scenario, JSON.stringify(steps));
        const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;
        const command = [
          'env',
          `TMUX_TEAM_HOME=${fixture.globalDir}`,
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
        if (provider === 'codex') expect(activity(16)).toEqual(activity(12));
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
        expect(ended.sinceMs).toBeNull();
        expect(ended.lastActivityMs).toBe(activity(12).lastActivityMs);
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
        `TMUX_TEAM_HOME=${fixture.globalDir}`,
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
      const db = new Database(path.join(fixture.globalDir, 'tmux-team.db'));
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
