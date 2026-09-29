import fs from 'node:fs';
import { describe, expect, it } from 'vitest';
import { expectJsonResult, withoutAddress } from './cli-assertions.js';
import { withE2EFixture, type E2EFixture } from './harness.js';
import {
  durableIdentity,
  durableState,
  withoutVerificationTimestamp,
} from './identity-state-oracle.js';

interface DurableIdentity {
  id: string;
  name: string;
  lifetime: 'temporary' | 'saved';
}

interface BoundResult extends DurableIdentity {
  bound: true;
  pane: string;
}

interface ListedIdentity extends DurableIdentity {
  canonicalName: string;
  presence: 'active' | 'offline' | 'unknown';
  pane: string | null;
  command: string;
  target?: string;
  cwd?: string;
}

interface Listing {
  identities: ListedIdentity[];
}

function expectVerificationTimestampsNondecreasing(
  before: Array<Record<string, unknown>>,
  after: Array<Record<string, unknown>>
): void {
  const beforeById = new Map(before.map((binding) => [String(binding.id), binding]));
  for (const binding of after) {
    const prior = beforeById.get(String(binding.id));
    expect(prior).toBeDefined();
    const priorTimestamp = Date.parse(String(prior!.last_verified_at));
    const currentTimestamp = Date.parse(String(binding.last_verified_at));
    expect(Number.isNaN(priorTimestamp)).toBe(false);
    expect(Number.isNaN(currentTimestamp)).toBe(false);
    expect(currentTimestamp).toBeGreaterThanOrEqual(priorTimestamp);
  }
}

async function withTwoServers(callback: (a: E2EFixture, b: E2EFixture) => Promise<void>) {
  const servers: E2EFixture[] = [];
  try {
    await withE2EFixture(async (a) => {
      servers.push(a);
      await withE2EFixture(
        async (b) => {
          servers.push(b);
          expect(a.pane).toBe(b.pane);
          expect(a.socketPath).not.toBe(b.socketPath);
          expect(a.serverPid).not.toBe(b.serverPid);
          await callback(a, b);
        },
        { globalDir: a.globalDir }
      );
    });
  } finally {
    for (const server of servers) {
      await server.waitFor(
        () => !server.serverProcessIsRunning() && !server.mockProcessIsRunning(),
        2_000,
        'both private server and mock process cleanup'
      );
      expect(fs.existsSync(server.root)).toBe(false);
      expect(fs.existsSync(server.socketRoot)).toBe(false);
    }
  }
}

