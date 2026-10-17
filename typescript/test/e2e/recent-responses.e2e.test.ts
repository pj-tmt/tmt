import fs from 'node:fs';
import path from 'node:path';
import Database from 'better-sqlite3';
import { expect, it } from 'vite-plus/test';
import { withE2EFixture } from './harness.js';

// Native runtime-shaped parents execute the installed hook admission, with no
// model, host provider settings or pane scraping. Checkpoints publish complete
// transcript records before Stop; every assertion uses the public local API.
const session = '55555555-5555-4555-8555-555555555555';
const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;
const source = JSON.parse(
  fs.readFileSync(
    path.resolve('../rust/crates/tmt-adapters/src/runtime/fixtures/claude-assistant-usage.jsonl'),
    'utf8'
  )
);

it.each(['claude', 'codex'] as const)(
  '%s retains only newly admitted final turns and honors usage consent',
  async (provider) => {
    await withE2EFixture(
      async (fixture) => {
        const created = await fixture.runJsonCli<{ identity: { id: string } }>([
          'identity',
          'create',
          'Recent Reader',
        ]);
        expect(created.code, created.stderr).toBe(0);
        const identityId = created.json!.identity.id;
        const home = path.join(fixture.root, 'provider-home');
        const tree = path.join(
          home,
          provider === 'claude' ? '.claude/projects/-workspace' : '.codex/sessions'
        );
        fs.mkdirSync(tree, { recursive: true });
        const transcript = path.join(tree, `${session}.jsonl`);
        const bin = path.join(fixture.root, 'stable-bin');
        fs.mkdirSync(bin);
        fs.symlinkSync(fixture.executables.cli.executable, path.join(bin, 'tmt'));
        const hook = (event: string, turn: string, text: string, id = session) => ({
          args: ['__hook', provider],
          input: {
            hook_event_name: event,
            session_id: id,
            turn_id: turn,
            source: 'startup',
            transcript_path: transcript,
            last_assistant_message: text,
          },
        });
        const recent = {
          args: ['api'],
          input: { version: 1, operation: 'responses.recent', input: { identityId } },
        };
        const texts = Array.from({ length: 12 }, (_, i) =>
          i >= 10
            ? 'same text in two distinct turns'
            : i === 9
              ? 'x'.repeat(8190) + '🦀tail'
              : `final-${i}\r\nline\n`
        );
        const checkpoints = texts.map((_, i) => path.join(fixture.root, `before-stop-${i}`));
        const steps: Array<{ args: string[]; input?: unknown; checkpoint?: string }> = [
          { args: ['setup', provider, '--usage', '--yes', '--json'] },
          { args: ['name', 'Recent Reader', '-s', '--json'] },
          hook('SessionStart', 'start', ''),
        ];
        const reads: number[] = [];
        for (let i = 0; i < texts.length; i++) {
          steps.push(hook('UserPromptSubmit', `turn-${i}`, texts[i]));
          steps.push({ ...hook('Stop', `turn-${i}`, texts[i]), checkpoint: checkpoints[i] });
          reads.push(steps.length);
          steps.push(recent);
        }
        const retained = steps.length;
        steps.push(
          hook('Stop', 'turn-11', 'duplicate must not replace the text'),
          recent,
          hook('Stop', 'turn-11', 'foreign session', '66666666-6666-4666-8666-666666666666'),
          recent,
          { args: ['setup', provider, '--no-usage', '--yes', '--json'] },
          hook('UserPromptSubmit', 'disabled', 'disabled capture'),
          hook('Stop', 'disabled', 'disabled capture'),
          recent
        );
        const scenario = path.join(fixture.root, 'recent-scenario.json');
        const report = path.join(fixture.root, 'recent-report.json');
        fs.writeFileSync(scenario, JSON.stringify(steps));
        const command = [
          'env',
          `HOME=${home}`,
          `TMT_HOME=${fixture.globalDir}`,
          `CLAUDE_CONFIG_DIR=${path.join(home, '.claude')}`,
          `CODEX_HOME=${path.join(home, '.codex')}`,
          `PATH=${bin}:${process.env.PATH ?? ''}`,
          provider === 'claude' ? '/opt/tmt-tests/claude' : '/opt/tmt-tests/hook-runtime/codex',
          fixture.executables.cli.executable,
          scenario,
          report,
        ]
          .map(quote)
          .join(' ');
        const pane = fixture.createShellPane('recent-responses').pane;
        fixture.tmux(['send-keys', '-t', pane, '-l', command]);
        fixture.tmux(['send-keys', '-t', pane, 'Enter']);
        for (let i = 0; i < texts.length; i++) {
          await fixture.waitFor(() => fs.existsSync(checkpoints[i]), 15000, `Stop ${i} reached`);
          const record = structuredClone(source);
          record.sessionId = session;
          record.message.content = [
            { type: 'thinking', thinking: 'not final text' },
            { type: 'text', text: texts[i] },
          ];
          fs.writeFileSync(transcript, JSON.stringify(record) + '\n');
          fs.writeFileSync(checkpoints[i], 'continue');
        }
        await fixture.waitFor(() => fs.existsSync(report), 15000, 'recent-response report');
        const results = JSON.parse(fs.readFileSync(report, 'utf8')) as Array<{
          code: number;
          stdout: string;
          stderr: string;
        }>;
        expect(results).toHaveLength(steps.length);
        for (const result of results) expect(result.code, result.stderr).toBe(0);
        for (let i = 0; i < texts.length; i++) {
          const resource = JSON.parse(results[reads[i]].stdout);
          const expected = texts.slice(Math.max(0, i - 9), i + 1).reverse();
          expect(resource.identityId).toBe(identityId);
          expect(resource.items).toHaveLength(expected.length);
          for (let n = 0; n < expected.length; n++) {
            const text = expected[n];
            expect(resource.items[n]).toEqual({
              text: Buffer.byteLength(text) > 8192 ? 'x'.repeat(8190) : text,
              endedMs: expect.any(Number),
              provider,
              truncated: Buffer.byteLength(text) > 8192,
              sessionId: session,
            });
          }
        }
        const final = JSON.parse(results[reads[11]].stdout);
        for (const index of [retained + 1, retained + 3, retained + 7]) {
          expect(JSON.parse(results[index].stdout)).toEqual(final);
        }
        const database = new Database(path.join(fixture.globalDir, 'tmt.db'), { readonly: true });
        try {
          const preferences = database
            .prepare('SELECT driver_state FROM identity_session_preferences WHERE identity_id=?')
            .get(identityId);
          expect(JSON.stringify(preferences)).not.toContain('same text in two distinct turns');
          expect(
            database
              .prepare('SELECT count(*) AS n FROM recent_responses WHERE identity_id=?')
              .get(identityId)
          ).toEqual({ n: 10 });
        } finally {
          database.close();
        }
        expect((await fixture.runJsonCli(['rm', 'Recent Reader', '--force'])).code).toBe(0);
        const retired = new Database(path.join(fixture.globalDir, 'tmt.db'), { readonly: true });
        try {
          expect(
            retired
              .prepare('SELECT count(*) AS n FROM recent_responses WHERE identity_id=?')
              .get(identityId)
          ).toEqual({ n: 0 });
        } finally {
          retired.close();
        }
      },
      { mode: 'input-log' }
    );
  },
  60000
);
