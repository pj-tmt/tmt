import fs from 'node:fs';
import path from 'node:path';
import Database from 'better-sqlite3';
import { describe, expect, it } from 'vitest';
import { withE2EFixture, type E2EFixture } from './harness.js';

// #519/#872: Stop records context usage and completed-request counters from
// its own provider's transcript. The transcript lines are the real,
// minimized fixtures the Rust tests use.
const fixtures = path.resolve('../rust/crates/tmt-adapters/src/runtime/fixtures');
const claudeRecords = fs
  .readFileSync(path.join(fixtures, 'claude-usage-sequence.jsonl'), 'utf8')
  .trimEnd()
  .split('\n');
const claudeLine = claudeRecords[0] + '\n';
const codexLine = fs.readFileSync(path.join(fixtures, 'codex-token-count.jsonl'), 'utf8');
const codexUpdated = JSON.parse(codexLine);
codexUpdated.payload.info.total_token_usage.output_tokens += 10;
codexUpdated.payload.info.total_token_usage.total_tokens += 10;
const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;

interface Provider {
  name: 'claude' | 'codex';
  runtime: string;
  /** The transcript directory under HOME that the driver trusts. */
  tree: string;
  line: string;
  usage: Record<string, number>;
  consumption: Record<string, number>;
  appended: string;
  updated: Record<string, number>;
}

const providers: Provider[] = [
  {
    name: 'claude',
    runtime: '/opt/tmt-tests/claude',
    tree: '.claude/projects/-workspace',
    line: claudeLine,
    usage: { tokens: 355113 },
    consumption: { inputTokens: 0, outputTokens: 0, cachedInputTokens: 0 },
    appended: claudeRecords.slice(1).join('\n') + '\n',
    updated: { inputTokens: 5415987, outputTokens: 5609, cachedInputTokens: 5405674 },
  },
  {
    name: 'codex',
    runtime: '/opt/tmt-tests/hook-runtime/codex',
    tree: '.codex/sessions/2026/09/29',
    line: codexLine,
    usage: { tokens: 146577, windowTokens: 258400 },
    consumption: { inputTokens: 2674657871, outputTokens: 6910968, cachedInputTokens: 2624251008 },
    appended: JSON.stringify(codexUpdated) + '\n',
    updated: { inputTokens: 2674657871, outputTokens: 6910978, cachedInputTokens: 2624251008 },
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
  const checkpoint = path.join(fixture.root, `${provider.name}-append-ready`);
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
      // Append after the admitted baseline, then replay the unchanged Stop.
      { ...stop(transcript), checkpoint },
      list,
      stop(transcript),
      list,
      // A compacted context drops usage and consumption; the model stays.
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
  await fixture.waitFor(
    () => fs.existsSync(checkpoint),
    15000,
    `${provider.name} append checkpoint`
  );
  fs.appendFileSync(transcript, provider.appended);
  fs.writeFileSync(checkpoint, 'continue');
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

describe('turn-end usage hooks with a real pane and verified runtime', { concurrent: false }, () => {
  for (const provider of providers) {
    it(`records ${provider.name} usage only for the remembered conversation`, async () => {
      await withE2EFixture(
        async (fixture) => {
          expect((await fixture.runJsonCli(['name', 'Owner', '-s'])).code).toBe(0);
          const results = await runScenario(fixture, provider);
          expect(results).toHaveLength(15);
          for (const result of results) {
            expect(result.code).toBe(0);
            expect(result.stderr).toBe('');
          }
          // Turn ends print nothing: Stop output could carry a decision.
          for (const index of [2, 4, 5, 6, 7, 9, 11]) expect(results[index].stdout).toBe('');
          const recorded = resumeOf(results[3].stdout);
          expect(recorded).toMatchObject({ driver: provider.name, model: 'model-a' });
          expect(recorded?.usage).toMatchObject(provider.usage);
          expect(Object.keys(recorded?.usage as object).sort()).toEqual(
            [...Object.keys(provider.usage), 'observedAtMs'].sort()
          );
          expect(recorded?.consumption).toMatchObject({
            ...provider.consumption,
            sequence: 1,
            observedAtMs: expect.any(Number),
            epoch: expect.stringMatching(/^[0-9a-f-]{36}$/),
            complete: false,
            gap: true,
          });
          expect(Object.keys(recorded?.consumption as object).sort()).toEqual([
            'cachedInputTokens',
            'complete',
            'epoch',
            'gap',
            'inputTokens',
            'observedAtMs',
            'outputTokens',
            'sequence',
          ]);
          expect(resumeOf(results[8].stdout)).toEqual(recorded);
          const updated = resumeOf(results[10].stdout)?.consumption;
          expect(updated).toMatchObject({
            ...provider.updated,
            epoch: (recorded?.consumption as Record<string, unknown>).epoch,
            sequence: 2,
            complete: true,
            gap: false,
          });
          expect(resumeOf(results[12].stdout)?.consumption).toEqual(updated);
          const compacted = resumeOf(results[14].stdout);
          expect(compacted).toMatchObject({ model: 'model-a' });
          expect(compacted).not.toHaveProperty('usage');
          expect(compacted).not.toHaveProperty('consumption');

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
