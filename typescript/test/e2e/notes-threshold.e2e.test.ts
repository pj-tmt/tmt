import Database from 'better-sqlite3';
import fs from 'node:fs';
import path from 'node:path';
import { expect, it } from 'vite-plus/test';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { withE2EFixture, type E2EFixture } from './harness.js';

const quote = (text: string) => `'${text.replaceAll("'", "'\\''")}'`;
const session = '55555555-5555-4555-8555-555555555555';
const fixtureRoot = path.resolve('../rust/crates/tmt-adapters/src/runtime/fixtures');
type Provider = 'claude' | 'codex';
type Step = { args: string[]; input?: object; checkpoint?: string; close_stdout?: boolean };
interface Result {
  code: number;
  stdout: string;
  stderr: string;
}
interface Row {
  id: string;
  notes_nudge: number | null;
  driver_state: string;
  latest: string | null;
}

// Derived provider fixtures change context evidence causally at each checkpoint.
// Claude still has no reported window. Codex's completed-request counters remain
// enormous and unchanged, so they cannot accidentally drive eligibility.
function usage(provider: Provider, tokens: number): string {
  if (provider === 'claude') {
    const record = JSON.parse(
      fs.readFileSync(path.join(fixtureRoot, 'claude-usage-sequence.jsonl'), 'utf8').split('\n')[0]
    );
    for (const value of [record.message.usage, ...record.message.usage.iterations]) {
      value.input_tokens = tokens;
      value.cache_creation_input_tokens = 0;
      value.cache_read_input_tokens = 0;
    }
    return JSON.stringify(record) + '\n';
  }
  const record = JSON.parse(
    fs.readFileSync(path.join(fixtureRoot, 'codex-token-count.jsonl'), 'utf8')
  );
  record.payload.info.last_token_usage.total_tokens = tokens;
  record.payload.info.model_context_window = 10000;
  return JSON.stringify(record) + '\n';
}

function startSampler(
  f: E2EFixture,
  provider: Provider,
  saved: boolean,
  promptHook: boolean,
  steps: (hook: (source?: string) => Step, checkpoints: string[]) => Step[]
) {
  const home = path.join(f.root, 'notes-sampling-home');
  const tree = path.join(
    home,
    provider === 'codex' ? '.codex/sessions/2026/10/04' : '.claude/projects/-workspace'
  );
  fs.mkdirSync(tree, { recursive: true });
  const transcript = path.join(
    tree,
    provider === 'codex' ? `rollout-2026-10-04-${session}.jsonl` : `${session}.jsonl`
  );
  fs.writeFileSync(transcript, usage(provider, 7900));
  const settings = path.join(
    home,
    provider === 'codex' ? '.codex/hooks.json' : '.claude/settings.json'
  );
  const entry = [
    {
      hooks: [
        {
          type: 'command',
          command: `${quote(f.executables.cli.executable)} __hook ${provider}`,
          timeout: 3,
        },
      ],
    },
  ];
  fs.writeFileSync(
    settings,
    JSON.stringify({ hooks: { Stop: entry, ...(promptHook ? { UserPromptSubmit: entry } : {}) } })
  );
  const runtime =
    provider === 'codex' ? '/opt/tmt-tests/hook-runtime/codex' : '/opt/tmt-tests/claude';
  const fake = path.join(f.wrapperDir, provider);
  writeExecutable(fake, `#!/bin/sh\nexec ${quote(runtime)} "$@"\n`, 0o700);
  const checkpoints = [0, 1, 2].map((i) => path.join(f.root, `notes-sample-ready-${i}`));
  const hook = (source?: string): Step => ({
    args: ['__hook', provider],
    input: {
      hook_event_name: source ? 'SessionStart' : 'UserPromptSubmit',
      session_id: session,
      source,
      transcript_path: transcript,
    },
  });
  const scenario = path.join(f.root, 'notes-sample-scenario.json');
  const report = path.join(f.root, 'notes-sample-report.json');
  const status = path.join(f.root, 'notes-sample-status');
  fs.writeFileSync(scenario, JSON.stringify(steps(hook, checkpoints)));
  const pane = f.createShellPane('notes-sampling').pane;
  const command = [
    'env',
    `HOME=${home}`,
    `CODEX_HOME=${path.join(home, '.codex')}`,
    `CLAUDE_CONFIG_DIR=${path.join(home, '.claude')}`,
    `TMUX_TEAM_HOME=${f.globalDir}`,
    f.executables.cli.executable,
    ...f.executables.cli.args,
    'run',
    '--no-channel',
    ...(saved ? ['--save'] : []),
    'Threshold Reader',
    fake,
    f.executables.cli.executable,
    scenario,
    report,
  ]
    .map(quote)
    .join(' ');
  f.tmux(['send-keys', '-t', pane, '-l', `${command}; printf '%s' "$?" > ${quote(status)}`]);
  f.tmux(['send-keys', '-t', pane, 'Enter']);
  return { transcript, checkpoints, report, status };
}