describe.sequential('global identities across isolated tmux servers', () => {
  it('cleans both private servers and their shared storage when a scenario throws', async () => {
    await expect(
      withTwoServers(async (a, b) => {
        expectJsonResult(await a.runJsonCli(['name', 'Local']));
        expectJsonResult(await b.runJsonCli(['name', 'Remote']));
        throw new Error('intentional multi-server scenario failure');
      })
    ).rejects.toThrow('intentional multi-server scenario failure');
  });

  it('preserves grouped foreign bindings and refuses a live name collision despite identical pane IDs', async () => {
    await withTwoServers(async (a, b) => {
      b.tmux(['new-session', '-d', '-t', 'e2e', '-s', 'grouped']);
      const remoteRows = b.tmux(['list-panes', '-a', '-F', '#{pane_id}']).trim().split('\n');
      expect(remoteRows.filter((pane) => pane === b.pane)).toHaveLength(2);
      expectJsonResult(await b.runJsonCli(['name', 'Remote']));
      expectJsonResult(await b.runJsonCli(['role', 'set', 'Keep the remote profile.']));
      const before = durableState(b);
      const remoteIdentity = durableIdentity(b, 'Remote');
      const remoteMetadata = b.paneMetadata();
      // Neither grouped session is attached: inventory retains the first linked
      // presentation, while display-message may choose the other session.
      const remoteTarget = b
        .tmux([
          'list-panes',
          '-a',
          '-F',
          '#{pane_id}|#{session_name}:#{window_index}.#{pane_index}',
        ])
        .trim()
        .split('\n')
        .find((row) => row.startsWith(`${b.pane}|`))!
        .split('|')[1];

      expect(expectJsonResult(withoutAddress(await a.runJsonCli<Listing>(['list'])))).toEqual({
        identities: [
          {
            id: remoteIdentity.id,
            name: 'Remote',
            canonicalName: 'remote',
            lifetime: 'temporary',
            presence: 'active',
            pane: b.pane,
            command: 'node',
            target: remoteTarget,
            cwd: b.workspace,
          },
        ],
      });
      const afterList = durableState(a);
      expect(afterList.identities).toEqual(before.identities);
      expect(withoutVerificationTimestamp(afterList.bindings)).toEqual(
        withoutVerificationTimestamp(before.bindings)
      );
      expectVerificationTimestampsNondecreasing(before.bindings, afterList.bindings);
      expect(afterList.profiles).toEqual(before.profiles);
      const collision = await a.runJsonCli(['name', 'REMOTE']);
      expect(collision.code).toBe(5);
      expect(collision.json).toMatchObject({ error: { code: 'NAME_ALREADY_ACTIVE' } });
      const afterCollision = durableState(a);
      expect(afterCollision.identities).toEqual(before.identities);
      expect(withoutVerificationTimestamp(afterCollision.bindings)).toEqual(
        withoutVerificationTimestamp(before.bindings)
      );
      expectVerificationTimestampsNondecreasing(before.bindings, afterCollision.bindings);
      expect(afterCollision.profiles).toEqual(before.profiles);
      expect(a.paneMetadata()).toBe('');
      expect(b.paneMetadata()).toBe(remoteMetadata);

      expectJsonResult(await a.runJsonCli(['name', 'Local']));
      const localIdentity = durableIdentity(a, 'Local');
      const local = durableState(a).bindings.filter(
        (row) => (row as { socket_path: string }).socket_path === a.socketPath
      );
      expect(
        durableState(a).bindings.filter(
          (row) => (row as { socket_path: string }).socket_path === a.socketPath
        )
      ).toEqual(local);

      const listB = expectJsonResult(withoutAddress(await b.runJsonCli<Listing>(['list'])));
      expect(listB).toEqual({
        identities: [
          {
            id: localIdentity.id,
            name: 'Local',
            canonicalName: 'local',
            lifetime: 'temporary',
            presence: 'active',
            pane: a.pane,
            command: 'node',
            target: a.paneTarget(a.pane),
            cwd: a.workspace,
          },
          {
            id: remoteIdentity.id,
            name: 'Remote',
            canonicalName: 'remote',
            lifetime: 'temporary',
            presence: 'active',
            pane: b.pane,
            command: 'node',
            target: remoteTarget,
            cwd: b.workspace,
          },
        ],
      });

      const foreignTalk = await a.runJsonCli(['talk', 'Remote', 'must-not-cross-servers']);
      expect(foreignTalk.code).toBe(3);
      expect(foreignTalk.json).toMatchObject({ error: { code: 'NAME_NOT_FOUND' } });
      const message = 'local-server-only';
      const talk = expectJsonResult(
        await a.runJsonCli<{ status: string; response: string }>([
          'talk',
          'Local',
          message,
          '--timeout',
          '10',
        ])
      );
      expect(talk.status).toBe('completed');
      expect(talk.response).toContain(`mock-agent response: ${message}`);
      await a.waitForEvent(
        (event) =>
          event.event === 'submitted' && event.pid === a.panePid && event.message === message
      );
      expect(b.events().filter((event) => event.event === 'request')).toEqual([]);
    });
  }, 30_000);

  it.each(['hidden socket', 'suspended server'] as const)(
    'fails closed with bounded probing for a %s while its process remains alive',
    async (failure) => {
      await withTwoServers(async (a, b) => {
        expectJsonResult(await b.runJsonCli(['name', 'Remote']));
        expectJsonResult(await b.runJsonCli(['role', 'set', 'Preserve on uncertain evidence.']));
        const before = durableState(b);
        const remoteIdentity = durableIdentity(b, 'Remote');
        const metadata = b.paneMetadata();
        const hiddenSocket = `${b.socketPath}.unreachable`;
        if (failure === 'hidden socket') fs.renameSync(b.socketPath, hiddenSocket);
        else process.kill(b.serverPid, 'SIGSTOP');
        try {
          expect(b.serverProcessIsRunning()).toBe(true);
          expect(expectJsonResult(withoutAddress(await a.runJsonCli<Listing>(['list'])))).toEqual({
            identities: [
              {
                id: remoteIdentity.id,
                name: 'Remote',
                canonicalName: 'remote',
                lifetime: 'temporary',
                presence: 'unknown',
                pane: null,
                command: '',
              },
            ],
          });
          expect(expectJsonResult(await a.runJsonCli(['whoami']))).toMatchObject({ bound: false });
          expect(durableState(a)).toEqual(before);
          const started = Date.now();
          const result = await a.runJsonCli(['name', 'Remote']);
          expect(Date.now() - started).toBeLessThan(5_000);
          expect(result.code).toBe(1);
          expect(result.json).toMatchObject({ error: { code: 'RECONCILIATION_FAILED' } });
          expect(durableState(a)).toEqual(before);
          expect(a.paneMetadata()).toBe('');
        } finally {
          if (failure === 'hidden socket') fs.renameSync(hiddenSocket, b.socketPath);
          else process.kill(b.serverPid, 'SIGCONT');
        }
        expect(b.paneMetadata()).toBe(metadata);
        expect(expectJsonResult(await b.runJsonCli(['whoami']))).toEqual({
          interfaceKind: 'container',
          sessionState: 'unknown',
          bound: true,
          id: remoteIdentity.id,
          name: 'Remote',
          pane: b.pane,
          lifetime: 'temporary',
        });
      });
    },
    30_000
  );

  it('reclaims a proven-dead foreign endpoint without replacing the identity or profile', async () => {
    await withTwoServers(async (a, b) => {
      expectJsonResult(await b.runJsonCli(['name', 'Remote', '-s']));
      const profile = expectJsonResult(
        await b.runJsonCli(['role', 'set', 'Survive endpoint death.'])
      );
      const before = durableState(b);
      const remoteIdentity = durableIdentity(b, 'Remote');
      expect(remoteIdentity.lifetime).toBe('saved');
      b.tmux(['kill-server']);
      await b.waitFor(() => !b.serverProcessIsRunning(), 2_000, 'foreign server process exit');
      expect(expectJsonResult(await a.runJsonCli<BoundResult>(['name', 'Remote']))).toEqual({
        bound: true,
        id: remoteIdentity.id,
        name: 'Remote',
        pane: a.pane,
        lifetime: 'saved',
      });
      const after = durableState(a);
      expect(after.identities).toEqual(before.identities);
      expect(after.profiles).toEqual(before.profiles);
      expect(after.bindings).toHaveLength(1);
      expect(after.bindings[0]).toMatchObject({ socket_path: a.socketPath });
      expect(after.bindings).not.toEqual(before.bindings);
      expect(expectJsonResult(await a.runJsonCli(['role', 'show']))).toEqual(profile);
    });
  }, 30_000);

  it('prunes only the restarted socket while retaining another live server and both durable identities', async () => {
    await withTwoServers(async (a, b) => {
      expectJsonResult(await a.runJsonCli(['name', 'Local', '-s']));
      expectJsonResult(await a.runJsonCli(['role', 'set', 'Survive local restart.']));
      expectJsonResult(await b.runJsonCli(['name', 'Remote']));
      const before = durableState(a);
      const localIdentity = durableIdentity(a, 'Local');
      const remoteIdentity = durableIdentity(a, 'Remote');
      expect(localIdentity.lifetime).toBe('saved');
      const foreign = before.bindings.filter(
        (row) => (row as { socket_path: string }).socket_path === b.socketPath
      );
      expect(foreign).toHaveLength(1);
      const oldSocket = a.socketPath;
      await a.restartServer();
      expect(a.socketPath).toBe(oldSocket);
      expect(expectJsonResult(withoutAddress(await a.runJsonCli<Listing>(['list'])))).toEqual({
        identities: [
          {
            id: localIdentity.id,
            name: 'Local',
            canonicalName: 'local',
            lifetime: 'saved',
            presence: 'offline',
            pane: null,
            command: '',
          },
          {
            id: remoteIdentity.id,
            name: 'Remote',
            canonicalName: 'remote',
            lifetime: 'temporary',
            presence: 'active',
            pane: b.pane,
            command: 'node',
            target: b.paneTarget(b.pane),
            cwd: b.workspace,
          },
        ],
      });
      const after = durableState(a);
      expect(withoutVerificationTimestamp(after.bindings)).toEqual(
        withoutVerificationTimestamp(foreign)
      );
      expectVerificationTimestampsNondecreasing(foreign, after.bindings);
      expect(after.identities).toEqual(before.identities);
      expect(after.profiles).toEqual(before.profiles);
      expect(expectJsonResult(await a.runJsonCli<BoundResult>(['name', 'Local']))).toEqual({
        bound: true,
        id: localIdentity.id,
        name: 'Local',
        pane: a.pane,
        lifetime: 'saved',
      });
      expect(durableState(a).identities).toEqual(before.identities);
      expect(expectJsonResult(await b.runJsonCli(['whoami']))).toEqual({
        interfaceKind: 'container',
        sessionState: 'unknown',
        bound: true,
        id: remoteIdentity.id,
        name: 'Remote',
        pane: b.pane,
        lifetime: 'temporary',
      });
    });
  }, 30_000);

  it('reclaims a dead foreign pane while its original server and another identity remain alive', async () => {
    await withTwoServers(async (a, b) => {
      expectJsonResult(await b.runJsonCli(['name', 'Remote', '-s']));
      const profile = expectJsonResult(await b.runJsonCli(['role', 'set', 'Survive pane death.']));
      const peer = await b.createMockPane('survivor');
      expectJsonResult(await b.runJsonCli(['add', peer.pane, 'Survivor']));
      const before = durableState(b);
      const remoteIdentity = durableIdentity(b, 'Remote');
      const survivorIdentity = durableIdentity(b, 'Survivor');
      expect(remoteIdentity.lifetime).toBe('saved');
      const survivor = before.bindings.filter(
        (row) => (row as { pane_id: string }).pane_id === peer.pane
      );
      expect(survivor).toHaveLength(1);
      b.tmux(['kill-pane', '-t', b.pane]);
      await b.waitFor(() => !b.mockProcessIsRunning(), 2_000, 'foreign pane process exit');
      expect(b.serverProcessIsRunning()).toBe(true);

      expect(expectJsonResult(withoutAddress(await a.runJsonCli<Listing>(['list'])))).toEqual({
        identities: [
          {
            id: remoteIdentity.id,
            name: 'Remote',
            canonicalName: 'remote',
            lifetime: 'saved',
            presence: 'offline',
            pane: null,
            command: '',
          },
          {
            id: survivorIdentity.id,
            name: 'Survivor',
            canonicalName: 'survivor',
            lifetime: 'temporary',
            presence: 'active',
            pane: peer.pane,
            command: 'node',
            target: b.paneTarget(peer.pane),
            cwd: peer.workspace,
          },
        ],
      });
      const observed = durableState(a);
      expect(observed.identities).toEqual(before.identities);
      expect(observed.profiles).toEqual(before.profiles);
      const observedSurvivor = observed.bindings.filter(
        (binding) => binding.identity_id === survivorIdentity.id
      );
      expect(observedSurvivor).toHaveLength(1);
      expect(withoutVerificationTimestamp(observedSurvivor)).toEqual(
        withoutVerificationTimestamp(survivor)
      );
      expectVerificationTimestampsNondecreasing(survivor, observedSurvivor);
      expect(expectJsonResult(await a.runJsonCli<BoundResult>(['name', 'Remote']))).toEqual({
        bound: true,
        id: remoteIdentity.id,
        name: 'Remote',
        pane: a.pane,
        lifetime: 'saved',
      });
      const after = durableState(a);
      expect(after.identities).toEqual(before.identities);
      expect(after.profiles).toEqual(before.profiles);
      expect(after.bindings).toHaveLength(2);
      const afterSurvivor = after.bindings.filter(
        (binding) => binding.identity_id === survivorIdentity.id
      );
      expect(afterSurvivor).toHaveLength(1);
      expect(withoutVerificationTimestamp(afterSurvivor)).toEqual(
        withoutVerificationTimestamp(survivor)
      );
      expectVerificationTimestampsNondecreasing(survivor, afterSurvivor);
      expect(expectJsonResult(await a.runJsonCli(['role', 'show']))).toEqual(profile);
      expect(expectJsonResult(await b.runJsonCli(['whoami'], { pane: peer.pane }))).toEqual({
        interfaceKind: 'container',
        sessionState: 'unknown',
        bound: true,
        id: survivorIdentity.id,
        name: 'Survivor',
        pane: peer.pane,
        lifetime: 'temporary',
      });
    });
  }, 30_000);
});
