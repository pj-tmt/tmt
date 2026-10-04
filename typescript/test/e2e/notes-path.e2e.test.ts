import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import { withE2EFixture } from './harness.js';

const inputLog = { mode: 'input-log' } as const;

describe('saved identity notes through verified tmux callers', { concurrent: false }, () => {
  it('uses the verified saved identity implicitly and remains available explicitly offline', async () => {
    await withE2EFixture(async (fixture) => {
      const bound = await fixture.runJsonCli<{ id: string }>(['name', 'Saved Researcher', '-s']);
      expect(bound).toMatchObject({ code: 0, json: { id: expect.any(String) } });
      const expected = path.join(fixture.globalDir, 'notes', bound.json!.id, 'notes.md');

      expect(await fixture.runJsonCli(['notes', 'path'])).toMatchObject({
        code: 0,
        json: { identityId: bound.json!.id, path: expected, created: true },
      });
      fs.writeFileSync(expected, '# durable local context\n');
      expect(
        await fixture.runJsonCli(['notes', 'path', '--identity', 'Saved Researcher'], {
          withoutTmux: true,
        })
      ).toMatchObject({
        code: 0,
        json: { identityId: bound.json!.id, path: expected, created: false },
      });
      expect(fs.readFileSync(expected, 'utf8')).toBe('# durable local context\n');
    }, inputLog);
  });

  it('rejects temporary and unverifiable implicit callers before creating a notebook', async () => {
    await withE2EFixture(async (fixture) => {
      const bound = await fixture.runJsonCli<{ id: string }>(['name', 'Temporary']);
      expect(bound.code).toBe(0);
      expect(await fixture.runJsonCli(['notes', 'path'])).toMatchObject({
        code: 1,
        json: { error: { code: 'NOTES_SAVED_IDENTITY_REQUIRED' } },
      });
      expect(fs.existsSync(path.join(fixture.globalDir, 'notes', bound.json!.id))).toBe(false);

      const metadata = JSON.parse(fixture.paneMetadata());
      metadata.globalIdentity.bindingId = 'different-binding';
      fixture.tmux([
        'set-option',
        '-p',
        '-t',
        fixture.pane,
        '@tmux-team.agent',
        JSON.stringify(metadata),
      ]);
      expect(await fixture.runJsonCli(['notes', 'path'])).toMatchObject({
        code: 1,
        json: { error: { code: 'IDENTITY_REQUIRED' } },
      });
      expect(fs.existsSync(path.join(fixture.globalDir, 'notes'))).toBe(false);
    }, inputLog);
  });
});

const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;

it.each(['claude', 'codex'] as const)(
  'reminds only saved %s sessions after admitted compaction and obeys the global toggle',
  async (provider) => {
    for (const saved of [true, false]) {
      await withE2EFixture(async (fixture) => {
        expect((await fixture.runJsonCli(['name', 'Fixture Owner', '-s'])).code).toBe(0);
        const session = '55555555-5555-4555-8555-555555555555';
        const hook = (source: string) => ({
          args: ['__hook', provider],
          input: { hook_event_name: 'SessionStart', session_id: session, source },
        });
        const scenario = path.join(fixture.root, 'notes-hook-scenario.json');
        const report = path.join(fixture.root, 'notes-hook-report.json');
        fs.writeFileSync(
          scenario,
          JSON.stringify([
            { args: ['name', 'Notes Reader', ...(saved ? ['-s'] : []), '--json'] },
            hook('startup'),
            hook('compact'),
            { args: ['config', 'set', 'notes.compactionReminder', 'false', '--global', '--json'] },
            hook('compact'),
            { args: ['config', 'set', 'notes.compactionReminder', 'true', '--global', '--json'] },
            { args: ['notes', 'path', '--json'] },
            hook('compact'),
            // A later startup must not reuse the previously recorded Compacted transition.
            hook('startup'),
          ])
        );
        const runtime =
          provider === 'claude' ? '/opt/tmt-tests/claude' : '/opt/tmt-tests/hook-runtime/codex';
        const command = [
          'env',
          `TMUX_TEAM_HOME=${fixture.globalDir}`,
          runtime,
          fixture.executables.cli.executable,
          scenario,
          report,
        ]
          .map(quote)
          .join(' ');
        fixture.tmux(['new-window', '-d', '-t', 'e2e', '-n', 'notes-hooks', command]);
        await fixture.waitFor(() => fs.existsSync(report), 20000, 'causal notes hook report');
        const results = JSON.parse(fs.readFileSync(report, 'utf8')) as Array<{
          code: number;
          stdout: string;
          stderr: string;
        }>;
        expect(results).toHaveLength(9);
        const identity = JSON.parse(results[0].stdout);
        expect(identity.lifetime).toBe(saved ? 'saved' : 'temporary');
        const context = (index: number) => {
          expect(results[index].code).toBe(0);
          expect(results[index].stderr).toBe('');
          const text = JSON.parse(results[index].stdout).hookSpecificOutput
            .additionalContext as string;
          expect(Buffer.byteLength(text)).toBeLessThanOrEqual(4096);
          return text;
        };
        expect(context(1)).not.toContain('Context was compacted.');
        expect(context(4)).not.toContain('Context was compacted.');
        expect(context(8)).not.toContain('Context was compacted.');
        if (saved) {
          expect(context(2)).toContain(
            `Context was compacted. Re-read your notes using tmt notes path --identity '${identity.id}' and update them with anything still open.`
          );
          const notebook = JSON.parse(results[6].stdout);
          expect(notebook).toMatchObject({ identityId: identity.id, created: true });
          expect(context(7)).toContain(
            `Context was compacted. Re-read your notes at ${JSON.stringify(notebook.path)} and update them with anything still open.`
          );
          expect(fs.readFileSync(notebook.path, 'utf8')).toBe('');
        } else {
          expect(context(2)).not.toContain('Context was compacted.');
          expect(context(7)).not.toContain('Context was compacted.');
          expect(results[6].code).toBe(1);
          expect(JSON.parse(results[6].stdout).error.code).toBe('NOTES_SAVED_IDENTITY_REQUIRED');
          expect(fs.existsSync(path.join(fixture.globalDir, 'notes'))).toBe(false);
        }
      }, inputLog);
    }
  }
);