function row(db: Database.Database): Row | undefined {
  return db
    .prepare(
      'SELECT i.id,b.notes_nudge,p.driver_state,s.latest FROM identities i JOIN bindings b ON b.identity_id=i.id JOIN identity_session_preferences p ON p.identity_id=i.id LEFT JOIN consumption_sources s ON s.identity_id=i.id WHERE i.name=?'
    )
    .get('Threshold Reader') as Row | undefined;
}
const promptContext = (result: Result) =>
  result.stdout ? (JSON.parse(result.stdout).hookSpecificOutput.additionalContext as string) : '';

async function settled(f: E2EFixture, worker: ReturnType<typeof startSampler>) {
  // On assertion failure, release current and future owned fixture checkpoints,
  // then await the wrapper's post-reap status before the harness removes its root.
  await f.waitFor(
    () => {
      for (const checkpoint of worker.checkpoints) {
        if (fs.existsSync(checkpoint) && fs.readFileSync(checkpoint, 'utf8') === 'ready')
          fs.writeFileSync(checkpoint, 'continue');
      }
      return fs.existsSync(worker.status);
    },
    15000,
    'notes sampler foreground and wrapper settled'
  );
}

it.each([
  { provider: 'codex' as const, saved: true, promptHook: true, eligible: true },
  { provider: 'codex' as const, saved: false, promptHook: true, eligible: false },
  { provider: 'codex' as const, saved: true, promptHook: false, eligible: false },
  { provider: 'claude' as const, saved: true, promptHook: true, eligible: false },
])(
  'claims $provider threshold context only for saved=$saved, promptHook=$promptHook',
  async ({ provider, saved, promptHook, eligible }) => {
    await withE2EFixture(
      async (f) => {
        const worker = startSampler(f, provider, saved, promptHook, (hook, checkpoints) => [
          hook('startup'),
          hook(),
          { args: ['whoami', '--json'], checkpoint: checkpoints[0] },
          hook(),
          hook(),
          hook('compact'),
          { args: ['whoami', '--json'], checkpoint: checkpoints[1] },
          hook(),
          hook(),
          hook('startup'),
          hook(),
        ]);
        let db: Database.Database | undefined;
        try {
          await f.waitFor(
            () => fs.existsSync(worker.checkpoints[0]),
            10000,
            'before threshold append'
          );
          db = new Database(path.join(f.globalDir, 'tmux-team.db'), { readonly: true });
          expect(row(db)?.notes_nudge).toBeNull();
          fs.appendFileSync(worker.transcript, usage(provider, 8000));
          await f.waitFor(
            () => {
              const current = row(db!);
              const used = current && JSON.parse(current.driver_state).usage?.tokens;
              return used === 8000 && current!.notes_nudge === (eligible ? 80 : null);
            },
            8500,
            'fresh reported usage and eligibility committed before prompt'
          );
          const id = row(db)!.id;
          expect(fs.existsSync(path.join(f.globalDir, 'notes'))).toBe(false);
          // Keep the next climb below the threshold until its causal append.
          fs.appendFileSync(worker.transcript, usage(provider, 1000));
          fs.writeFileSync(worker.checkpoints[0], 'continue');
          await f.waitFor(
            () => fs.existsSync(worker.checkpoints[1]),
            3000,
            'compaction reset before next append'
          );
          expect(row(db)?.notes_nudge).toBeNull();
          let notebook: string | undefined;
          if (eligible) {
            const created = await f.runJsonCli<{ created: boolean; path: string }>([
              'notes',
              'path',
              '--identity',
              id,
            ]);
            expect(created).toMatchObject({ code: 0, json: { created: true } });
            notebook = created.json!.path;
            fs.writeFileSync(notebook, '# preserved durable notes\n');
          }
          fs.appendFileSync(worker.transcript, usage(provider, 9000));
          await f.waitFor(
            () => {
              const current = row(db!);
              return (
                current?.notes_nudge === (eligible ? 90 : null) &&
                JSON.parse(current!.driver_state).usage?.tokens === 9000
              );
            },
            8500,
            'new climb eligible only after fresh reported high usage'
          );
          fs.writeFileSync(worker.checkpoints[1], 'continue');
          await f.waitFor(
            () => fs.existsSync(worker.status),
            5000,
            'notes scenario exited and reaped'
          );
          expect(fs.readFileSync(worker.status, 'utf8')).toBe('0');
          const results = JSON.parse(fs.readFileSync(worker.report, 'utf8')) as Result[];
          expect(results).toHaveLength(11);
          expect(results.every((result) => result.code === 0 && result.stderr === '')).toBe(true);
          for (const index of [1, 4, 8, 10])
            expect(promptContext(results[index])).not.toContain('Context usage is');
          if (eligible) {
            expect(promptContext(results[3])).toContain(
              `Context usage is 80% of the reported window. Re-read and update your notes using tmt notes path --identity '${id}' before compaction.`
            );
            expect(promptContext(results[7])).toContain(
              `Context usage is 90% of the reported window. Re-read and update your notes at ${JSON.stringify(notebook)} before compaction.`
            );
            expect(row(db)?.notes_nudge).toBe(0);
            expect(fs.readFileSync(notebook!, 'utf8')).toBe('# preserved durable notes\n');
          } else {
            for (const index of [3, 7])
              expect(promptContext(results[index])).not.toContain('Context usage is');
            expect(row(db)?.notes_nudge).toBeNull();
            expect(fs.existsSync(path.join(f.globalDir, 'notes'))).toBe(false);
          }
          for (const result of results.filter((result) =>
            result.stdout.includes('hookSpecificOutput')
          ))
            expect(Buffer.byteLength(promptContext(result))).toBeLessThanOrEqual(4096);
        } finally {
          db?.close();
          await settled(f, worker);
        }
      },
      { mode: 'input-log' }
    );
  },
  45000
);

