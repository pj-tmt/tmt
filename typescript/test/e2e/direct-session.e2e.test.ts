import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import Database from 'better-sqlite3';
import { describe, expect, it } from 'vite-plus/test';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { expectJsonResult } from './cli-assertions.js';
import { withE2EFixture } from './harness.js';
import { requestAttempts } from './request-state-oracle.js';
import { waitForFileContent } from './wait-for-file.js';

function quote(value: string): string {
  return `'${value.replaceAll("'", "'\\''")}'`;
}

const capturedCodex = JSON.parse(
  readFileSync(
    new URL(
      '../../../rust/crates/tmt-adapters/src/runtime/fixtures/codex-caller-session.json',
      import.meta.url
    ),
    'utf8'
  )
) as {
  index: { columns: Array<{ name: string; type: string; notNull: number; primaryKey: number }> };
  header: {
    type: string;
    payload: { id: string; session_id: string; source: unknown; parent_thread_id?: string };
  };
};

const cases = [
  { provider: 'claude', kind: 'main' },
  { provider: 'codex', kind: 'main' },
  { provider: 'claude', kind: 'hook-primary' },
  { provider: 'codex', kind: 'hook-primary' },
  { provider: 'claude', kind: 'spoofed-pid' },
  { provider: 'codex', kind: 'held-index' },
  { provider: 'codex', kind: 'unknown-header' },
  { provider: 'codex', kind: 'subagent' },
] as const;

