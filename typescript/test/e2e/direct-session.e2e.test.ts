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

describe('direct provider conversation discovery', { concurrent: false }, () => {
  it.each(['claude', 'codex'] as const)(
    'reproduces missing remembered session after direct %s calls without hooks',
    async (provider) => {
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
        const session = '12345678-1234-4234-8234-123456789abc';
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
          `#!/bin/sh\n${provider === 'claude' ? 'export CLAUDE_PID="$PPID"\n' : ''}exec ${[fixture.executables.cli.executable, ...fixture.executables.cli.args].map(quote).join(' ')} "$@"\n`,
          0o700
        );
        writeFileSync(
          scenario,
          JSON.stringify([
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
          expect(remembered?.provider_session_id ?? null).toBeNull();
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
        expect(await waitForFileContent(status, { description: 'direct provider exit' })).toBe('0');
        const calls = JSON.parse(readFileSync(report, 'utf8')) as Array<{
          code: number;
          stdout: string;
          stderr: string;
        }>;
        expect(calls.map((call) => call.code)).toEqual([0, 0, 0]);
        expect(JSON.parse(calls[0].stdout)).toMatchObject({ id: identity.id, name: 'Direct' });
        expect(JSON.parse(calls[1].stdout)).toMatchObject({ status: 'queued' });
        const resume = await fixture.runCli(['resume', 'Direct'], { pane });
        expect(resume.code).toBe(1);
        expect(resume.stderr).toContain('Direct has no remembered session to resume.');
        console.info(
          `REPRO #2131 ${provider}: verified sender, 3 successful CLI calls, remembered session absent, resume exit 1: no remembered session`
        );
      });
    }
  );
});