it('ignores retained usage after a failed read, keeps a disabled reminder, and never replays after closed output', async () => {
  await withE2EFixture(
    async (f) => {
      expect(
        (await f.runJsonCli(['config', 'set', 'notes.compactionReminder', 'false', '--global']))
          .code
      ).toBe(0);
      const setting = (value: string): Step => ({
        args: ['config', 'set', 'notes.compactionReminder', value, '--global', '--json'],
      });
      const worker = startSampler(f, 'codex', true, true, (hook, checkpoints) => [
        hook('startup'),
        hook(),
        { args: ['whoami', '--json'], checkpoint: checkpoints[0] },
        setting('true'),
        { args: ['whoami', '--json'], checkpoint: checkpoints[1] },
        hook(),
        { args: ['whoami', '--json'], checkpoint: checkpoints[2] },
        setting('false'),
        hook(),
        setting('true'),
        { ...hook(), close_stdout: true },
        hook(),
      ]);
      let db: Database.Database | undefined;
      try {
        await f.waitFor(
          () => fs.existsSync(worker.checkpoints[0]),
          10000,
          'disabled high usage checkpoint'
        );
        db = new Database(path.join(f.globalDir, 'tmux-team.db'), { readonly: true });
        fs.appendFileSync(worker.transcript, usage('codex', 8000));
        await f.waitFor(
          () => JSON.parse(row(db!)!.driver_state).usage?.tokens === 8000,
          8500,
          'high usage collected while reminders disabled'
        );
        expect(row(db)?.notes_nudge).toBeNull();
        fs.unlinkSync(worker.transcript);
        fs.writeFileSync(worker.checkpoints[0], 'continue');
        await f.waitFor(
          () => fs.existsSync(worker.checkpoints[1]),
          2000,
          'setting re-enabled with source missing'
        );
        await f.waitFor(() => row(db!)?.latest === null, 8500, 'failed source sample recorded');
        expect(JSON.parse(row(db)!.driver_state).usage.tokens).toBe(8000);
        expect(row(db)?.notes_nudge).toBeNull();
        fs.writeFileSync(worker.checkpoints[1], 'continue');
        await f.waitFor(
          () => fs.existsSync(worker.checkpoints[2]),
          2000,
          'failed-read prompt settled before restored source'
        );
        fs.writeFileSync(worker.transcript, usage('codex', 8000));
        await f.waitFor(
          () => row(db!)?.notes_nudge === 80,
          8500,
          'restored fresh source qualifies'
        );
        fs.writeFileSync(worker.checkpoints[2], 'continue');
        await f.waitFor(() => fs.existsSync(worker.status), 5000, 'toggle scenario reaped');
        expect(fs.readFileSync(worker.status, 'utf8')).toBe('0');
        const results = JSON.parse(fs.readFileSync(worker.report, 'utf8')) as Result[];
        expect(results).toHaveLength(12);
        expect(results.every((result) => result.code === 0 && result.stderr === '')).toBe(true);
        for (const index of [1, 5, 8, 11]) expect(promptContext(results[index])).toBe('');
        // The output reader was closed before stdin admitted the prompt. Its
        // committed claim survives the broken pipe and the next prompt stays quiet.
        expect(results[10].stdout).toBe('');
        expect(row(db)?.notes_nudge).toBe(0);
        expect(fs.existsSync(path.join(f.globalDir, 'notes'))).toBe(false);
      } finally {
        db?.close();
        await settled(f, worker);
      }
    },
    { mode: 'input-log' }
  );
}, 60000);
