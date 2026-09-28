import { describe, expect, it } from 'vitest';
import { resolveCliExecutables } from '../support/cli-executable.mjs';
import { expectJsonResult } from './cli-assertions.js';
import { withE2EFixture } from './harness.js';
import { durableState } from './identity-state-oracle.js';
import { requestAttempts } from './request-state-oracle.js';

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

function runtimeSelection(mode: 'app-server' | '--no-daemon'): NodeJS.ProcessEnv {
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

describe.sequential('runtime-owned caller attribution through real process ancestry', () => {
  it('never borrows the shared host identity, preserves anonymous sends and honors explicit identity', async () => {
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
            error: { code: string; suggestion: string };
          }>(args);
          expect(rejected.code).toBe(1);
          expect(rejected.json?.error.code).toBe('CALLER_IDENTITY_AMBIGUOUS');
          expect(rejected.json?.error.suggestion).toContain('--identity');
          expect(durableState(fixture)).toEqual(before);
          expect(fixture.paneMetadata()).toBe(metadata);
        }

        // run is deliberately not a JSON command. Its implicit caller must be
        // fenced before it can bind or execute the supplied child.
        const launch = await fixture.runCli(['run', 'Wrong', '/bin/sh', '-c', 'exit 77']);
        expect(launch.code).toBe(1);
        expect(launch.stderr).toContain('The runtime host does not establish');
        expect(launch.stderr).toContain('--identity');
        expect(durableState(fixture)).toEqual(before);
        expect(fixture.paneMetadata()).toBe(metadata);

        const anonymous = expectJsonResult(
          await fixture.runJsonCli<Queued>([
            'talk',
            'Alice',
            'Anonymous from shared host',
            '--inbox',
            '--detach',
          ])
        );
        const explicit = expectJsonResult(
          await fixture.runJsonCli<Queued>([
            'talk',
            'Host',
            'Explicit Alice from shared host',
            '--inbox',
            '--detach',
            '--identity',
            alice.id,
          ])
        );
        expect(anonymous.status).toBe('queued');
        expect(explicit.status).toBe('queued');
        const attempts = requestAttempts(fixture);
        expect(attempts).toHaveLength(2);
        expect(attempts.find((row) => row.request_id === anonymous.requestId)).toMatchObject({
          originator_kind: 'unknown',
          originator_identity_id: null,
          recipient_identity_id: alice.id,
          message_text: 'Anonymous from shared host',
        });
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

  it('keeps independent Codex caller verification and its exact durable originator', async () => {
    await withE2EFixture(
      async (fixture) => {
        const alice = expectJsonResult(await fixture.runJsonCli<Bound>(['name', 'Alice', '-s']));
        expect(expectJsonResult(await fixture.runJsonCli<Bound>(['whoami']))).toEqual(alice);
        const receiver = expectJsonResult(
          await fixture.runJsonCli<{ identity: { id: string } }>(['identity', 'create', 'Receiver'])
        ).identity;
        const queued = expectJsonResult(
          await fixture.runJsonCli<Queued>([
            'talk',
            'Receiver',
            'Independent caller',
            '--inbox',
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
        expect(durableState(fixture).identities.find((row) => row.id === alice.id)).toMatchObject({
          name: 'Alice',
          lifetime: 'saved',
          retired_at_ms: null,
        });
      },
      { mode: 'input-log', executableEnv: runtimeSelection('--no-daemon') }
    );
  });
});
