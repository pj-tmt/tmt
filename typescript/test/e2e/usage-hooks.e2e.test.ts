import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { writeExecutable } from '../support/executable-fixture.mjs';
import Database from 'better-sqlite3';
import { describe, expect, it } from 'vite-plus/test';
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

async function runScenario(fixture: E2EFixture, provider: Provider, historyBaseline = false) {
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
  const historyCheckpoint = path.join(fixture.root, `${provider.name}-history-baseline`);
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
      // Missing transcript evidence clears history continuity. The C1 history
      // cases need a new valid seed before measuring the following increment.
      ...(historyBaseline ? [{ ...stop(transcript), checkpoint: historyCheckpoint }] : []),
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
    `TMT_HOME=${fixture.globalDir}`,
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
  const historySeed = () => {
    const db = new Database(path.join(fixture.globalDir, 'tmux-team.db'), { readonly: true });
    try {
      return db
        .prepare('SELECT latest FROM consumption_sources WHERE driver=? AND session=?')
        .get(provider.name, session) as { latest: string | null };
    } finally {
      db.close();
    }
  };
  if (historyBaseline) {
    await fixture.waitFor(
      () => fs.existsSync(historyCheckpoint),
      15000,
      `${provider.name} unknown history checkpoint`
    );
    expect(historySeed().latest, 'missing evidence invalidates the prior history seed').toBeNull();
    fs.writeFileSync(historyCheckpoint, 'continue');
  }
  await fixture.waitFor(
    () => fs.existsSync(checkpoint),
    15000,
    `${provider.name} append checkpoint`
  );
  if (historyBaseline) {
    expect(JSON.parse(historySeed().latest!).consumption).toMatchObject({
      ...provider.consumption,
      sequence: 1,
    });
  }
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

describe(
  'turn-end usage hooks with a real pane and verified runtime',
  { concurrent: false },
  () => {
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
            expect(Object.keys(recorded?.consumption as object).sort()).toEqual(
              [
                'cacheWriteTokens',
                ...(provider.name === 'claude' ? ['deltaByModel', 'modelId'] : []),
                'cachedInputTokens',
                'complete',
                'epoch',
                'gap',
                'inputTokens',
                'observedAtMs',
                'outputTokens',
                'sequence',
              ].sort()
            );
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
  }
);

// #1492: the real foreground owner samples model-free providers while Stop is
// deliberately held. SQLite is only an independent readiness/cleanup oracle;
// feature assertions use the ordinary public ls and bounded API projection.
for (const provider of providers) {
  it(`samples ${provider.name} before Stop and seeds closed history without recounting`, async () => {
    await withE2EFixture(async (fixture) => {
      const home = path.join(fixture.root, 'sampling-home');
      const tree = path.join(home, provider.tree);
      fs.mkdirSync(tree, { recursive: true });
      const transcript = path.join(
        tree,
        provider.name === 'codex' ? `rollout-2026-10-04-${session}.jsonl` : `${session}.jsonl`
      );
      fs.writeFileSync(transcript, provider.line);
      const settings = path.join(
        home,
        provider.name === 'claude' ? '.claude/settings.json' : '.codex/hooks.json'
      );
      fs.writeFileSync(
        settings,
        JSON.stringify({
          hooks: {
            Stop: [
              {
                hooks: [
                  {
                    type: 'command',
                    command: `'${fixture.executables.cli.executable}' __hook ${provider.name}`,
                    timeout: 3,
                  },
                ],
              },
            ],
          },
        })
      );
      const fake = path.join(fixture.wrapperDir, provider.name);
      writeExecutable(fake, `#!/bin/sh\nexec ${quote(provider.runtime)} "$@"\n`, 0o700);
      const appendReady = path.join(fixture.root, 'append-ready');
      const stopReady = path.join(fixture.root, 'stop-ready');
      const report = path.join(fixture.root, 'sample-report.json');
      const scenario = path.join(fixture.root, 'sample-scenario.json');
      const hook = (event: string) => ({
        args: ['__hook', provider.name],
        input: {
          hook_event_name: event,
          session_id: session,
          turn_id: other,
          source: 'startup',
          model: 'model-a',
          transcript_path: transcript,
        },
      });
      fs.writeFileSync(
        scenario,
        JSON.stringify([
          hook('SessionStart'),
          hook('UserPromptSubmit'),
          { checkpoint: appendReady, args: ['whoami', '--json'] },
          { ...hook('Stop'), checkpoint: stopReady },
          { args: ['ls', '--json'] },
          hook('Stop'),
          { args: ['ls', '--json'] },
        ])
      );
      const pane = fixture.createShellPane('sampling').pane;
      const status = path.join(fixture.root, 'sampling.status');
      const command = [
        'env',
        `HOME=${home}`,
        `TMT_HOME=${fixture.globalDir}`,
        fixture.executables.cli.executable,
        ...fixture.executables.cli.args,
        'run',
        '--no-channel',
        '--save',
        'Sample Reader',
        fake,
        fixture.executables.cli.executable,
        scenario,
        report,
      ]
        .map(quote)
        .join(' ');
      fixture.tmux([
        'send-keys',
        '-t',
        pane,
        '-l',
        `${command}; printf '%s' "$?" > ${quote(status)}`,
      ]);
      fixture.tmux(['send-keys', '-t', pane, 'Enter']);
      await fixture.waitFor(
        () => fs.existsSync(appendReady),
        10000,
        'prompt baseline before append'
      );
      const database = new Database(path.join(fixture.globalDir, 'tmux-team.db'), {
        readonly: true,
      });
      try {
        expect(fs.existsSync(report)).toBe(false);
        fs.appendFileSync(transcript, provider.appended);
        const latest = () =>
          database
            .prepare(
              'SELECT s.identity_id,s.sampled_at_ms,s.latest FROM consumption_sources s JOIN identities i ON i.id=s.identity_id WHERE i.name=?'
            )
            .get('Sample Reader') as
            | { identity_id: string; sampled_at_ms: number; latest: string }
            | undefined;
        await fixture.waitFor(
          () => {
            const row = latest();
            if (!row?.latest) return false;
            return (
              JSON.parse(row.latest).consumption.outputTokens === provider.updated.outputTokens
            );
          },
          8500,
          'foreground counter advanced before Stop'
        );
        const observed = latest()!;
        const listed = await fixture.runJsonCli<{
          identities: Array<{
            name: string;
            resume?: { consumption: Record<string, unknown> };
            session: { activity: { state: string } };
          }>;
        }>(['ls']);
        expect(listed.code).toBe(0);
        const live = listed.json!.identities.find((row) => row.name === 'Sample Reader')!;
        expect(live.resume?.consumption).toMatchObject(provider.updated);
        expect(live.session.activity.state).toBe('working');
        expect(fs.existsSync(report), 'Stop has not run').toBe(false);
        fs.writeFileSync(appendReady, 'continue');
        await fixture.waitFor(() => fs.existsSync(stopReady), 2000, 'held Stop checkpoint');
        await fixture.waitFor(
          () => {
            const through = Math.floor(Date.now() / 5000) * 5000;
            const coverage = database
              .prepare(
                'SELECT SUM(covered_ms) AS covered FROM consumption_buckets WHERE identity_id=? AND from_ms<?'
              )
              .get(observed.identity_id, through) as { covered: number | null };
            return (
              through > Math.floor(observed.sampled_at_ms / 5000) * 5000 &&
              (coverage.covered ?? 0) > 0
            );
          },
          7000,
          'sample bucket closed with confirmed coverage'
        );
        const request = {
          version: 1,
          operation: 'consumption.history',
          input: { identityIds: [observed.identity_id], windowsMs: [60000], maxBuckets: 120 },
        };
        const history = JSON.parse(
          execFileSync(
            fixture.executables.cli.executable,
            [...fixture.executables.cli.args, 'api'],
            {
              input: JSON.stringify(request),
              encoding: 'utf8',
              timeout: 5000,
              env: { PATH: process.env.PATH, HOME: home, TMT_HOME: fixture.globalDir },
            }
          )
        );
        const seeded = history.identities[0];
        expect(seeded.latest.consumption).toMatchObject(provider.updated);
        expect(seeded.lastSampleAtMs).toBeLessThan(history.throughMs);
        const buckets = seeded.windows[0].buckets as Array<{
          inputTokens: number;
          outputTokens: number;
          coveredMs: number;
        }>;
        expect(buckets.reduce((sum, row) => sum + row.inputTokens, 0)).toBe(
          provider.updated.inputTokens - provider.consumption.inputTokens
        );
        expect(buckets.reduce((sum, row) => sum + row.outputTokens, 0)).toBe(
          provider.updated.outputTokens - provider.consumption.outputTokens
        );
        expect(buckets.reduce((sum, row) => sum + row.coveredMs, 0)).toBeGreaterThan(0);
        fs.writeFileSync(stopReady, 'continue');
        await fixture.waitFor(
          () => fs.existsSync(status),
          5000,
          'original child exited and wrapper reaped'
        );
        expect(fs.readFileSync(status, 'utf8')).toBe('0');
        const results = JSON.parse(fs.readFileSync(report, 'utf8')) as Array<{
          code: number;
          stdout: string;
          stderr: string;
        }>;
        expect(results.every((row) => row.code === 0 && row.stderr === '')).toBe(true);
        expect(results[3].stdout).toBe('');
        expect(results[5].stdout).toBe('');
        const first = JSON.parse(results[4].stdout).identities.find(
          (row: { name: string }) => row.name === 'Sample Reader'
        ).resume.consumption;
        const repeated = JSON.parse(results[6].stdout).identities.find(
          (row: { name: string }) => row.name === 'Sample Reader'
        ).resume.consumption;
        expect(first).toEqual(live.resume?.consumption);
        expect(repeated).toEqual(first);
        expect(
          database
            .prepare('SELECT runtime_state FROM bindings WHERE identity_id=?')
            .get(observed.identity_id)
        ).toEqual({ runtime_state: 'ended' });
        const totals = database
          .prepare(
            'SELECT SUM(input_tokens) AS input,SUM(output_tokens) AS output FROM consumption_buckets WHERE identity_id=?'
          )
          .get(observed.identity_id);
        expect(totals).toEqual({
          input: provider.updated.inputTokens - provider.consumption.inputTokens,
          output: provider.updated.outputTokens - provider.consumption.outputTokens,
        });
      } finally {
        database.close();
      }
    });
  }, 45000);
}

// C1: launch selection and completed-turn billing models are distinct. Each
// provider changes model twice inside one accepted Stop read, without replay.
describe('consumption cache-write and turn model attribution', { concurrent: false }, () => {
  for (const base of providers) {
    it(`persists ${base.name} model changes and cache-write separately through the CLI`, async () => {
      const context = (model: string) =>
        JSON.stringify({ type: 'turn_context', payload: { model, turn_id: model } }) +
        '\n' +
        JSON.stringify({ type: 'event_msg', payload: { type: 'task_started', turn_id: model } }) +
        '\n';
      const claude = (id: string, model: string, write: number) => {
        const entry = JSON.parse(claudeRecords[0]);
        entry.message.id = id;
        entry.message.model = model;
        entry.message.usage = {
          input_tokens: 2,
          cache_read_input_tokens: 5,
          cache_creation_input_tokens: write,
          output_tokens: 7,
        };
        return JSON.stringify(entry) + '\n';
      };
      const codex = (input: number, output: number, cached: number, write: number) => {
        const entry = JSON.parse(codexLine);
        Object.assign(entry.payload.info.total_token_usage, {
          input_tokens: input,
          output_tokens: output,
          cached_input_tokens: cached,
          cache_write_input_tokens: write,
          total_tokens: input + output,
        });
        return JSON.stringify(entry) + '\n';
      };
      const rows =
        base.name === 'claude'
          ? [
              {
                modelId: 'billing-b',
                inputTokens: 10,
                outputTokens: 7,
                cachedInputTokens: 5,
                cacheWriteTokens: 3,
              },
              {
                modelId: 'billing-c',
                inputTokens: 13,
                outputTokens: 7,
                cachedInputTokens: 5,
                cacheWriteTokens: 6,
              },
            ]
          : [
              {
                modelId: 'billing-b',
                inputTokens: 100,
                outputTokens: 10,
                cachedInputTokens: 50,
                cacheWriteTokens: 20,
              },
              {
                modelId: 'billing-c',
                inputTokens: 100,
                outputTokens: 10,
                cachedInputTokens: 30,
                cacheWriteTokens: 10,
              },
            ];
      const provider: Provider = {
        ...base,
        line: base.name === 'claude' ? base.line : context('billing-a') + base.line,
        appended:
          base.name === 'claude'
            ? claude('c1-b', 'billing-b', 3) + claude('c1-c', 'billing-c', 6)
            : context('billing-b') +
              codex(2674657971, 6910978, 2624251058, 20) +
              context('billing-c') +
              codex(2674658071, 6910988, 2624251088, 30),
        updated:
          base.name === 'claude'
            ? { inputTokens: 23, outputTokens: 14, cachedInputTokens: 10 }
            : { inputTokens: 2674658071, outputTokens: 6910988, cachedInputTokens: 2624251088 },
      };
      await withE2EFixture(
        async (fixture) => {
          const results = await runScenario(fixture, provider, true);
          expect(results).toHaveLength(16);
          for (const result of results) {
            expect(result.code).toBe(0);
            expect(result.stderr).toBe('');
          }
          const updated = resumeOf(results[11].stdout);
          expect(updated).toMatchObject({
            model: 'model-a',
            consumption: {
              ...provider.updated,
              cacheWriteTokens: base.name === 'claude' ? 9 : 30,
              modelId: 'billing-c',
              deltaByModel: rows,
              sequence: 2,
            },
          });
          expect(resumeOf(results[13].stdout)?.consumption).toEqual(updated?.consumption);
          const db = new Database(path.join(fixture.globalDir, 'tmux-team.db'), { readonly: true });
          try {
            const persisted = db
              .prepare('SELECT details FROM consumption_buckets WHERE input_tokens > 0')
              .all() as { details: string }[];
            expect(persisted).toHaveLength(1);
            expect(JSON.parse(persisted[0].details)).toEqual({
              cacheWriteTokens: base.name === 'claude' ? 9 : 30,
              byModel: rows,
            });
          } finally {
            db.close();
          }
        },
        { mode: 'input-log' }
      );
    }, 45000);
  }
});
