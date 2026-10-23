import fs from 'node:fs';
import path from 'node:path';
import { expect, it } from 'vite-plus/test';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { withE2EFixture, type E2EFixture } from './harness.js';

const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;
const oldSession = '11111111-1111-4111-8111-111111111111';
const newSession = '22222222-2222-4222-8222-222222222222';
type Step = { args: string[]; input?: unknown };
type Result = { code: number; stdout: string; stderr: string };

// Same private pane through the public driver protocol, with no tmux operation
// or default-server fallback in the external driver.
async function externalHost(fixture: E2EFixture, pid: number) {
  const directory = fixture.createWorkspace('hand-resume-driver');
  const driver = path.join(directory, 'tmt-driver-fake');
  const capabilities = {
    ok: {
      protocols: [1],
      kind: 'host',
      name: 'fake',
      version: '0.0.0-test',
      ops: ['caller', 'server', 'resolve-target', 'snapshot', 'publish', 'clear'],
      paneId: { prefix: 'fake-' },
      target: 'f{n}',
      callerEnv: ['FAKE_PANE_ID'],
    },
  };
  const server = { socket: fixture.socketPath, pid: fixture.serverPid, startTime: 'fixture' };
  writeExecutable(
    driver,
    `#!${process.execPath}
const fs = require('node:fs');
const path = require('node:path');
const request = JSON.parse(fs.readFileSync(0, 'utf8') || '{}');
const marker = path.join(__dirname, 'marker.json');
let answer;
switch (process.argv[4]) {
case 'capabilities': answer = ${JSON.stringify(capabilities)}; break;
case 'server': answer = {ok: {server: ${JSON.stringify(server)}}}; break;
case 'caller': answer = {ok: {pane: {id: 'fake-1', socket: ${JSON.stringify(fixture.socketPath)}, shellPid: ${pid}}}}; break;
case 'resolve-target': answer = {ok: {paneId: request.target === 'f1' ? 'fake-1' : null}}; break;
case 'snapshot': answer = {ok: {panes: [{id: 'fake-1', target: 'f1', cwd: ${JSON.stringify(fixture.workspace)}, command: 'sh', panePid: ${pid}, suggestedName: null, marker: fs.existsSync(marker) ? JSON.parse(fs.readFileSync(marker, 'utf8')) : null}]}}; break;
case 'publish': fs.writeFileSync(marker, JSON.stringify(request.marker)); answer = {ok: {}}; break;
case 'clear': fs.rmSync(marker, {force: true}); answer = {ok: {cleared: true}}; break;
default: answer = {error: {code: 'unsupported', message: 'not a fixture operation'}};
}
process.stdout.write(JSON.stringify(answer));
`,
    0o755
  );
  const installed = await fixture.runCli(['driver', 'install', driver, '--yes']);
  expect(installed.code, installed.stderr).toBe(0);
}

function row(result: Result) {
  return JSON.parse(result.stdout).identities.find(
    (item: { name: string }) => item.name === 'Hand Reader'
  );
}

