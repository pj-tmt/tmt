import fs from 'node:fs';
import path from 'node:path';
import Database from 'better-sqlite3';
import { expect, it } from 'vitest';
import { withE2EFixture } from './harness.js';

const session = '33333333-3333-4333-8333-333333333333';
const foreign = '44444444-4444-4444-8444-444444444444';
const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;
// Codex documents `model` (the active model slug) on every hook input.
const hook = (source: string, id = session, model = 'gpt-5.2-codex') => ({
  args: ['__hook', 'codex'],
  input: { hook_event_name: 'SessionStart', source, session_id: id, model },
});

it('maps independent Codex then shared exact-thread hooks without using the server pane', async () => {
  await withE2EFixture(
    async (fixture) => {
      expect((await fixture.runJsonCli(['name', 'Owner', '-s'])).code).toBe(0);
      expect(
        (await fixture.runJsonCli(['config', 'set', 'ui.paneBadge', 'on', '--global'])).code
      ).toBe(0);
      const scenario = path.join(fixture.root, 'codex-independent.json');
      const report = path.join(fixture.root, 'codex-independent-report.json');
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
      fixture.tmux(['send-keys', '-t', targetPane, '-l', command]);
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
      expect(results[4].badge).toBe(
        '#[push-default]#[dim]○ Codex Reader (tmt)#[default]#[pop-default]'
      );
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
        expect(before).toMatchObject({
          runtime_state: 'ended',
          runtime_mode: 'embedded',
          observed_provider_session_id: session,
        });
        // The old process must be gone, not merely done writing its report.
        await fixture.waitFor(
          () => {
            const row = db
              .prepare('SELECT runtime_pid FROM bindings WHERE identity_id = ?')
              .get(identity.id) as { runtime_pid: number };
            try {
              process.kill(row.runtime_pid, 0);
              return false;
            } catch (error) {
              if ((error as NodeJS.ErrnoException).code === 'ESRCH') return true;
              throw error;
            }
          },
          5000,
          'embedded fixture process exit'
        );
        const sharedScenario = path.join(fixture.root, 'codex-shared.json');
        const sharedReport = path.join(fixture.root, 'codex-shared-report.json');
        fs.writeFileSync(
          sharedScenario,
          JSON.stringify([
            hook('resume', foreign, 'gpt-5.3-codex'),
            hook('resume', session, 'gpt-5.3-codex'),
            hook('compact', session, 'gpt-5.3-codex'),
          ])
        );
        fixture.tmux([
          'new-window',
          '-d',
          '-t',
          'e2e',
          '-n',
          'codex-server',
          [
            'env',
            `TMUX_TEAM_HOME=${fixture.globalDir}`,
            '/opt/tmt-tests/hook-runtime/codex',
            'app-server',
            fixture.executables.cli.executable,
            sharedScenario,
            sharedReport,
          ]
            .map(quote)
            .join(' '),
        ]);
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
        await fixture.waitFor(
          () => {
            const row = db
              .prepare('SELECT runtime_pid FROM bindings WHERE identity_id = ?')
              .get(identity.id) as { runtime_pid: number };
            try {
              process.kill(row.runtime_pid, 0);
              return false;
            } catch (error) {
              if ((error as NodeJS.ErrnoException).code === 'ESRCH') return true;
              throw error;
            }
          },
          5000,
          'shared fixture process exit'
        );

        const resumed = path.join(fixture.root, 'resumed-client.json');
        const release = path.join(fixture.root, 'release-client');
        const status = path.join(fixture.root, 'client-exit.status');
        fs.writeFileSync(
          path.join(fixture.wrapperDir, 'codex'),
          `#!${process.execPath}\nconst fs = require('node:fs');\nfs.writeFileSync(${JSON.stringify(resumed)}, JSON.stringify(process.argv.slice(2)));\nsetInterval(() => { if (fs.existsSync(${JSON.stringify(release)})) process.exit(0); }, 20);\n`,
          { mode: 0o700 }
        );
        const resumeCommand = [
          fixture.executables.cli.executable,
          ...fixture.executables.cli.args,
          'run',
          '--resume',
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
          session,
          '-m',
          'gpt-5.3-codex',
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
