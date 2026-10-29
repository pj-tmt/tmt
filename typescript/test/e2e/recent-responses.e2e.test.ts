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
  '%s retains distinct finals in their recorded batches and reads no usage data without consent',
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
          waitForFlush:
            event === 'Stop'
              ? {
                  node: process.execPath,
                  script: path.resolve('test/support/wait-hook-flush.mjs'),
                  directory: fixture.globalDir,
                }
              : undefined,
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
              ? 'x'.repeat(16382) + '🦀tail'
              : `final-${i}\r\nline\n`
        );
        const checkpoints = texts.map((_, i) => path.join(fixture.root, `before-stop-${i}`));
        const steps: Array<{
          args: string[];
          input?: unknown;
          checkpoint?: string;
          waitForFlush?: { node: string; script: string; directory: string };
        }> = [
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
          hook('Stop', 'turn-11', texts[11]),
          recent,
          hook('Stop', 'turn-11', 'foreign session', '66666666-6666-4666-8666-666666666666'),
          recent,
          hook('Stop', 'turn-11', 'a distinct final whose prompt boundary was missed'),
          recent,
          hook('Stop', 'turn-11', 'a distinct final whose prompt boundary was missed'),
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
        fixture.watchHookFailures(pane);
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
          expect(resource.coverage).toEqual({ olderDropped: false });
          for (let n = 0; n < expected.length; n++) {
            const text = expected[n];
            expect(resource.items[n]).toEqual({
              responseId: expect.stringMatching(/^rsp_\d+$/),
              inputBatchId: expect.stringMatching(/^[0-9a-f-]{36}$/),
              text: Buffer.byteLength(text) > 16384 ? 'x'.repeat(16382) : text,
              endedMs: expect.any(Number),
              provider,
              truncated: Buffer.byteLength(text) > 16384,
              originalBytes: Buffer.byteLength(text),
              sessionId: session,
            });
          }
        }
        const final = JSON.parse(results[reads[11]].stdout);
        for (const index of [retained + 1, retained + 3]) {
          expect(JSON.parse(results[index].stdout)).toEqual(final);
        }
        const missedPrompt = JSON.parse(results[retained + 5].stdout);
        expect(missedPrompt.items[0]).toEqual({
          ...final.items[0],
          responseId: expect.stringMatching(/^rsp_\d+$/),
          text: 'a distinct final whose prompt boundary was missed',
          endedMs: expect.any(Number),
          truncated: false,
          originalBytes: Buffer.byteLength('a distinct final whose prompt boundary was missed'),
        });
        expect(missedPrompt.items[0].responseId).not.toBe(final.items[0].responseId);
        expect(missedPrompt.items[0].inputBatchId).toBe(final.items[0].inputBatchId);
        expect(missedPrompt.items.slice(1)).toEqual(final.items.slice(0, 9));
        expect(JSON.parse(results[retained + 7].stdout)).toEqual(missedPrompt);
        // --no-usage keeps the turn-end hook for the agent's answers: the payload text is kept
        // (a Stop after the opt-out is captured) and no transcript or usage data is read.
        const afterOptOut = JSON.parse(results[retained + 11].stdout);
        expect(afterOptOut.capture).toEqual({ enabled: true });
        expect(afterOptOut.items[0]).toMatchObject({
          text: 'disabled capture',
          provider,
          sessionId: session,
          originalBytes: Buffer.byteLength('disabled capture'),
        });
        expect(afterOptOut.items.slice(1)).toEqual(missedPrompt.items.slice(0, 9));
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
          ).toEqual({ n: 14 });
          expect(
            database
              .prepare(
                "SELECT count(*) AS n FROM capture_entries WHERE identity_id=? AND kind='final'"
              )
              .get(identityId)
          ).toEqual({ n: 14 });
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
