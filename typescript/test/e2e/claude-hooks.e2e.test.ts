import fs from 'node:fs';
import path from 'node:path';
import Database from 'better-sqlite3';
import { describe, expect, it } from 'vite-plus/test';
import { withE2EFixture } from './harness.js';

const first = '11111111-1111-4111-8111-111111111111';
const second = '22222222-2222-4222-8222-222222222222';
const hook = (event: string, transition: string, session: string, model?: string) => ({
  args: ['__hook', 'claude'],
  input: {
    hook_event_name: event,
    session_id: session,
    [event === 'SessionStart' ? 'source' : 'reason']: transition,
    ...(model === undefined ? {} : { model }),
  },
});
const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;

describe(
  'Claude hooks with a real pane and verified runtime ancestry',
  { concurrent: false },
  () => {
    it('injects the bound identity, retains it through clear and fences stale end events', async () => {
      await withE2EFixture(
        async (fixture) => {
          // Initialize only fixture-owned server/storage before the unbound check.
          expect((await fixture.runJsonCli(['name', 'Fixture owner', '-s'])).code).toBe(0);
          expect(
            (await fixture.runJsonCli(['config', 'set', 'ui.paneBadge', 'on', '--global'])).code
          ).toBe(0);
          const scenario = path.join(fixture.root, 'hook-scenario.json');
          const report = path.join(fixture.root, 'hook-report.json');
          const whoami = { args: ['whoami', '--json'] };
          fs.writeFileSync(
            scenario,
            JSON.stringify([
              hook('SessionStart', 'startup', first),
              { args: ['name', 'Hook Reader', '-s', '--json'] },
              // Claude reports the model on startup and may omit it after /clear.
              hook('SessionStart', 'startup', first, 'claude-opus-5'),
              whoami,
              hook('SessionEnd', 'clear', first),
              hook('SessionStart', 'clear', second),
              whoami,
              hook('SessionEnd', 'other', first),
              whoami,
              hook('SessionStart', 'compact', second),
              hook('SessionEnd', 'other', second),
              whoami,
            ])
          );
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
          fixture.tmux(['new-window', '-d', '-t', 'e2e', '-n', 'claude-hooks', command]);
          await fixture.waitFor(
            () => fs.existsSync(report),
            15000,
            'complete hook lifecycle report'
          );
          const results = JSON.parse(fs.readFileSync(report, 'utf8')) as Array<{
            code: number;
            stdout: string;
            stderr: string;
            badge: string;
          }>;
          expect(results).toHaveLength(12);
          expect(results.every((item) => item.code === 0)).toBe(true);
          expect(results[0].stderr).toBe('');
          expect(JSON.parse(results[0].stdout).hookSpecificOutput.additionalContext).toBe(
            'TMT: this pane has no identity. If the user wants TMT messaging here, they can run: tmt name <name> (-s to save).\n'
          );
          const identity = JSON.parse(results[1].stdout);
          expect(results[1].badge).toBe('Hook Reader (tmt)');
          const runningBadge =
            '#[push-default]#[fg=green]●#[default]#[pop-default] Hook Reader (tmt)';
          for (const index of [2, 5, 9]) {
            expect(results[index].stderr).toBe('');
            expect(results[index].badge).toBe(runningBadge);
            const context = JSON.parse(results[index].stdout).hookSpecificOutput.additionalContext;
            expect(context).toContain('TMT identity: "Hook Reader" (saved)');
            expect(context).toContain(identity.id);
          }
          for (const index of [3, 6, 8]) {
            expect(JSON.parse(results[index].stdout)).toMatchObject({
              id: identity.id,
              sessionState: 'running',
            });
          }
          expect(results[7].stdout).toBe('');
          expect(results[7].stderr).toContain('continuing without context');
          expect(results[7].badge).toBe(runningBadge);
          expect(results[10].stdout).toBe('');
          expect(results[10].stderr).toBe('');
          expect(results[10].badge).toBe('Hook Reader (tmt)');
          expect(JSON.parse(results[11].stdout)).toMatchObject({
            id: identity.id,
            sessionState: 'running',
          });
          const db = new Database(path.join(fixture.globalDir, 'tmt.db'), { readonly: true });
          try {
            expect(
              db
                .prepare(
                  'SELECT preferred_harness, remembered_harness, runtime_mode, provider_session_id, driver_state_version, driver_state, stale_at_ms FROM identity_session_preferences WHERE identity_id = ?'
                )
                .get(identity.id)
            ).toEqual({
              preferred_harness: 'claude',
              remembered_harness: 'claude',
              runtime_mode: 'default',
              provider_session_id: second,
              // The new session keeps the model no later start reported.
              driver_state_version: 1,
              driver_state: '{"model":"claude-opus-5"}',
              stale_at_ms: null,
            });
          } finally {
            db.close();
          }
        },
        { mode: 'input-log' }
      );
    });

    it('records only the main provider of a pane; a nested provider is refused', async () => {
      await withE2EFixture(
        async (fixture) => {
          expect((await fixture.runJsonCli(['name', 'Fixture owner', '-s'])).code).toBe(0);
          const start = (provider: string) => ({
            args: ['__hook', provider],
            input: { hook_event_name: 'SessionStart', session_id: first, source: 'startup' },
          });
          // Each pane binds a new identity with no session, so only the
          // provider's position can refuse the same start that the control records.
          const cases = [
            { name: 'claude-in-claude', inner: '/opt/tmt-tests/claude', provider: 'claude' },
            {
              name: 'codex-in-claude',
              inner: '/opt/tmt-tests/hook-runtime/codex',
              provider: 'codex',
            },
            { name: 'main-claude', inner: undefined, provider: 'claude' },
          ];
          const reports = cases.map(({ name, inner, provider }) => {
            const file = (suffix: string) => path.join(fixture.root, `${name}-${suffix}`);
            fs.writeFileSync(
              file('scenario.json'),
              JSON.stringify([
                { args: ['name', name, '-s', '--json'] },
                start(provider),
                { args: ['whoami', '--json'] },
              ])
            );
            const cli = fixture.executables.cli.executable;
            const main = [cli, file('scenario.json'), file('report.json')];
            if (inner !== undefined) {
              fs.writeFileSync(file('outer.json'), JSON.stringify([{ args: main }]));
            }
            const command = [
              'env',
              `TMT_HOME=${fixture.globalDir}`,
              '/opt/tmt-tests/claude',
              ...(inner === undefined
                ? main
                : [inner, file('outer.json'), file('outer-report.json')]),
            ]
              .map(quote)
              .join(' ');
            fixture.tmux(['new-window', '-d', '-t', 'e2e', '-n', name, command]);
            return { name, inner, report: file('report.json'), outer: file('outer-report.json') };
          });
          const read = (file: string) =>
            JSON.parse(fs.readFileSync(file, 'utf8')) as Array<{
              code: number;
              stdout: string;
              stderr: string;
            }>;
          const db = () => new Database(path.join(fixture.globalDir, 'tmt.db'), { readonly: true });
          for (const { name, inner, report, outer } of reports) {
            await fixture.waitFor(
              () => fs.existsSync(inner === undefined ? report : outer),
              15000,
              `${name} report`
            );
            if (inner !== undefined) expect(read(outer), name).toMatchObject([{ code: 0 }]);
            const results = read(report);
            expect(
              results.map((item) => item.code),
              name
            ).toEqual([0, 0, 0]);
            const identity = JSON.parse(results[0].stdout).id as string;
            const database = db();
            let row:
              | {
                  runtime_state: string | null;
                  runtime_pid: number | null;
                  observed_provider_session_id: string | null;
                  provider_session_id: string | null;
                  remembered_harness: string | null;
                }
              | undefined;
            try {
              row = database
                .prepare(
                  `SELECT b.runtime_state, b.runtime_pid, b.observed_provider_session_id,
                          p.provider_session_id, p.remembered_harness
                   FROM bindings b LEFT JOIN identity_session_preferences p
                     ON p.identity_id = b.identity_id
                   WHERE b.identity_id = ?`
                )
                .get(identity) as typeof row;
            } finally {
              database.close();
            }
            if (inner === undefined) {
              expect(results[1].stderr, name).toBe('');
              expect(
                JSON.parse(results[1].stdout).hookSpecificOutput.additionalContext,
                name
              ).toContain(identity);
              expect(row, name).toMatchObject({
                runtime_state: 'running',
                observed_provider_session_id: first,
                provider_session_id: first,
                remembered_harness: 'claude',
              });
              expect(JSON.parse(results[2].stdout), name).toMatchObject({
                id: identity,
                sessionState: 'running',
              });
            } else {
              expect(results[1], name).toMatchObject({
                stdout: '',
                stderr: 'tmt: lifecycle context unavailable; continuing without context.\n',
              });
              expect(row, name).toMatchObject({
                runtime_pid: null,
                observed_provider_session_id: null,
              });
              expect(row?.runtime_state, name).not.toBe('running');
              expect(row?.provider_session_id ?? null, name).toBeNull();
              expect(JSON.parse(results[2].stdout).sessionState, name).not.toBe('running');
            }
          }
        },
        { mode: 'input-log' }
      );
    });
  }
);