for (const host of ['tmux', 'external'] as const) {
  for (const provider of ['claude', 'codex'] as const) {
    it(`${host}/${provider}: refuses a live replacement, then adopts an ended session and records its next turn`, async () => {
      await withE2EFixture(async (fixture) => {
        const created = await fixture.runJsonCli<{ identity: { id: string } }>([
          'identity',
          'create',
          'Hand Reader',
        ]);
        expect(created.code, created.stderr).toBe(0);
        const identityId = created.json!.identity.id;
        const pane = fixture.createShellPane('hand-resume');
        if (host === 'external') await externalHost(fixture, pane.pid);
        const home = fixture.createWorkspace('provider-home');
        const transcript = path.join(home, '.claude/projects/hand', `${newSession}.jsonl`);
        fs.mkdirSync(path.dirname(transcript), { recursive: true });
        const final = JSON.parse(
          fs.readFileSync(
            path.resolve(
              '../rust/crates/tmt-adapters/src/runtime/fixtures/claude-assistant-usage.jsonl'
            ),
            'utf8'
          )
        );
        final.sessionId = newSession;
        final.message.content = [{ type: 'text', text: 'hand-resumed final' }];
        fs.writeFileSync(transcript, `${JSON.stringify(final)}\n`);
        const hook = (event: string, session = oldSession, source = 'startup'): Step => ({
          args: ['__hook', provider],
          input: {
            hook_event_name: event,
            session_id: session,
            source,
            reason: 'other',
            turn_id: 'verified-turn',
            last_assistant_message: 'hand-resumed final',
            ...(provider === 'claude' ? { transcript_path: transcript } : {}),
          },
        });
        const list: Step = { args: ['ls', '--json'] };
        const recent: Step = {
          args: ['api'],
          input: { version: 1, operation: 'responses.recent', input: { identityId } },
        };
        const steps: Step[] = [
          { args: ['setup', provider, '--usage', '--yes', '--json'] },
          { args: ['name', 'Hand Reader', '-s', '--json'] },
          hook('SessionStart'),
          list,
          hook('SessionStart', newSession),
          list,
          hook('SessionEnd'),
          hook('SessionStart', newSession, 'resume'),
          list,
          hook('UserPromptSubmit', newSession),
          list,
          hook('Stop', newSession),
          list,
          recent,
        ];
        const scenario = path.join(fixture.root, 'hand-scenario.json');
        const report = path.join(fixture.root, 'hand-report.json');
        fs.writeFileSync(scenario, JSON.stringify(steps));
        const command = [
          'env',
          ...(host === 'external' ? ['-u', 'TMUX'] : []),
          `HOME=${home}`,
          `TMT_HOME=${fixture.globalDir}`,
          `CLAUDE_CONFIG_DIR=${path.join(home, '.claude')}`,
          `CODEX_HOME=${path.join(home, '.codex')}`,
          ...(host === 'external' ? ['FAKE_PANE_ID=f1'] : []),
          provider === 'claude' ? '/opt/tmt-tests/claude' : '/opt/tmt-tests/hook-runtime/codex',
          fixture.executables.cli.executable,
          scenario,
          report,
        ]
          .map(quote)
          .join(' ');
        fixture.tmux(['send-keys', '-t', pane.pane, '-l', command]);
        fixture.tmux(['send-keys', '-t', pane.pane, 'Enter']);
        await fixture.waitFor(() => fs.existsSync(report), 15000, 'hand-resume report');
        const results = JSON.parse(fs.readFileSync(report, 'utf8')) as Result[];
        expect(results).toHaveLength(steps.length);
        expect(
          results.every((result) => result.code === 0),
          JSON.stringify(results)
        ).toBe(true);
        const initial = row(results[3]);
        const refused = row(results[5]);
        const admitted = row(results[8]);
        expect(refused.id).toBe(initial.id);
        expect(refused.resume.session).toBe(oldSession);
        expect(refused.session.unbound).toBe(true);
        expect(results[4].stdout).toBe('');
        expect(admitted.id).toBe(initial.id);
        expect(admitted.pane).toBe(initial.pane);
        expect(admitted.resume.session).toBe(newSession);
        expect(admitted.session.unbound).toBe(false);
        expect(row(results[10]).session.activity.state).toBe('working');
        expect(row(results[12]).session.activity.state).toBe('idle');
        expect(JSON.parse(results[13].stdout).items).toEqual([
          expect.objectContaining({ text: 'hand-resumed final', sessionId: newSession }),
        ]);
      });
    });
  }
}

