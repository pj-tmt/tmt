import { writeExecutable } from '../support/executable-fixture.mjs';
import fs from 'node:fs';
import path from 'node:path';
import { expect, it } from 'vitest';
import { withE2EFixture } from './harness.js';

it.each(['claude', 'codex'] as const)(
  '%s receives Squad reminders on the next prompt, once, with Stop unchanged',
  async (provider) => {
    await withE2EFixture(
      async (fixture) => {
        const quote = (text: string) => `'${text.replaceAll("'", "'\\''")}'`;
        const executable = path.join(fixture.wrapperDir, 'tmt-squad');
        const cache = path.join(fixture.root, 'cache');
        fs.chmodSync(fixture.wrapperDir, 0o755);
        writeExecutable(
          executable,
          `#!/bin/sh\nexec env XDG_CACHE_HOME=${quote(cache)} ${quote(path.join(path.dirname(fixture.executables.cli.executable), 'tmt-squad'))} "$@"\n`,
          0o755
        );
        expect((await fixture.runJsonCli(['name', 'Fixture Owner', '-s'])).code).toBe(0);
        expect((await fixture.runJsonCli(['identity', 'create', 'Reminder Lead'])).code).toBe(0);
        const initialized = await fixture.runCli([
          'squad',
          'init',
          'product',
          '--me',
          'Fixture Owner',
          '--json',
        ]);
        expect(initialized.code, initialized.stdout + initialized.stderr).toBe(0);
        expect((await fixture.runCli(['squad', 'lead', 'Reminder Lead', '--json'])).code).toBe(0);
        const config = path.join(fixture.globalDir, 'squad.toml');
        fs.writeFileSync(
          config,
          fs.readFileSync(config, 'utf8') +
            '\n[squad.product.reminders]\nenabled=true\nstale_after="1m"\n'
        );
        const notes = await fixture.runJsonCli<{ path: string }>([
          'notes',
          'path',
          '--identity',
          'Reminder Lead',
        ]);
        expect(notes.code).toBe(0);
        fs.writeFileSync(notes.json!.path, 'Observed lead plan');
        expect((await fixture.runCli(['squad', 'ls', '--json'])).code).toBe(0);
        const directory = path.join(cache, 'tmt-squad', 'staleness');
        const file = path.join(
          directory,
          fs.readdirSync(directory).find((name) => name.endsWith('.json'))!
        );
        // Independently age the fixture's observation; never wait a real minute.
        const observed = JSON.parse(fs.readFileSync(file, 'utf8'));
        observed.notes.sinceMs = Date.now() - 125_000;
        observed.observedAtMs = observed.notes.sinceMs;
        fs.writeFileSync(file, JSON.stringify(observed));
        const session = '11111111-1111-4111-8111-111111111111';
        const hook = (event: string) => ({
          args: ['__hook', provider],
          input: {
            hook_event_name: event,
            session_id: session,
            source: 'startup',
            reason: 'other',
            turn_id: 'reminder-turn',
          },
        });
        const scenario = path.join(fixture.root, 'reminder-scenario.json');
        const report = path.join(fixture.root, 'reminder-report.json');
        fs.writeFileSync(
          scenario,
          JSON.stringify([
            { args: ['name', 'Reminder Lead', '-s', '--json'] },
            hook('SessionStart'), // No extension consent yet.
            hook('Stop'),
            { args: ['extension', 'hooks', 'enable', 'squad', '--json'] },
            hook('UserPromptSubmit'),
            hook('UserPromptSubmit'), // The generation was already claimed.
            hook('Stop'),
          ])
        );
        const command = [
          'env',
          `TMUX_TEAM_HOME=${fixture.globalDir}`,
          `PATH=${fixture.wrapperDir}:${process.env.PATH ?? ''}`,
          provider === 'claude' ? '/opt/tmt-tests/claude' : '/opt/tmt-tests/hook-runtime/codex',
          fixture.executables.cli.executable,
          scenario,
          report,
        ]
          .map(quote)
          .join(' ');
        const pane = fixture.createShellPane('squad-reminder').pane;
        fixture.tmux(['send-keys', '-t', pane, '-l', command]);
        fixture.tmux(['send-keys', '-t', pane, 'Enter']);
        await fixture.waitFor(() => fs.existsSync(report), 15_000, 'mock-agent reminder report');
        const results = JSON.parse(fs.readFileSync(report, 'utf8')) as Array<{
          code: number;
          stdout: string;
          stderr: string;
        }>;
        expect(results).toHaveLength(7);
        expect(results.every((result) => result.code === 0 && result.stderr === '')).toBe(true);
        expect(JSON.parse(results[4].stdout)).toEqual({
          hookSpecificOutput: {
            hookEventName: 'UserPromptSubmit',
            additionalContext:
              'Extension squad (informational): "Squad product: stale lead notes. Review notes/task/state."\n',
          },
        });
        for (const index of [2, 5, 6]) expect(results[index].stdout).toBe('');
        expect(JSON.parse(fs.readFileSync(file, 'utf8')).notes.claimed).toBe(true);
        expect(fs.readFileSync(notes.json!.path, 'utf8')).toBe('Observed lead plan');
      },
      { mode: 'input-log' }
    );
  }
);
