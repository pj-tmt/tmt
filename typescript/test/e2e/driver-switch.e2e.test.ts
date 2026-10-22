import { copyFileSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import Database from 'better-sqlite3';
import { describe, expect, it } from 'vite-plus/test';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { withE2EFixture } from './harness.js';
import { waitForFileContent } from './wait-for-file.js';

const quote = (text: string): string => `'${text.replaceAll("'", "'\\''")}'`;

describe('identity driver switching', { concurrent: false }, () => {
  for (const [first, second] of [
    ['claude', 'codex'],
    ['codex', 'claude'],
  ] as const) {
    it(`retains ${first}'s exact session across a ${second} switch in a caller pane`, async () => {
      await withE2EFixture(async (fixture) => {
        const source = fixture.createShellPane('switch-source').pane;
        const caller = fixture.createShellPane('switch-caller').pane;
        const native = path.join(fixture.root, 'native-peers');
        mkdirSync(native);
        const sessions = {
          claude: '11111111-1111-4111-8111-111111111111',
          codex: '22222222-2222-4222-8222-222222222222',
        };
        for (const provider of ['claude', 'codex'] as const) {
          // Reuse the existing lifecycle peer with a provider-shaped native
          // basename; the wrapper only records argv and selects its scenario.
          const peer = path.join(native, provider);
          copyFileSync('/opt/tmt-tests/claude', peer);
          const scenario = path.join(fixture.root, `${provider}.scenario.json`);
          const report = path.join(fixture.root, `${provider}.report.json`);
          const calls = path.join(fixture.root, `${provider}.calls.jsonl`);
          writeFileSync(
            scenario,
            JSON.stringify([
              {
                args: ['__hook', provider],
                input: {
                  hook_event_name: 'SessionStart',
                  session_id: sessions[provider],
                  source: 'startup',
                },
              },
              { args: ['whoami', '--json'] },
            ])
          );
          const record = `require('node:fs').appendFileSync(${JSON.stringify(calls)},JSON.stringify(process.argv.slice(1))+'\n')`;
          writeExecutable(
            path.join(fixture.wrapperDir, provider),
            `#!/bin/sh
${quote(process.execPath)} -e ${quote(record)} -- "$@" || exit "$?"
exec ${quote(peer)} ${quote(fixture.executables.cli.executable)} ${quote(scenario)} ${quote(report)}
`,
            0o700
          );
        }
        const run = async (pane: string, label: string, args: string[]) => {
          const status = path.join(fixture.root, `${label}.status`);
          const argv = [
            fixture.executables.cli.executable,
            ...fixture.executables.cli.args,
            ...args,
          ];
          fixture.tmux([
            'send-keys',
            '-t',
            pane,
            '-l',
            `${argv.map(quote).join(' ')}; printf '%s' "$?" > ${quote(status)}`,
          ]);
          fixture.tmux(['send-keys', '-t', pane, 'Enter']);
          expect(await waitForFileContent(status, { description: label })).toBe('0');
        };
        await run(source, 'initial', ['run', '--save', 'Switch', first]);
        const db = new Database(path.join(fixture.globalDir, 'tmt.db'), { readonly: true });
        try {
          const identity = db
            .prepare("SELECT id, name, lifetime FROM identities WHERE name = 'Switch'")
            .get() as { id: string; name: string; lifetime: string };
          expect(identity.lifetime).toBe('saved');
          const notes = path.join(fixture.globalDir, 'notes', identity.id);
          mkdirSync(notes, { recursive: true });
          writeFileSync(path.join(notes, 'notes.md'), 'Current work: retain this identity.');
          await run(caller, 'switch', [
            'resume',
            'Switch',
            '--driver',
            second,
            '--model',
            'switch-model',
            '--no-channel',
          ]);
          expect(
            db.prepare('SELECT pane_id FROM bindings WHERE identity_id = ?').get(identity.id)
          ).toEqual({ pane_id: caller });
          expect(
            db
              .prepare(
                'SELECT harness, provider_session_id FROM identity_resume_records WHERE identity_id = ?'
              )
              .all(identity.id)
          ).toEqual([{ harness: first, provider_session_id: sessions[first] }]);
          const args = JSON.parse(
            readFileSync(path.join(fixture.root, `${second}.calls.jsonl`), 'utf8').trim()
          ) as string[];
          expect(args).toContain('switch-model');
          expect(args.at(-1)).toContain('Current work: retain this identity.');
          expect(args.at(-1)).toContain('untrusted context');
          expect(args.at(-1)).not.toContain('--receipt');
          await run(caller, 'switch-back', ['resume', 'Switch', '--driver', first, '--no-channel']);
          const back = readFileSync(path.join(fixture.root, `${first}.calls.jsonl`), 'utf8')
            .trim()
            .split('\n')
            .map((line) => JSON.parse(line) as string[])
            .at(-1)!;
          expect(back).toContain(first === 'claude' ? '--resume' : 'resume');
          expect(back).toContain(sessions[first]);
          expect(back).not.toContain('switch-model');
          expect(
            db.prepare('SELECT id, name, lifetime FROM identities WHERE id = ?').get(identity.id)
          ).toEqual(identity);
          expect(
            db
              .prepare(
                'SELECT provider_session_id FROM identity_session_preferences WHERE identity_id = ?'
              )
              .get(identity.id)
          ).toEqual({ provider_session_id: sessions[first] });
          expect(fixture.tmux(['list-panes', '-a', '-F', '#{pane_id}'])).toContain(source);
          expect(fixture.tmux(['list-panes', '-a', '-F', '#{pane_id}'])).toContain(caller);
        } finally {
          db.close();
        }
      });
    });
  }
});