// The native parent stays the provider; its deterministic peer is a tool caller.
for (const host of ['tmux', 'external'] as const) {
  for (const provider of ['claude', 'codex'] as const) {
    for (const command of ['this', 'name', 'add'] as const) {
      it(`${host}/${provider}: ${command} completes its own session before any hook`, async () => {
        await withE2EFixture(async (fixture) => {
          const pane = fixture.createShellPane('explicit-hand-resume');
          if (host === 'external') await externalHost(fixture, pane.pid);
          const home = fixture.createWorkspace('explicit-home');
          const codexHome = path.join(home, '.codex');
          const captured = JSON.parse(
            fs.readFileSync(
              path.resolve(
                '../rust/crates/tmt-adapters/src/runtime/fixtures/codex-caller-session.json'
              ),
              'utf8'
            )
          );
          const session = captured.environment.CODEX_THREAD_ID as string;
          fs.mkdirSync(path.join(codexHome, 'sessions'), { recursive: true });
          const rollout = path.join(codexHome, 'sessions/root.jsonl');
          fs.writeFileSync(rollout, `${JSON.stringify(captured.header)}\n`);
          const { default: Database } = await import('better-sqlite3');
          const database = new Database(path.join(codexHome, 'state_5.sqlite'));
          try {
            const columns = captured.index.columns as Array<{
              name: string;
              type: string;
              notNull: number;
              primaryKey: number;
            }>;
            database.exec(
              `CREATE TABLE threads (${columns.map((column) => `${column.name} ${column.type} ${column.notNull ? 'NOT NULL' : ''} ${column.primaryKey ? 'PRIMARY KEY' : ''} DEFAULT ${column.type === 'TEXT' ? "''" : '0'}`).join(',')})`
            );
            database
              .prepare('INSERT INTO threads (id, rollout_path, source) VALUES (?, ?, ?)')
              .run(session, rollout, captured.index.row.source);
          } finally {
            database.close();
          }
          const report = path.join(fixture.root, 'explicit-report.json');
          const peer = path.join(fixture.root, 'explicit-peer.cjs');
          const target = host === 'external' ? 'f1' : pane.pane;
          const args =
            command === 'add'
              ? ['add', target, 'Hand Reader', '--json']
              : [command, 'Hand Reader', '--json'];
          fs.writeFileSync(
            peer,
            `const fs = require('node:fs');
const {spawnSync} = require('node:child_process');
const env = {...process.env, ${provider === 'claude' ? `CLAUDE_CODE_SESSION_ID: ${JSON.stringify(session)}, CLAUDE_PID: String(process.ppid)` : `CODEX_THREAD_ID: ${JSON.stringify(session)}`}};
const results = [${JSON.stringify(args)}, ['ls', '--json']].map(args => {
 const output = spawnSync(${JSON.stringify(fixture.executables.cli.executable)}, args, {env, encoding: 'utf8'});
 return {code: output.status, stdout: output.stdout, stderr: output.stderr};
});
fs.writeFileSync(${JSON.stringify(report + '.pending')}, JSON.stringify(results));
fs.renameSync(${JSON.stringify(report + '.pending')}, ${JSON.stringify(report)});
`
          );
          const launch = [
            'env',
            ...(host === 'external' ? ['-u', 'TMUX', '-u', 'TMUX_PANE'] : []),
            `HOME=${home}`,
            `TMT_HOME=${fixture.globalDir}`,
            `CODEX_HOME=${codexHome}`,
            `TMT_TEST_CLAUDE_NODE=${process.execPath}`,
            `TMT_TEST_CLAUDE_MOCK=${peer}`,
            ...(host === 'external' ? ['FAKE_PANE_ID=f1'] : []),
            provider === 'claude' ? '/opt/tmt-tests/claude' : '/opt/tmt-tests/hook-runtime/codex',
          ]
            .map(quote)
            .join(' ');
          fixture.tmux(['send-keys', '-t', pane.pane, '-l', launch]);
          fixture.tmux(['send-keys', '-t', pane.pane, 'Enter']);
          await fixture.waitFor(() => fs.existsSync(report), 15000, 'explicit provider bind');
          const results = JSON.parse(fs.readFileSync(report, 'utf8')) as Result[];
          expect(
            results.every((result) => result.code === 0),
            JSON.stringify(results)
          ).toBe(true);
          const bound = JSON.parse(results[0].stdout);
          expect(bound.session.unbound).toBe(false);
          const listed = row(results[1]);
          expect(listed.id).toBe(bound.id);
          expect(listed.resume.session).toBe(session);
          expect(listed.session.unbound).toBe(false);
        });
      });
    }
  }
}

