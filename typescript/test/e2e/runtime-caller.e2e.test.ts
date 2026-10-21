import { describe, expect, it } from 'vite-plus/test';
import { existsSync, mkdirSync, readFileSync } from 'node:fs';
import path from 'node:path';
import Database from 'better-sqlite3';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { resolveCliExecutables } from '../support/cli-executable.mjs';
import { expectJsonResult } from './cli-assertions.js';
import { withE2EFixture } from './harness.js';
import { durableState } from './identity-state-oracle.js';
import { requestAttempts } from './request-state-oracle.js';
import { waitForFileContent } from './wait-for-file.js';

interface Bound {
  id: string;
  name: string;
  bound: boolean;
  pane: string;
}

interface Queued {
  requestId: string;
  status: string;
}

function runtimeSelection(mode: 'app-server' | '--no-daemon' | 'orphan'): NodeJS.ProcessEnv {
  const selected = resolveCliExecutables();
  return {
    ...process.env,
    TMT_TEST_CLI: JSON.stringify({
      executable: '/opt/tmt-tests/codex',
      args: [mode, selected.cli.executable, ...selected.cli.args],
    }),
    TMT_TEST_PEER_CLI: JSON.stringify(selected.peer),
  };
}

describe(
  'runtime-owned caller attribution through real process ancestry',
  { concurrent: false },
  () => {
    it('launches and resumes the managed provider embedded without changing provider configuration', async () => {
      await withE2EFixture(async (fixture) => {
        const pane = fixture.createShellPane('managed-runtime').pane;
        const callsFile = path.join(fixture.root, 'managed-calls.jsonl');
        const fake = path.join(fixture.wrapperDir, 'codex');
        writeExecutable(
          fake,
          `#!${process.execPath}\nrequire('node:fs').appendFileSync(${JSON.stringify(callsFile)}, JSON.stringify(process.argv.slice(2)) + String.fromCharCode(10));\n`,
          0o700
        );
        const home = path.join(fixture.root, 'managed-home');
        mkdirSync(home);
        const quote = (word: string) => `'${word.replaceAll("'", "'\\''")}'`;
        const launch = async (label: string, args: string[]) => {
          const status = path.join(fixture.root, `${label}.status`);
          const command = [
            'env',
            `HOME=${home}`,
            `CODEX_HOME=${path.join(home, '.codex')}`,
            fixture.executables.cli.executable,
            ...fixture.executables.cli.args,
            ...args,
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
          expect(
            await waitForFileContent(status, { description: `${label} launch completed` })
          ).toBe('0');
        };
        await launch('fresh', [
          'run',
          '--no-channel',
          '-s',
          'Managed',
          fake,
          '--model',
          'fixture-model',
        ]);
        const session = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa';
        const db = new Database(path.join(fixture.globalDir, 'tmt.db'));
        try {
          expect(
            db
              .prepare(
                `UPDATE identity_session_preferences SET preferred_harness = 'codex', remembered_harness = 'codex', runtime_mode = 'shared', provider_session_id = ?, stale_at_ms = NULL, resume_pending_at_ms = NULL WHERE identity_id = (SELECT id FROM identities WHERE canonical_name = 'managed')`
              )
              .run(session).changes
          ).toBe(1);
        } finally {
          db.close();
        }
        await launch('resume', ['resume', '--no-channel', 'Managed']);
        const calls = readFileSync(callsFile, 'utf8')
          .trim()
          .split('\n')
          .map((line) => JSON.parse(line) as string[]);
        expect(calls).toHaveLength(2);
        for (const args of calls) {
          expect(args.filter((word) => word === '--no-daemon')).toHaveLength(1);
          expect(args[args.indexOf('--add-dir') + 1]).toBe(fixture.globalDir);
        }
        expect(calls[0]).toContain('fixture-model');
        expect(calls[1][0]).toBe('resume');
        expect(calls[1]).toContain(session);
        expect(existsSync(path.join(home, '.codex/config.toml'))).toBe(false);
        expect(existsSync(path.join(home, '.codex/hooks.json'))).toBe(false);
      });
    });

    it('explains the shared runtime refusal without anonymous delivery and honors explicit identity', async () => {
      await withE2EFixture(
        async (fixture) => {
          const client = await fixture.createMockPane('client');
          const host = expectJsonResult(
            await fixture.runJsonCli<Bound>(['add', fixture.pane, 'Host', '-s'])
          );
          const alice = expectJsonResult(
            await fixture.runJsonCli<Bound>(['add', client.pane, 'Alice', '-s'])
          );
          expect(host.id).not.toBe(alice.id);
          const before = durableState(fixture);
          const metadata = fixture.paneMetadata();
          const context = await fixture.runJsonCli(['whoami', '--context']);
          expect(context).toMatchObject({ code: 0, json: { bound: false, status: 'unavailable' } });
          const humanContext = await fixture.runCli(['whoami', '--context']);
          expect(humanContext).toMatchObject({ code: 0, stdout: '', stderr: '' });
          expect(context.stdout).not.toContain(host.id);
          expect(durableState(fixture)).toEqual(before);
          expect(fixture.paneMetadata()).toBe(metadata);
          // The real fixture process is named codex and retains the host pane's
          // environment, while the calling conversation cannot be inferred from it.
          // No real provider, credentials or external model is involved.
          for (const args of [
            ['whoami'],
            ['name', 'Wrong'],
            ['unbind'],
            ['role', 'set', 'wrong owner'],
          ]) {
            const rejected = await fixture.runJsonCli<{
              error: { code: string; reason: string; fix: string };
            }>(args);
            expect(rejected.code).toBe(1);
            expect(rejected.json?.error.code).toBe('CALLER_IDENTITY_AMBIGUOUS');
            expect(rejected.json?.error.reason).toBe('codex-shared-server');
            expect(rejected.json?.error.fix).toBe(
              args[0] === 'name' ? 'tmt run Wrong codex' : 'tmt run <name> codex'
            );
            expect(rejected.stdout).not.toContain('--identity');
            expect(durableState(fixture)).toEqual(before);
            expect(fixture.paneMetadata()).toBe(metadata);
          }

          // run is deliberately not a JSON command. Its implicit caller must be
          // fenced before it can bind or execute the supplied child.
          const launch = await fixture.runCli(['run', 'Wrong', '/bin/sh', '-c', 'exit 77']);
          expect(launch.code).toBe(1);
          expect(launch.stderr).toContain("Codex's shared background server");
          expect(launch.stderr).toContain(
            'fix: Exit this Codex, then start it with:  tmt run Wrong codex'
          );
          expect(launch.stderr).not.toContain('--identity');
          expect(durableState(fixture)).toEqual(before);
          expect(fixture.paneMetadata()).toBe(metadata);

          const anonymousResult = await fixture.runJsonCli<{
            error: { code: string; reason: string; fix: string };
          }>(['talk', 'Alice', 'Anonymous from shared host', '--detach']);
          expect(anonymousResult.code).toBe(1);
          expect(anonymousResult.json?.error).toMatchObject({
            code: 'CALLER_IDENTITY_AMBIGUOUS',
            reason: 'codex-shared-server',
            fix: 'tmt run <name> codex',
          });
          expect(anonymousResult.stderr).toBe('');
          expect(requestAttempts(fixture)).toEqual([]);
          const human = await fixture.runCli(['whoami']);
          expect(human.code).toBe(1);
          expect(human.stderr).toContain('cannot name itself, send as itself, or reply');
          expect(human.stderr).toContain(
            'fix: Exit this Codex, then start it with:  tmt run <name> codex'
          );
          const explicit = expectJsonResult(
            await fixture.runJsonCli<Queued>([
              'talk',
              'Host',
              'Explicit Alice from shared host',
              '--detach',
              '--identity',
              alice.id,
            ])
          );
          expect(explicit.status).toBe('queued');
          const attempts = requestAttempts(fixture);
          expect(attempts).toHaveLength(1);
          expect(attempts.find((row) => row.request_id === explicit.requestId)).toMatchObject({
            originator_kind: 'explicit',
            originator_identity_id: alice.id,
            recipient_identity_id: host.id,
            message_text: 'Explicit Alice from shared host',
          });
          expect(attempts.some((row) => row.originator_identity_id === host.id)).toBe(false);
          expect(durableState(fixture)).toEqual(before);
          expect(fixture.events().filter((event) => event.event === 'request')).toEqual([]);
        },
        { mode: 'input-log', executableEnv: runtimeSelection('app-server') }
      );
    });

    it.each(['--no-daemon', 'orphan'] as const)(
      'keeps %s caller verification and its exact durable originator without notices',
      async (mode) => {
        await withE2EFixture(
          async (fixture) => {
            const alice = expectJsonResult(
              await fixture.runJsonCli<Bound>(['name', 'Alice', '-s'])
            );
            expect(expectJsonResult(await fixture.runJsonCli<Bound>(['whoami']))).toEqual({
              ...alice,
              interfaceKind: 'container',
              sessionState: 'unknown',
            });
            expect(await fixture.runJsonCli(['whoami', '--context'])).toMatchObject({
              code: 0,
              json: { bound: true, id: alice.id, name: 'Alice' },
            });
            const receiver = expectJsonResult(
              await fixture.runJsonCli<{ identity: { id: string } }>([
                'identity',
                'create',
                'Receiver',
              ])
            ).identity;
            const queued = expectJsonResult(
              await fixture.runJsonCli<Queued>([
                'talk',
                'Receiver',
                'Independent caller',
                '--detach',
              ])
            );
            expect(queued.status).toBe('queued');
            expect(requestAttempts(fixture)).toEqual([
              expect.objectContaining({
                request_id: queued.requestId,
                originator_kind: 'verified',
                originator_identity_id: alice.id,
                recipient_identity_id: receiver.id,
                message_text: 'Independent caller',
              }),
            ]);
            const launch = await fixture.runCli(['run', 'Alice', '/bin/sh', '-c', 'exit 17']);
            expect(launch.code, launch.stderr).toBe(17);
            expect(
              durableState(fixture).identities.find((row) => row.id === alice.id)
            ).toMatchObject({
              name: 'Alice',
              lifetime: 'saved',
              retired_at_ms: null,
            });
          },
          { mode: 'input-log', executableEnv: runtimeSelection(mode) }
        );
      }
    );
  }
);