describe('direct provider conversation discovery', { concurrent: false }, () => {
  it.each([
    { provider: 'claude', bound: false },
    { provider: 'codex', bound: false },
    { provider: 'claude', bound: true },
    { provider: 'codex', bound: true },
  ] as const)(
    '$provider refuses an unbound provider or unrelated bound shell ($bound)',
    async ({ provider, bound }) => {
      await withE2EFixture(async (fixture) => {
        const pane = fixture.createShellPane('refused-direct').pane;
        const identity = bound
          ? expectJsonResult(
              await fixture.runJsonCli<{ id: string }>(['add', pane, 'Direct', '-s'])
            )
          : expectJsonResult(
              await fixture.runJsonCli<{ identity: { id: string } }>([
                'identity',
                'create',
                'Direct',
              ])
            ).identity;
        const home = path.join(fixture.root, 'provider-home');
        mkdirSync(home);
        const cli = path.join(fixture.wrapperDir, 'refused-cli');
        writeExecutable(
          cli,
          `#!/bin/sh\n${provider === 'claude' ? 'export CLAUDE_PID="$PPID"\n' : ''}exec ${[fixture.executables.cli.executable, ...fixture.executables.cli.args].map(quote).join(' ')} "$@"\n`,
          0o700
        );
        const scenario = path.join(fixture.root, 'scenario.json');
        const report = path.join(fixture.root, 'report.json');
        const status = path.join(fixture.root, 'provider.status');
        writeFileSync(scenario, JSON.stringify([{ args: ['config', 'show', '--json'] }]));
        const executable =
          provider === 'claude' ? '/opt/tmt-tests/claude' : '/opt/tmt-tests/hook-runtime/codex';
        const command = [
          'env',
          '-u',
          provider === 'claude' ? 'CODEX_THREAD_ID' : 'CLAUDE_CODE_SESSION_ID',
          `HOME=${home}`,
          `CODEX_HOME=${path.join(home, '.codex')}`,
          `${provider === 'claude' ? 'CLAUDE_CODE_SESSION_ID' : 'CODEX_THREAD_ID'}=01234567-89ab-7cde-8fab-0123456789ab`,
          ...(bound ? [cli, 'config', 'show', '--json'] : [executable, cli, scenario, report]),
        ]
          .map(quote)
          .join(' ');
        const output = path.join(fixture.root, 'output.json');
        const diagnostic = path.join(fixture.root, 'diagnostic');
        fixture.tmux([
          'send-keys',
          '-t',
          pane,
          '-l',
          `${command} > ${quote(output)} 2> ${quote(diagnostic)}; printf '%s' "$?" > ${quote(status)}`,
        ]);
        fixture.tmux(['send-keys', '-t', pane, 'Enter']);
        expect(
          await waitForFileContent(status, { description: 'refused optional observation exit' })
        ).toBe('0');
        if (bound) expect(() => JSON.parse(readFileSync(output, 'utf8'))).not.toThrow();
        else {
          const calls = JSON.parse(readFileSync(report, 'utf8')) as Array<{
            code: number;
            stdout: string;
          }>;
          expect(calls).toHaveLength(1);
          expect(calls[0].code).toBe(0);
          expect(() => JSON.parse(calls[0].stdout)).not.toThrow();
        }
        expect(readFileSync(diagnostic, 'utf8')).not.toContain('caller session not recorded');
        const database = new Database(path.join(fixture.globalDir, 'tmux-team.db'), {
          readonly: true,
        });
        try {
          const remembered = database
            .prepare(
              'SELECT provider_session_id FROM identity_session_preferences WHERE identity_id = ?'
            )
            .get(identity.id) as { provider_session_id: string | null } | undefined;
          expect(remembered?.provider_session_id ?? null).toBeNull();
        } finally {
          database.close();
        }
        const resume = await fixture.runCli(['resume', 'Direct'], { pane });
        expect(resume.code).toBe(1);
        expect(resume.stderr).toContain('Direct has no remembered session to resume.');
      });
    }
  );

  it('flushes the user result before an optional native probe may complete', async () => {
    await withE2EFixture(async (fixture) => {
      const pane = fixture.createShellPane('direct-output-first').pane;
      const identity = expectJsonResult(
        await fixture.runJsonCli<{ id: string }>(['add', pane, 'Direct', '-s'])
      );
      const home = path.join(fixture.root, 'provider-home');
      mkdirSync(home);
      const gate = path.join(fixture.root, 'probe-gate');
      const once = path.join(fixture.root, 'probe-once');
      const outputSeen = path.join(fixture.root, 'output-seen');
      const report = path.join(fixture.root, 'report.json');
      const status = path.join(fixture.root, 'provider.status');
      const session = '50fec1a2-581c-4adc-924d-15c31cef4da4';
      // The native probe cannot finish until the supervisor has received the
      // command's complete JSON. Recording proves admission ran after output;
      // moving admission before output spends its unchanged budget and refuses.
      fixture.tmux(['run-shell', `mkfifo ${quote(gate)}`]);
      writeExecutable(
        path.join(fixture.wrapperDir, 'ps'),
        `#!/bin/sh\nif mkdir ${quote(once)} 2>/dev/null; then read -r release < ${quote(gate)}; fi\nexec /bin/ps "$@"\n`,
        0o700
      );
      const cli = path.join(fixture.wrapperDir, 'output-cli');
      writeExecutable(
        cli,
        `#!${process.execPath}\nconst fs = require('node:fs');\nconst child = require('node:child_process').spawn(${JSON.stringify(fixture.executables.cli.executable)}, ${JSON.stringify([...fixture.executables.cli.args, 'config', 'show', '--json'])}, {env: {...process.env, CLAUDE_PID: String(process.ppid)}, stdio: ['ignore', 'pipe', 'inherit']});\nlet output = ''; let released = false;\nchild.stdout.on('data', bytes => { output += bytes.toString(); process.stdout.write(bytes); if (!released) { try { JSON.parse(output); } catch { return; } released = true; fs.writeFileSync(${JSON.stringify(outputSeen)}, 'json-before-probe'); fs.writeFile(${JSON.stringify(gate)}, 'continue\\n', error => { if (error) process.exit(1); }); }});\nchild.on('exit', code => process.exit(code ?? 1));\n`,
        0o700
      );
      const scenario = path.join(fixture.root, 'scenario.json');
      writeFileSync(scenario, JSON.stringify([{ args: [] }]));
      const command = [
        'env',
        '-u',
        'CODEX_THREAD_ID',
        `HOME=${home}`,
        `CLAUDE_CODE_SESSION_ID=${session}`,
        '/opt/tmt-tests/claude',
        cli,
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
      expect(await waitForFileContent(status, { description: 'output-first provider exit' })).toBe(
        '0'
      );
      expect(readFileSync(outputSeen, 'utf8')).toBe('json-before-probe');
      const calls = JSON.parse(readFileSync(report, 'utf8')) as Array<{
        code: number;
        stdout: string;
        stderr: string;
      }>;
      expect(calls).toHaveLength(1);
      expect(calls[0].code).toBe(0);
      expect(() => JSON.parse(calls[0].stdout)).not.toThrow();
      expect(calls[0].stderr).not.toContain('caller session not recorded');
      const database = new Database(path.join(fixture.globalDir, 'tmux-team.db'), {
        readonly: true,
      });
      try {
        expect(
          database
            .prepare(
              'SELECT provider_session_id FROM identity_session_preferences WHERE identity_id = ?'
            )
            .get(identity.id)
        ).toEqual({ provider_session_id: session });
      } finally {
        database.close();
      }
    });
  });

  it.each(cases)(
    '$provider $kind preserves successful commands and admits only exact main sessions',
    async ({ provider, kind }) => {
      await withE2EFixture(async (fixture) => {
        const pane = fixture.createShellPane(`direct-${provider}`).pane;
        const identity = expectJsonResult(
          await fixture.runJsonCli<{ id: string; name: string }>(['add', pane, 'Direct', '-s'])
        );
        const receiver = expectJsonResult(
          await fixture.runJsonCli<{ identity: { id: string } }>(['identity', 'create', 'Receiver'])
        ).identity;
        const home = path.join(fixture.root, 'provider-home');
        mkdirSync(home);
        const session = '01234567-89ab-7cde-8fab-0123456789ab';
        const hookSession = '01234567-89ab-7cde-8fab-0123456789ac';
        const expectedSession =
          kind === 'hook-primary' ? hookSession : kind === 'main' ? session : null;
        let providerDatabase: Database.Database | undefined;
        if (provider === 'codex') {
          const codexHome = path.join(home, '.codex');
          const rollout = path.join(
            codexHome,
            'sessions',
            '2026',
            '10',
            '09',
            `root-${session}.jsonl`
          );
          mkdirSync(path.dirname(rollout), { recursive: true });
          const header = structuredClone(capturedCodex.header);
          header.payload.id = session;
          header.payload.session_id = session;
          if (kind === 'unknown-header') header.type = 'new-unknown-shape';
          if (kind === 'subagent')
            header.payload.source = { subagent: { parent_thread_id: 'parent' } };
          writeFileSync(rollout, `${JSON.stringify(header)}\nignored transcript record\n`);
          providerDatabase = new Database(path.join(codexHome, 'state_5.sqlite'));
          providerDatabase.exec(
            `CREATE TABLE threads (${capturedCodex.index.columns
              .map(
                (column) =>
                  `${column.name} ${column.type} ${column.notNull ? 'NOT NULL' : ''} ${column.primaryKey ? 'PRIMARY KEY' : ''} DEFAULT ${column.type === 'TEXT' ? "''" : '0'}`
              )
              .join(',')})`
          );
          providerDatabase
            .prepare('INSERT INTO threads (id, rollout_path, source) VALUES (?, ?, ?)')
            .run(session, rollout, 'cli');
          if (kind === 'held-index') providerDatabase.exec('BEGIN EXCLUSIVE');
        }
        try {
          const scenario = path.join(fixture.root, 'scenario.json');
          const report = path.join(fixture.root, 'report.json');
          const checkpoint = path.join(fixture.root, 'calls-complete');
          const status = path.join(fixture.root, 'provider.status');
          // The existing compiled fixture retains a real provider-named native
          // parent. This wrapper reproduces the tool's inherited PID locator;
          // it execs the real CLI and never calls a lifecycle hook or tmt run.
          const cli = path.join(fixture.wrapperDir, 'direct-cli');
          writeExecutable(
            cli,
            `#!/bin/sh\n${provider === 'claude' ? `export CLAUDE_PID="${kind === 'spoofed-pid' ? '1' : '$PPID'}"\n` : ''}exec ${[fixture.executables.cli.executable, ...fixture.executables.cli.args].map(quote).join(' ')} "$@"\n`,
            0o700
          );
          writeFileSync(
            scenario,
            JSON.stringify([
              ...(kind === 'hook-primary'
                ? [
                    {
                      args: ['__hook', provider],
                      input: {
                        hook_event_name: 'SessionStart',
                        session_id: hookSession,
                        source: 'startup',
                      },
                    },
                  ]
                : []),
              { args: ['whoami', '--json'] },
              {
                args: [
                  'talk',
                  'Receiver',
                  'Direct provider request',
                  '--inbox',
                  '--detach',
                  '--json',
                ],
              },
              { checkpoint, args: ['whoami', '--json'] },
            ])
          );
          const executable =
            provider === 'claude' ? '/opt/tmt-tests/claude' : '/opt/tmt-tests/hook-runtime/codex';
          const command = [
            'env',
            '-u',
            provider === 'claude' ? 'CODEX_THREAD_ID' : 'CLAUDE_CODE_SESSION_ID',
            ...(kind === 'unknown-header' ? ['TMT_CALLER_SESSION_DEBUG=1'] : []),
            `HOME=${home}`,
            `CLAUDE_CONFIG_DIR=${path.join(home, '.claude')}`,
            `CODEX_HOME=${path.join(home, '.codex')}`,
            ...(provider === 'claude'
              ? [`CLAUDE_CODE_SESSION_ID=${session}`, 'CLAUDECODE=1']
              : [`CODEX_THREAD_ID=${session}`]),
            executable,
            cli,
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
          await waitForFileContent(checkpoint, {
            description: 'direct provider completed its CLI calls',
          });
          const database = new Database(path.join(fixture.globalDir, 'tmux-team.db'), {
            readonly: true,
          });
          try {
            const remembered = database
              .prepare(
                'SELECT provider_session_id FROM identity_session_preferences WHERE identity_id = ?'
              )
              .get(identity.id) as { provider_session_id: string | null } | undefined;
            expect(remembered?.provider_session_id ?? null).toBe(expectedSession);
            expect(requestAttempts(fixture)).toEqual([
              expect.objectContaining({
                originator_kind: 'verified',
                originator_identity_id: identity.id,
                recipient_identity_id: receiver.id,
                message_text: 'Direct provider request',
              }),
            ]);
          } finally {
            database.close();
            writeFileSync(checkpoint, 'continue');
          }
          expect(await waitForFileContent(status, { description: 'direct provider exit' })).toBe(
            '0'
          );
          const calls = JSON.parse(readFileSync(report, 'utf8')) as Array<{
            code: number;
            stdout: string;
            stderr: string;
          }>;
          expect(calls.every((call) => call.code === 0)).toBe(true);
          const ordinaryCalls = kind === 'hook-primary' ? calls.slice(1) : calls;
          expect(ordinaryCalls).toHaveLength(3);
          if (kind === 'unknown-header') {
            expect(ordinaryCalls[0].stderr).toContain(
              'caller session not recorded: provider-header-shape'
            );
          } else {
            expect(
              ordinaryCalls.every((call) => !call.stderr.includes('caller session not recorded'))
            ).toBe(true);
          }
          expect(JSON.parse(ordinaryCalls[0].stdout)).toMatchObject({
            id: identity.id,
            name: 'Direct',
          });
          expect(JSON.parse(ordinaryCalls[1].stdout)).toMatchObject({ status: 'queued' });
          if (expectedSession) {
            const resumedArgs = path.join(fixture.root, 'resume-args.json');
            writeExecutable(
              path.join(fixture.wrapperDir, provider),
              `#!${process.execPath}\nrequire('node:fs').writeFileSync(${JSON.stringify(resumedArgs)}, JSON.stringify(process.argv.slice(2)));\n`,
              0o700
            );
            const resume = await fixture.runCli(['resume', 'Direct'], { pane });
            expect(resume.code).toBe(0);
            const argv = JSON.parse(readFileSync(resumedArgs, 'utf8')) as string[];
            expect(argv.slice(0, 2)).toEqual(
              provider === 'claude' ? ['--resume', expectedSession] : ['resume', expectedSession]
            );
          } else {
            const resume = await fixture.runCli(['resume', 'Direct'], { pane });
            expect(resume.code).toBe(1);
            expect(resume.stderr).toContain('Direct has no remembered session to resume.');
          }
        } finally {
          if (providerDatabase?.inTransaction) providerDatabase.exec('ROLLBACK');
          providerDatabase?.close();
        }
      });
    }
  );
});