for (const host of ['tmux', 'external'] as const) {
  for (const provider of ['claude', 'codex'] as const) {
    for (const mode of ['nested', 'pane-process'] as const) {
      it(`${host}/${provider}: ${mode} cannot supply private foreground admission`, async () => {
        await withE2EFixture(async (fixture) => {
          const pane = fixture.createShellPane('refused-hand-resume');
          if (host === 'external') await externalHost(fixture, pane.pid);
          const target = host === 'external' ? 'f1' : pane.pane;
          const bound = await fixture.runJsonCli<{ id: string }>([
            'add',
            target,
            'Hand Reader',
            '-s',
            '--json',
          ]);
          expect(bound.code, bound.stderr).toBe(0);
          const scenario = path.join(fixture.root, 'refused-scenario.json');
          const report = path.join(fixture.root, 'refused-report.json');
          fs.writeFileSync(
            scenario,
            JSON.stringify([
              {
                args: ['__hook', provider],
                input: {
                  hook_event_name: 'SessionStart',
                  session_id: newSession,
                  source: 'resume',
                },
              },
              { args: ['ls', '--json'] },
            ])
          );
          const native =
            provider === 'claude' ? '/opt/tmt-tests/claude' : '/opt/tmt-tests/hook-runtime/codex';
          const wrapper = path.join(fixture.root, 'nested-native.sh');
          fs.writeFileSync(wrapper, 'unset TMT_TEST_CLAUDE_MOCK TMT_TEST_CLAUDE_NODE\nexec "$@"\n');
          const launch = [
            'env',
            ...(host === 'external' ? ['-u', 'TMUX', 'FAKE_PANE_ID=f1'] : []),
            `TMT_HOME=${fixture.globalDir}`,
            ...(mode === 'nested'
              ? [`TMT_TEST_CLAUDE_NODE=/bin/sh`, `TMT_TEST_CLAUDE_MOCK=${wrapper}`, native]
              : []),
            native,
            fixture.executables.cli.executable,
            scenario,
            report,
          ]
            .map(quote)
            .join(' ');
          fixture.tmux([
            'send-keys',
            '-t',
            pane.pane,
            '-l',
            `${mode === 'pane-process' ? 'exec ' : ''}${launch}`,
          ]);
          fixture.tmux(['send-keys', '-t', pane.pane, 'Enter']);
          await fixture.waitFor(() => fs.existsSync(report), 15000, 'refused native start');
          const results = JSON.parse(fs.readFileSync(report, 'utf8')) as Result[];
          expect(
            results.every((result) => result.code === 0),
            JSON.stringify(results)
          ).toBe(true);
          expect(results[0].stdout).toBe('');
          const refused = row(results[1]);
          expect(refused.id).toBe(bound.json!.id);
          expect(refused.session.unbound).toBe(true);
          expect(refused.resume ?? null).toBeNull();
        });
      });
    }
  }
}

// Independent native parents under one owned pane exercise the process-loss
// fence, rather than pretending two provider session IDs are two processes.
for (const host of ['tmux', 'external'] as const) {
  for (const provider of ['claude', 'codex'] as const) {
    it(`${host}/${provider}: a stopped old incarnation stays unknown and refuses adoption`, async () => {
      await withE2EFixture(async (fixture) => {
        const pane = fixture.createShellPane('unknown-hand-resume');
        if (host === 'external') await externalHost(fixture, pane.pid);
        const checkpoint = path.join(fixture.root, 'old-checkpoint');
        const oldScenario = path.join(fixture.root, 'old-provider.json');
        const oldReport = path.join(fixture.root, 'old-report.json');
        const newScenario = path.join(fixture.root, 'new-provider.json');
        const newReport = path.join(fixture.root, 'new-report.json');
        const hook = (session: string) => ({
          args: ['__hook', provider],
          input: { hook_event_name: 'SessionStart', session_id: session, source: 'resume' },
        });
        fs.writeFileSync(
          oldScenario,
          JSON.stringify([
            { args: ['name', 'Hand Reader', '-s', '--json'] },
            hook(oldSession),
            { args: ['ls', '--json'], checkpoint },
          ])
        );
        fs.writeFileSync(
          newScenario,
          JSON.stringify([hook(newSession), { args: ['ls', '--json'] }])
        );
        const native =
          provider === 'claude' ? '/opt/tmt-tests/claude' : '/opt/tmt-tests/hook-runtime/codex';
        const launch = (scenario: string, report: string) =>
          [
            'env',
            ...(host === 'external' ? ['-u', 'TMUX', 'FAKE_PANE_ID=f1'] : []),
            `TMT_HOME=${fixture.globalDir}`,
            native,
            fixture.executables.cli.executable,
            scenario,
            report,
          ]
            .map(quote)
            .join(' ');
        fixture.tmux(['send-keys', '-t', pane.pane, '-l', `${launch(oldScenario, oldReport)} &`]);
        fixture.tmux(['send-keys', '-t', pane.pane, 'Enter']);
        await fixture.waitFor(
          () => fs.existsSync(checkpoint),
          15000,
          'old provider committed before checkpoint'
        );
        const { default: Database } = await import('better-sqlite3');
        const database = new Database(path.join(fixture.globalDir, 'tmt.db'), { readonly: true });
        let stopped = false;
        let pid = 0;
        try {
          const binding = database
            .prepare(
              "SELECT runtime_pid AS pid, observed_provider_session_id AS session FROM bindings b JOIN identities i ON i.id=b.identity_id WHERE i.name='Hand Reader'"
            )
            .get() as { pid: number; session: string };
          expect(binding.session).toBe(oldSession);
          pid = binding.pid;
          const { execFileSync } = await import('node:child_process');
          const args = execFileSync('ps', ['-p', String(pid), '-o', 'args='], { encoding: 'utf8' });
          expect(args).toContain(oldScenario);
          expect(args).toContain(native);
          process.kill(pid, 'SIGSTOP');
          stopped = true;
          await fixture.waitFor(
            () =>
              execFileSync('ps', ['-p', String(pid), '-o', 'state='], { encoding: 'utf8' })
                .trim()
                .startsWith('T'),
            5000,
            'owned provider stopped'
          );
          fixture.tmux(['send-keys', '-t', pane.pane, '-l', launch(newScenario, newReport)]);
          fixture.tmux(['send-keys', '-t', pane.pane, 'Enter']);
          await fixture.waitFor(() => fs.existsSync(newReport), 15000, 'new provider refusal');
          const results = JSON.parse(fs.readFileSync(newReport, 'utf8')) as Result[];
          expect(
            results.every((result) => result.code === 0),
            JSON.stringify(results)
          ).toBe(true);
          expect(results[0].stdout).toBe('');
          expect(row(results[1]).session.unbound).toBe(true);
          expect(row(results[1]).resume.session).toBe(oldSession);
          const refused = database
            .prepare(
              'SELECT runtime_start_refusal AS observation FROM bindings WHERE runtime_pid=?'
            )
            .get(pid) as { observation: string };
          expect(JSON.parse(refused.observation).reason).toBe('previous_session_unknown');
        } finally {
          database.close();
          if (stopped) process.kill(pid, 'SIGCONT');
          fs.writeFileSync(checkpoint, 'continue');
        }
        await fixture.waitFor(
          () => fs.existsSync(oldReport),
          15000,
          'old provider reaped normally'
        );
      });
    });
  }
}

