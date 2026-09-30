import fs from 'node:fs';
import path from 'node:path';
import Database from 'better-sqlite3';
import { describe, expect, it } from 'vitest';
import { withE2EFixture, type E2EFixture } from './harness.js';

// #519: a turn end (`Stop`) records the context usage the driver reads from
// its own provider's transcript. The transcript lines are the real,
// minimized fixtures the Rust tests use.
const fixtures = path.resolve('../rust/crates/tmt-adapters/src/runtime/fixtures');
const claudeLine = fs.readFileSync(path.join(fixtures, 'claude-assistant-usage.jsonl'), 'utf8');
const codexLine = fs.readFileSync(path.join(fixtures, 'codex-token-count.jsonl'), 'utf8');
const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;

interface Provider {
  name: 'claude' | 'codex';
  runtime: string;
  /** The transcript directory under HOME that the driver trusts. */
  tree: string;
  line: string;
  usage: Record<string, number>;
}

const providers: Provider[] = [
  {
    name: 'claude',
    runtime: '/opt/tmt-tests/claude',
    tree: '.claude/projects/-workspace',
    line: claudeLine,
    usage: { tokens: 195664 },
  },
  {
    name: 'codex',
    runtime: '/opt/tmt-tests/hook-runtime/codex',
    tree: '.codex/sessions/2026/09/29',
    line: codexLine,
    usage: { tokens: 146577, windowTokens: 258400 },
  },
];

const session = '55555555-5555-4555-8555-555555555555';
const other = '66666666-6666-4666-8666-666666666666';

async function runScenario(fixture: E2EFixture, provider: Provider) {
  const home = path.join(fixture.root, `${provider.name} home`);
  const tree = path.join(home, provider.tree);
  fs.mkdirSync(tree, { recursive: true });
  const transcript = path.join(tree, `${session}.jsonl`);
  fs.writeFileSync(transcript, `{"type":"user"}\n${provider.line}`);
  const outside = path.join(fixture.root, 'outside.jsonl');
  fs.writeFileSync(outside, provider.line);
  const hook = (input: Record<string, unknown>) => ({
    args: ['__hook', provider.name],
    input: { session_id: session, ...input },
  });
  const stop = (transcriptPath: string | null, id = session) =>
    hook({ hook_event_name: 'Stop', session_id: id, transcript_path: transcriptPath });
  const list = { args: ['ls', '--json'] };
  const scenario = path.join(fixture.root, `${provider.name}-usage.json`);
  const report = path.join(fixture.root, `${provider.name}-usage-report.json`);
  fs.writeFileSync(
    scenario,
    JSON.stringify([
      { args: ['name', 'Usage Reader', '-s', '--json'] },
      hook({ hook_event_name: 'SessionStart', source: 'startup', model: 'model-a' }),
      stop(transcript),
      list,
      // A late turn end from another conversation, a path outside the
      // driver's tree and a missing file change nothing, silently.
      stop(transcript, other),
      stop(outside),
      stop(path.join(tree, 'missing.jsonl')),
      stop(null),
      list,
      // A compacted context drops usage; the model stays.
      hook({ hook_event_name: 'SessionStart', source: 'compact', model: 'model-a' }),
      list,
    ])
  );
  const command = [
    'env',
    `HOME=${home}`,
    `TMUX_TEAM_HOME=${fixture.globalDir}`,
    provider.runtime,
    fixture.executables.cli.executable,
    scenario,
    report,
  ]
    .map(quote)
    .join(' ');
  const pane = fixture.createShellPane(`${provider.name}-usage`).pane;
  fixture.tmux(['send-keys', '-t', pane, '-l', command]);
  fixture.tmux(['send-keys', '-t', pane, 'Enter']);
  await fixture.waitFor(() => fs.existsSync(report), 15000, `${provider.name} usage report`);
  return JSON.parse(fs.readFileSync(report, 'utf8')) as Array<{
    code: number;
    stdout: string;
    stderr: string;
  }>;
}

const resumeOf = (stdout: string) =>
  (JSON.parse(stdout).identities as Array<{ name: string; resume?: Record<string, unknown> }>).find(
    (identity) => identity.name === 'Usage Reader'
  )?.resume;

describe.sequential('turn-end usage hooks with a real pane and verified runtime', () => {
  for (const provider of providers) {
    it(`records ${provider.name} usage only for the remembered conversation`, async () => {
      await withE2EFixture(
        async (fixture) => {
          expect((await fixture.runJsonCli(['name', 'Owner', '-s'])).code).toBe(0);
          const results = await runScenario(fixture, provider);
          expect(results).toHaveLength(11);
          for (const result of results) {
            expect(result.code).toBe(0);
            expect(result.stderr).toBe('');
          }
          // Turn ends print nothing: Stop output could carry a decision.
          for (const index of [2, 4, 5, 6, 7]) expect(results[index].stdout).toBe('');
          const recorded = resumeOf(results[3].stdout);
          expect(recorded).toMatchObject({ driver: provider.name, model: 'model-a' });
          expect(recorded?.usage).toMatchObject(provider.usage);
          expect(Object.keys(recorded?.usage as object).sort()).toEqual(
            [...Object.keys(provider.usage), 'observedAtMs'].sort()
          );
          expect(resumeOf(results[8].stdout)).toEqual(recorded);
          const compacted = resumeOf(results[10].stdout);
          expect(compacted).toMatchObject({ model: 'model-a' });
          expect(compacted).not.toHaveProperty('usage');

          const identity = JSON.parse(results[0].stdout);
          const db = new Database(path.join(fixture.globalDir, 'tmux-team.db'), {
            readonly: true,
          });
          try {
            expect(
              db
                .prepare(
                  'SELECT driver_state_version, driver_state FROM identity_session_preferences WHERE identity_id = ?'
                )
                .get(identity.id)
            ).toEqual({ driver_state_version: 1, driver_state: '{"model":"model-a"}' });
          } finally {
            db.close();
          }
        },
        { mode: 'input-log' }
      );
    });
  }
});