for (const host of ['tmux', 'external'] as const) {
  for (const provider of ['claude', 'codex'] as const) {
    it(`${host}/${provider}: process loss does not allow a different driver`, async () => {
      await withE2EFixture(async (fixture) => {
        const pane = fixture.createShellPane('other-hand-resume');
        if (host === 'external') await externalHost(fixture, pane.pid);
        const firstScenario = path.join(fixture.root, 'first-provider.json');
        const firstReport = path.join(fixture.root, 'first-report.json');
        const secondScenario = path.join(fixture.root, 'second-provider.json');
        const secondReport = path.join(fixture.root, 'second-report.json');
        const other = provider === 'claude' ? 'codex' : 'claude';
        const hook = (driver: string, session: string) => ({
          args: ['__hook', driver],
          input: { hook_event_name: 'SessionStart', session_id: session, source: 'resume' },
        });
        fs.writeFileSync(
          firstScenario,
          JSON.stringify([
            { args: ['name', 'Hand Reader', '-s', '--json'] },
            hook(provider, oldSession),
          ])
        );
        fs.writeFileSync(
          secondScenario,
          JSON.stringify([hook(other, newSession), { args: ['ls', '--json'] }])
        );
        const command = (driver: string, scenario: string, report: string) =>
          [
            'env',
            ...(host === 'external' ? ['-u', 'TMUX', 'FAKE_PANE_ID=f1'] : []),
            `TMT_HOME=${fixture.globalDir}`,
            driver === 'claude' ? '/opt/tmt-tests/claude' : '/opt/tmt-tests/hook-runtime/codex',
            fixture.executables.cli.executable,
            scenario,
            report,
          ]
            .map(quote)
            .join(' ');
        fixture.tmux([
          'send-keys',
          '-t',
          pane.pane,
          '-l',
          `${command(provider, firstScenario, firstReport)} && ${command(other, secondScenario, secondReport)}`,
        ]);
        fixture.tmux(['send-keys', '-t', pane.pane, 'Enter']);
        await fixture.waitFor(
          () => fs.existsSync(secondReport),
          15000,
          'other driver refused after process loss'
        );
        const original = JSON.parse(fs.readFileSync(firstReport, 'utf8')) as Result[];
        const refused = JSON.parse(fs.readFileSync(secondReport, 'utf8')) as Result[];
        expect(
          original.every((result) => result.code === 0),
          JSON.stringify(original)
        ).toBe(true);
        expect(
          refused.every((result) => result.code === 0),
          JSON.stringify(refused)
        ).toBe(true);
        expect(refused[0].stdout).toBe('');
        expect(row(refused[1]).id).toBe(JSON.parse(original[0].stdout).id);
        expect(row(refused[1]).resume.session).toBe(oldSession);
        expect(row(refused[1]).session.unbound).toBe(true);
      });
    });
  }
}
