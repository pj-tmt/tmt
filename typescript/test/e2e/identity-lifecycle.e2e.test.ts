import { unknownActivity, withoutAddress } from './cli-assertions.js';
import { describe, expect, it } from 'vite-plus/test';
import Database from 'better-sqlite3';
import fs from 'node:fs';
import path from 'node:path';
import { E2EFixture, withE2EFixture, type CliResult, type MockPane } from './harness.js';
import {
  durableIdentity,
  durableState,
  withoutVerificationTimestamp,
} from './identity-state-oracle.js';

interface Identity {
  name: string;
  canonicalName: string;
}

interface IdentityListItem extends Identity {
  pane: string;
  target: string;
  cwd: string;
  command: string;
}

interface CommandError {
  error: { code: string; message: string; suggestion?: string };
}

function json<T>(result: CliResult<T>): T {
  expect(result.stdout.trim()).not.toBe('');
  expect(() => JSON.parse(result.stdout)).not.toThrow();
  expect(result.stderr).toBe('');
  expect(result.json).toBeDefined();
  return result.json as T;
}

async function listIdentities(fixture: E2EFixture): Promise<IdentityListItem[]> {
  const result = withoutAddress(
    await fixture.runJsonCli<{ identities: IdentityListItem[] }>(['list'])
  );
  expect(result.code).toBe(0);
  return json(result).identities;
}

function boundProjection(fixture: E2EFixture, name: string, pane: string, lifetime = 'temporary') {
  const row = durableIdentity(fixture, name);
  expect(row.id).toMatch(/^[0-9a-f-]{36}$/);
  expect(row.lifetime).toBe(lifetime);
  return { bound: true, id: row.id, name, pane, lifetime };
}

function metadata(fixture: E2EFixture, pane: MockPane): Record<string, unknown> {
  return JSON.parse(fixture.paneMetadata(pane.pane)) as Record<string, unknown>;
}

function eventCount(
  fixture: E2EFixture,
  event: 'request' | 'submitted',
  pid: number,
  message: string
): number {
  return fixture
    .events()
    .filter((entry) => entry.event === event && entry.pid === pid && entry.message === message)
    .length;
}

async function talkToIdentity(
  fixture: E2EFixture,
  name: string,
  pid: number,
  otherPids: number[],
  message: string
): Promise<void> {
  const before = fixture.events();
  const result = await fixture.runJsonCli<{
    target: string;
    pane: string;
    identity: Identity;
    status: string;
    response: string;
  }>(['talk', name, message, '--timeout', '10']);
  expect(result.code).toBe(0);
  const output = json(result);
  expect(output).toMatchObject({ target: name, identity: { name }, status: 'completed' });
  expect(output.response).toContain(`mock-agent response: ${message}`);
  await fixture.waitForEvent(
    (entry) => entry.event === 'submitted' && entry.pid === pid && entry.message === message
  );

  const after = fixture.events();
  const beforeRequest = before.filter(
    (entry) => entry.event === 'request' && entry.pid === pid && entry.message === message
  ).length;
  const beforeSubmitted = before.filter(
    (entry) => entry.event === 'submitted' && entry.pid === pid && entry.message === message
  ).length;
  expect(eventCount(fixture, 'request', pid, message) - beforeRequest).toBe(1);
  expect(eventCount(fixture, 'submitted', pid, message) - beforeSubmitted).toBe(1);
  for (const otherPid of otherPids) {
    expect(
      after.filter(
        (entry) =>
          (entry.event === 'request' || entry.event === 'submitted') &&
          entry.pid === otherPid &&
          entry.message === message
      )
    ).toHaveLength(0);
  }
}

describe('global identity lifecycle', { concurrent: false }, () => {
  it('runs the real name/this/whoami/unbind lifecycle with human and JSON contracts', async () => {
    await withE2EFixture(async (fixture) => {
      const pane = fixture.pane;
      const peer = await fixture.createMockPane('peer');
      const peerBinding = await fixture.runJsonCli(['add', peer.pane, 'Peer']);
      expect(peerBinding.code).toBe(0);
      const peerMetadata = fixture.paneMetadata(peer.pane);
      const peerList = await listIdentities(fixture);

      const initial = await fixture.runJsonCli<{ bound: false; pane: string }>(['whoami']);
      expect(initial.code).toBe(0);
      expect(json(initial)).toEqual({
        bound: false,
        pane,
        interfaceKind: 'container',
        sessionState: 'unknown',
      });

      const initialHuman = await fixture.runCli(['whoami']);
      expect(initialHuman.code).toBe(0);
      expect(initialHuman.stdout).toContain(`Pane ${pane} is unbound`);
      expect(initialHuman.stderr).toBe('');

      const namedJson = await fixture.runJsonCli<{ bound: true; name: string; pane: string }>([
        'name',
        'Lifecycle',
      ]);
      expect(namedJson.code).toBe(0);
      const initialBinding = boundProjection(fixture, 'Lifecycle', pane);
      expect(json(namedJson)).toEqual(initialBinding);

      const namedHuman = await fixture.runCli(['name', 'Lifecycle']);
      expect(namedHuman.code).toBe(0);
      expect(namedHuman.stdout).toContain(`Bound temporary identity 'Lifecycle' on pane ${pane}`);
      expect(namedHuman.stderr).toBe('');

      const whoamiHuman = await fixture.runCli(['whoami']);
      expect(whoamiHuman.code).toBe(0);
      expect(whoamiHuman.stdout).toBe(`Lifecycle (temporary) on pane ${pane}\n`);
      expect(whoamiHuman.stderr).toBe('');

      const whoamiJson = await fixture.runJsonCli<{ bound: true; name: string; pane: string }>([
        'whoami',
      ]);
      expect(whoamiJson.code).toBe(0);
      expect(json(whoamiJson)).toEqual({
        ...initialBinding,
        interfaceKind: 'container',
        sessionState: 'unknown',
      });

      const unboundJson = await fixture.runJsonCli<{ unbound: true; name: string; pane: string }>([
        'unbind',
      ]);
      expect(unboundJson.code).toBe(0);
      expect(json(unboundJson)).toEqual({
        unbound: true,
        id: initialBinding.id,
        name: 'Lifecycle',
        pane,
        lifetime: 'temporary',
        retired: true,
      });

      const rebound = await fixture.runJsonCli<{ bound: true; name: string; pane: string }>([
        'this',
        'Lifecycle',
      ]);
      expect(rebound.code).toBe(0);
      const reboundBinding = boundProjection(fixture, 'Lifecycle', pane);
      expect(json(rebound)).toEqual(reboundBinding);
      expect(reboundBinding.id).not.toBe(initialBinding.id);
      expect(rebound.stderr).toBe('');

      const reboundHuman = await fixture.runCli(['this', 'Lifecycle']);
      expect(reboundHuman.code).toBe(0);
      expect(reboundHuman.stdout).toContain(`Bound temporary identity 'Lifecycle' on pane ${pane}`);
      expect(reboundHuman.stderr).toBe('');

      const beforeConflictList = await listIdentities(fixture);
      const beforeConflictMetadata = fixture.paneMetadata(pane);
      const nameConflict = await fixture.runJsonCli<CommandError>(['name', 'Conflict']);
      const thisConflict = await fixture.runJsonCli<CommandError>(['this', 'Conflict']);
      expect(nameConflict.code).toBe(5);
      expect(thisConflict.code).toBe(nameConflict.code);
      expect(json(nameConflict)).toEqual({
        error: { code: 'PANE_ALREADY_BOUND', message: 'Pane is already bound to another name.' },
      });
      expect(json(thisConflict)).toEqual(json(nameConflict));
      expect(fixture.paneMetadata(pane)).toBe(beforeConflictMetadata);
      // A refused bind leaves no never-bound temporary identity behind.
      expect(await listIdentities(fixture)).toEqual(beforeConflictList);

      const unboundHuman = await fixture.runCli(['unbind']);
      expect(unboundHuman.code).toBe(0);
      expect(unboundHuman.stdout).toContain(`Unbound 'Lifecycle' from pane ${pane}`);
      expect(unboundHuman.stderr).toBe('');

      const repeated = await fixture.runJsonCli<CommandError>(['unbind']);
      expect(repeated.code).toBe(1);
      expect(json(repeated)).toEqual({
        error: { code: 'UNBOUND_PANE', message: 'Pane has no active global name.' },
      });
      expect(fixture.paneMetadata(peer.pane)).toBe(peerMetadata);
      expect(await listIdentities(fixture)).toEqual(peerList);

      const finalWhoami = await fixture.runJsonCli<{ bound: false; pane: string }>(['whoami']);
      expect(finalWhoami.code).toBe(0);
      expect(json(finalWhoami)).toEqual({
        bound: false,
        pane,
        interfaceKind: 'container',
        sessionState: 'unknown',
      });
    });
  }, 30_000);

  it('accepts every pane target form, normalizes names, is idempotent, and preserves failed state', async () => {
    await withE2EFixture(async (fixture) => {
      const alpha = await fixture.createMockPane('alpha');
      const beta = await fixture.createMockPane('beta');
      const gamma = await fixture.createMockPane('gamma');
      const betaTarget = fixture.paneTarget(beta.pane);
      const gammaTarget = fixture.paneTarget(gamma.pane);

      const alphaAdd = await fixture.runJsonCli<{ bound: true; name: string; pane: string }>([
        'add',
        alpha.pane,
        '  Alpha  ',
      ]);
      expect(alphaAdd.code).toBe(0);
      expect(json(alphaAdd)).toEqual(boundProjection(fixture, 'Alpha', alpha.pane));

      const betaAdd = await fixture.runJsonCli<{ bound: true; name: string; pane: string }>([
        'add',
        betaTarget.slice(betaTarget.indexOf(':') + 1),
        'Beta',
      ]);
      expect(betaAdd.code).toBe(0);
      expect(json(betaAdd)).toEqual(boundProjection(fixture, 'Beta', beta.pane));

      const gammaAdd = await fixture.runJsonCli<{ bound: true; name: string; pane: string }>([
        'add',
        gammaTarget,
        'Gamma',
      ]);
      expect(gammaAdd.code).toBe(0);
      expect(json(gammaAdd)).toEqual(boundProjection(fixture, 'Gamma', gamma.pane));

      expect(metadata(fixture, alpha)).toMatchObject({
        version: 1,
        globalIdentity: { name: 'Alpha', canonicalName: 'alpha' },
      });
      expect(metadata(fixture, beta)).toMatchObject({
        version: 1,
        globalIdentity: { name: 'Beta', canonicalName: 'beta' },
      });
      expect(metadata(fixture, gamma)).toMatchObject({
        version: 1,
        globalIdentity: { name: 'Gamma', canonicalName: 'gamma' },
      });

      const beforeIdempotent = await listIdentities(fixture);
      const beforeMetadata = fixture.paneMetadata(alpha.pane);
      const idempotent = await fixture.runJsonCli(['add', alpha.pane, ' ＡＬＰＨＡ ']);
      expect(idempotent.code).toBe(0);
      expect(json(idempotent)).toEqual(json(alphaAdd));
      expect(fixture.paneMetadata(alpha.pane)).toBe(beforeMetadata);
      expect(await listIdentities(fixture)).toEqual(beforeIdempotent);

      const beforeConflictList = await listIdentities(fixture);
      const beforeConflictMetadata = [
        fixture.paneMetadata(alpha.pane),
        fixture.paneMetadata(beta.pane),
        fixture.paneMetadata(gamma.pane),
        fixture.paneMetadata(fixture.pane),
      ];
      const paneConflict = await fixture.runJsonCli<CommandError>(['add', alpha.pane, 'Other']);
      expect(paneConflict.code).toBe(5);
      expect(json(paneConflict)).toEqual({
        error: { code: 'PANE_ALREADY_BOUND', message: 'Pane is already bound to another name.' },
      });
      const nameConflict = await fixture.runJsonCli<CommandError>([
        'add',
        fixture.pane,
        ' ＡＬＰＨＡ ',
      ]);
      expect(nameConflict.code).toBe(5);
      expect(json(nameConflict)).toEqual({
        error: { code: 'NAME_ALREADY_ACTIVE', message: 'Name is already active on another pane.' },
      });
      expect(await listIdentities(fixture)).toEqual(beforeConflictList);
      expect([
        fixture.paneMetadata(alpha.pane),
        fixture.paneMetadata(beta.pane),
        fixture.paneMetadata(gamma.pane),
        fixture.paneMetadata(fixture.pane),
      ]).toEqual(beforeConflictMetadata);
    });
  }, 30_000);

  it('rejects empty, control-character, and pane-shaped names without side effects', async () => {
    await withE2EFixture(async (fixture) => {
      const beforeList = await listIdentities(fixture);
      const beforeMetadata = fixture.paneMetadata(fixture.pane);
      const cases = [
        ['', 'Identity name must not be empty or contain control characters.'],
        ['   ', 'Identity name must not be empty or contain control characters.'],
        ['bad\u0001name', 'Identity name must not be empty or contain control characters.'],
        ['%99999', 'Identity name must not look like a pane target.'],
        ['0.0', 'Identity name must not look like a pane target.'],
        ['e2e:0.0', 'Identity name must not look like a pane target.'],
      ] as const;
      for (const [invalidName, message] of cases) {
        const result = await fixture.runJsonCli<CommandError>(['add', fixture.pane, invalidName]);
        expect(result.code).toBe(1);
        expect(json(result)).toEqual({
          error: { code: 'INVALID_NAME', message },
        });
        expect(await listIdentities(fixture)).toEqual(beforeList);
        expect(fixture.paneMetadata(fixture.pane)).toBe(beforeMetadata);
      }
    });
  }, 30_000);

  it('retains bound IDs and routes to the right PID across join, break, and swap', async () => {
    await withE2EFixture(async (fixture) => {
      const alpha = await fixture.createMockPane('alpha');
      const beta = await fixture.createMockPane('beta');
      await fixture.runJsonCli(['add', alpha.pane, 'Alpha']);
      await fixture.runJsonCli(['add', beta.pane, 'Beta']);
      const alphaMetadata = fixture.paneMetadata(alpha.pane);
      const betaMetadata = fixture.paneMetadata(beta.pane);
      const alphaInitialTarget = fixture.paneTarget(alpha.pane);
      const betaInitialTarget = fixture.paneTarget(beta.pane);

      fixture.tmux(['join-pane', '-s', alpha.pane, '-t', beta.pane]);
      const alphaJoinedTarget = fixture.paneTarget(alpha.pane);
      expect(alphaJoinedTarget).not.toBe(alphaInitialTarget);
      expect(fixture.paneMetadata(alpha.pane)).toBe(alphaMetadata);
      expect(fixture.paneMetadata(beta.pane)).toBe(betaMetadata);
      expect((await listIdentities(fixture)).find((item) => item.pane === alpha.pane)?.target).toBe(
        alphaJoinedTarget
      );
      await talkToIdentity(fixture, 'Alpha', alpha.pid, [beta.pid], 'after-join-alpha');

      fixture.tmux(['break-pane', '-s', alpha.pane, '-d']);
      const alphaBrokenTarget = fixture.paneTarget(alpha.pane);
      expect(alphaBrokenTarget).not.toBe(alphaJoinedTarget);
      expect(fixture.paneMetadata(alpha.pane)).toBe(alphaMetadata);
      expect(fixture.paneMetadata(beta.pane)).toBe(betaMetadata);
      await talkToIdentity(fixture, 'Alpha', alpha.pid, [beta.pid], 'after-break-alpha');

      const alphaBeforeSwap = fixture.paneTarget(alpha.pane);
      const betaBeforeSwap = fixture.paneTarget(beta.pane);
      fixture.tmux(['swap-pane', '-s', alpha.pane, '-t', beta.pane]);
      expect(fixture.paneTarget(alpha.pane)).toBe(betaBeforeSwap);
      expect(fixture.paneTarget(beta.pane)).toBe(alphaBeforeSwap);
      expect(fixture.paneMetadata(alpha.pane)).toBe(alphaMetadata);
      expect(fixture.paneMetadata(beta.pane)).toBe(betaMetadata);
      await talkToIdentity(fixture, 'Alpha', alpha.pid, [beta.pid], 'after-swap-alpha');
      await talkToIdentity(fixture, 'Beta', beta.pid, [alpha.pid], 'after-swap-beta');

      expect(fixture.paneTarget(alpha.pane)).not.toBe(alphaInitialTarget);
      expect(fixture.paneTarget(beta.pane)).not.toBe(betaInitialTarget);
    });
  }, 45_000);

  it('distinguishes name and pane disappearance after kill-pane while preserving peers', async () => {
    await withE2EFixture(async (fixture) => {
      const gone = await fixture.createMockPane('gone');
      const peer = await fixture.createMockPane('peer');
      await fixture.runJsonCli(['add', gone.pane, 'Gone']);
      await fixture.runJsonCli(['add', peer.pane, 'Peer']);
      const peerMetadata = fixture.paneMetadata(peer.pane);

      fixture.tmux(['kill-pane', '-t', gone.pane]);
      await fixture.waitFor(
        () => {
          try {
            return !fixture
              .tmux(['list-panes', '-a', '-F', '#{pane_id}'])
              .split('\n')
              .includes(gone.pane);
          } catch {
            return true;
          }
        },
        2_000,
        'killed pane to disappear'
      );

      expect(await listIdentities(fixture)).toEqual([
        {
          name: 'Peer',
          id: boundProjection(fixture, 'Peer', peer.pane).id,
          lifetime: 'temporary',
          presence: 'active',
          canonicalName: 'peer',
          pane: peer.pane,
          target: fixture.paneTarget(peer.pane),
          cwd: peer.workspace,
          command: 'node',
          session: { activity: unknownActivity },
        },
      ]);
      expect(fixture.paneMetadata(peer.pane)).toBe(peerMetadata);

      const nameNotFound = await fixture.runJsonCli<CommandError>([
        'talk',
        'Gone',
        'must-not-send',
      ]);
      expect(nameNotFound.code).toBe(3);
      expect(json(nameNotFound)).toEqual({
        error: { code: 'NAME_NOT_FOUND', message: "Identity 'Gone' was not found." },
      });

      const paneNotFound = await fixture.runJsonCli<CommandError>(['check', gone.pane]);
      expect(paneNotFound.code).toBe(3);
      expect(json(paneNotFound)).toEqual({
        error: { code: 'PANE_NOT_FOUND', message: `Pane target '${gone.pane}' was not found.` },
      });
      await talkToIdentity(fixture, 'Peer', peer.pid, [], 'peer-still-routes');
    });
  }, 30_000);

  it('does not resurrect identities after a real tmux-server restart', async () => {
    await withE2EFixture(async (fixture) => {
      const oldPane = fixture.pane;
      const oldPid = fixture.panePid;
      const secondary = await fixture.createMockPane('secondary');
      const bound = await fixture.runJsonCli(['add', oldPane, 'BeforeRestart']);
      expect(bound.code).toBe(0);
      const secondaryBound = await fixture.runJsonCli([
        'add',
        secondary.pane,
        'SecondaryBeforeRestart',
      ]);
      expect(secondaryBound.code).toBe(0);
      expect((await listIdentities(fixture)).map(({ name }) => name)).toEqual([
        'BeforeRestart',
        'SecondaryBeforeRestart',
      ]);

      const restarted = await fixture.restartServer();
      expect(restarted.pid).not.toBe(oldPid);
      expect(restarted.pane).not.toBe(secondary.pane);
      expect(fixture.serverIsRunning()).toBe(true);
      expect(await listIdentities(fixture)).toEqual([]);

      const whoami = await fixture.runJsonCli<{ bound: false; pane: string }>(['whoami']);
      expect(whoami.code).toBe(0);
      expect(json(whoami)).toEqual({
        bound: false,
        pane: restarted.pane,
        interfaceKind: 'container',
        sessionState: 'unknown',
      });

      const stalePane = await fixture.runJsonCli<CommandError>(['check', secondary.pane]);
      expect(stalePane.code).toBe(3);
      expect(json(stalePane)).toEqual({
        error: {
          code: 'PANE_NOT_FOUND',
          message: `Pane target '${secondary.pane}' was not found.`,
        },
      });
      const staleName = await fixture.runJsonCli<CommandError>([
        'talk',
        'BeforeRestart',
        'must-not-send',
      ]);
      expect(staleName.code).toBe(3);
      expect(json(staleName)).toEqual({
        error: { code: 'NAME_NOT_FOUND', message: "Identity 'BeforeRestart' was not found." },
      });

      const rebound = await fixture.runJsonCli(['add', restarted.pane, 'AfterRestart']);
      expect(rebound.code).toBe(0);
      expect(await listIdentities(fixture)).toEqual([
        {
          name: 'AfterRestart',
          id: boundProjection(fixture, 'AfterRestart', restarted.pane).id,
          lifetime: 'temporary',
          presence: 'active',
          canonicalName: 'afterrestart',
          pane: restarted.pane,
          target: fixture.paneTarget(restarted.pane),
          cwd: restarted.workspace,
          command: 'node',
          session: { activity: unknownActivity },
        },
      ]);
    });
  }, 30_000);

  it('preserves a durable identity across pane death and a later rebind', async () => {
    await withE2EFixture(async (fixture) => {
      const gone = await fixture.createMockPane('durable');
      const original = await fixture.runJsonCli(['add', gone.pane, 'Durable', '-s']);
      expect(original.code).toBe(0);
      const database = new Database(path.join(fixture.globalDir, 'tmux-team.db'), {
        readonly: true,
      });
      const identity = database
        .prepare('SELECT id FROM identities WHERE canonical_name = ?')
        .get('durable') as { id: string };
      database.close();
      expect(identity.id).toMatch(/^[0-9a-f-]{36}$/);

      fixture.tmux(['kill-pane', '-t', gone.pane]);
      await fixture.waitFor(
        () => {
          try {
            return !fixture.tmux(['list-panes', '-a', '-F', '#{pane_id}']).includes(gone.pane);
          } catch {
            return true;
          }
        },
        2_000,
        'durable pane to disappear'
      );
      expect(withoutAddress(await fixture.runJsonCli(['list'])).json).toEqual({
        identities: [
          {
            id: identity.id,
            name: 'Durable',
            canonicalName: 'durable',
            lifetime: 'saved',
            presence: 'offline',
            pane: null,
            command: '',
            session: { activity: unknownActivity },
          },
        ],
      });

      const afterDeath = new Database(path.join(fixture.globalDir, 'tmux-team.db'), {
        readonly: true,
      });
      expect(
        afterDeath.prepare('SELECT id FROM identities WHERE canonical_name = ?').get('durable')
      ).toEqual(identity);
      expect(afterDeath.prepare('SELECT COUNT(*) AS count FROM bindings').get()).toEqual({
        count: 0,
      });
      afterDeath.close();

      const replacement = await fixture.createMockPane('replacement');
      const rebound = await fixture.runJsonCli(['add', replacement.pane, 'durable']);
      expect(rebound.code).toBe(0);
      const afterRebind = new Database(path.join(fixture.globalDir, 'tmux-team.db'), {
        readonly: true,
      });
      expect(
        afterRebind.prepare('SELECT id FROM identities WHERE canonical_name = ?').get('durable')
      ).toEqual(identity);
      afterRebind.close();
    });
  }, 30_000);

  it('ignores old and malformed pane markers without backfilling, then supports explicit rebinding', async () => {
    await withE2EFixture(async (fixture) => {
      const oldPane = await fixture.createMockPane('legacy-marker');
      const collidingOldPane = await fixture.createMockPane('legacy-collision');
      const malformedPane = await fixture.createMockPane('malformed-marker');
      const healthy = await fixture.runJsonCli<{ bound: true; name: string; pane: string }>([
        'name',
        'Healthy',
      ]);
      expect(healthy.code).toBe(0);
      const profile = await fixture.runJsonCli(['role', 'set', 'Keep the healthy profile.']);
      expect(profile.code).toBe(0);
      const legacyPath = path.join(fixture.workspace, 'tmux-team.json');
      const legacyBytes = '{\n  "legacy-file": {"pane": "%999", "remark": "keep"}\n}\n';
      fs.writeFileSync(legacyPath, legacyBytes);

      fixture.tmux([
        'set-option',
        '-p',
        '-t',
        oldPane.pane,
        '@tmt.agent',
        JSON.stringify({
          version: 1,
          globalIdentity: { name: 'LegacyOnly', canonicalName: 'legacyonly' },
        }),
      ]);
      fixture.tmux([
        'set-option',
        '-p',
        '-t',
        collidingOldPane.pane,
        '@tmt.agent',
        JSON.stringify({
          version: 1,
          globalIdentity: { name: 'LEGACYONLY', canonicalName: 'legacyonly' },
        }),
      ]);
      fixture.tmux([
        'set-option',
        '-p',
        '-t',
        malformedPane.pane,
        '@tmt.agent',
        JSON.stringify({ version: 1, globalIdentity: { name: 42, canonicalName: 'malformed' } }),
      ]);

      const oldMarkerBytes = fixture.paneMetadata(oldPane.pane);
      const collidingOldMarkerBytes = fixture.paneMetadata(collidingOldPane.pane);
      const malformedMarkerBytes = fixture.paneMetadata(malformedPane.pane);
      const before = durableState(fixture);
      expect(fs.readFileSync(legacyPath, 'utf8')).toBe(legacyBytes);

      for (let attempt = 0; attempt < 2; attempt += 1) {
        const listed = await listIdentities(fixture);
        expect(listed).toHaveLength(1);
        expect(listed[0]).toMatchObject({
          name: 'Healthy',
          canonicalName: 'healthy',
          pane: fixture.pane,
          target: fixture.paneTarget(fixture.pane),
          cwd: fixture.workspace,
          command: 'node',
          session: { activity: unknownActivity },
        });
        expect(fixture.paneMetadata(oldPane.pane)).toBe(oldMarkerBytes);
        expect(fixture.paneMetadata(collidingOldPane.pane)).toBe(collidingOldMarkerBytes);
        expect(fixture.paneMetadata(malformedPane.pane)).toBe(malformedMarkerBytes);
        expect(fs.readFileSync(legacyPath, 'utf8')).toBe(legacyBytes);
      }

      const afterLists = durableState(fixture);
      expect(afterLists.identities).toEqual(before.identities);
      expect(withoutVerificationTimestamp(afterLists.bindings)).toEqual(
        withoutVerificationTimestamp(before.bindings)
      );
      expect(afterLists.profiles).toEqual(before.profiles);

      for (const [name, pid, message] of [
        ['LegacyOnly', oldPane.pid, 'legacy-name-must-not-route'],
        ['LEGACYONLY', collidingOldPane.pid, 'legacy-collision-must-not-route'],
        ['Malformed', malformedPane.pid, 'malformed-marker-must-not-route'],
      ] as const) {
        const result = await fixture.runJsonCli<CommandError>(['talk', name, message]);
        expect(result.code).toBe(3);
        expect(json(result)).toMatchObject({ error: { code: 'NAME_NOT_FOUND' } });
        expect(
          fixture
            .events()
            .filter(
              (event) =>
                (event.event === 'request' || event.event === 'submitted') &&
                event.pid === pid &&
                event.message === message
            )
        ).toHaveLength(0);
      }

      await talkToIdentity(
        fixture,
        'Healthy',
        fixture.panePid,
        [oldPane.pid, collidingOldPane.pid, malformedPane.pid],
        'healthy-causal-routing'
      );

      const explicit = await fixture.runJsonCli<{ bound: true; name: string; pane: string }>([
        'add',
        oldPane.pane,
        'FreshExplicit',
      ]);
      expect(explicit.code).toBe(0);
      expect(json(explicit)).toEqual(boundProjection(fixture, 'FreshExplicit', oldPane.pane));
      expect(fixture.paneMetadata(collidingOldPane.pane)).toBe(collidingOldMarkerBytes);
      expect(fixture.paneMetadata(malformedPane.pane)).toBe(malformedMarkerBytes);
      expect(fs.readFileSync(legacyPath, 'utf8')).toBe(legacyBytes);
      await talkToIdentity(
        fixture,
        'FreshExplicit',
        oldPane.pid,
        [fixture.panePid, collidingOldPane.pid, malformedPane.pid],
        'fresh-explicit-routing'
      );
    });
  }, 45_000);

  it('drops only a binding with malformed durable metadata while retaining its identity, profile, and healthy peer', async () => {
    await withE2EFixture(async (fixture) => {
      const affectedPane = await fixture.createMockPane('malformed-current');
      const peerPane = await fixture.createMockPane('healthy-peer');
      const healthy = await fixture.runJsonCli(['name', 'HealthyCurrent']);
      expect(healthy.code).toBe(0);
      expect((await fixture.runJsonCli(['role', 'set', 'Healthy current profile.'])).code).toBe(0);
      const peer = await fixture.runJsonCli(['add', peerPane.pane, 'HealthyPeer']);
      expect(peer.code).toBe(0);
      expect(
        (
          await fixture.runJsonCli([
            'role',
            'set',
            'Retain this peer profile.',
            '--identity',
            'HealthyPeer',
          ])
        ).code
      ).toBe(0);
      const affected = await fixture.runJsonCli(['add', affectedPane.pane, 'MalformedCurrent']);
      expect(affected.code).toBe(0);
      expect(
        (
          await fixture.runJsonCli([
            'role',
            'set',
            'Retain this affected identity profile.',
            '--identity',
            'MalformedCurrent',
          ])
        ).code
      ).toBe(0);
      const affectedMetadataObject = metadata(fixture, affectedPane);
      const affectedIdentityMetadata = affectedMetadataObject.globalIdentity as Record<
        string,
        unknown
      >;
      affectedIdentityMetadata.name = null;
      fixture.tmux([
        'set-option',
        '-p',
        '-t',
        affectedPane.pane,
        '@tmt.agent',
        JSON.stringify(affectedMetadataObject),
      ]);
      const healthyMetadata = fixture.paneMetadata(fixture.pane);
      const affectedMetadata = fixture.paneMetadata(affectedPane.pane);
      const peerMetadata = fixture.paneMetadata(peerPane.pane);
      const before = durableState(fixture);
      const affectedIdentityId = before.identities.find(
        (identity) => identity.canonical_name === 'malformedcurrent'
      )?.id;
      const expectedHealthyBindingIds = before.bindings
        .filter((binding) => binding.identity_id !== affectedIdentityId)
        .map((binding) => binding.identity_id);
      expect(affectedIdentityId).toBeDefined();

      for (let attempt = 0; attempt < 2; attempt += 1) {
        const listed = await listIdentities(fixture);
        expect(listed.map(({ name }) => name)).toEqual([
          'HealthyCurrent',
          'HealthyPeer',
          'MalformedCurrent',
        ]);
        expect(listed.find(({ name }) => name === 'MalformedCurrent')).toEqual({
          id: affectedIdentityId,
          name: 'MalformedCurrent',
          canonicalName: 'malformedcurrent',
          lifetime: 'temporary',
          presence: 'offline',
          pane: null,
          command: '',
          session: { activity: unknownActivity },
        });
        expect(fixture.paneMetadata(fixture.pane)).toBe(healthyMetadata);
        expect(fixture.paneMetadata(affectedPane.pane)).toBe(affectedMetadata);
        expect(fixture.paneMetadata(peerPane.pane)).toBe(peerMetadata);
      }

      const after = durableState(fixture);
      expect(after.identities).toEqual(before.identities);
      expect(after.profiles).toEqual(before.profiles);
      expect(withoutVerificationTimestamp(after.bindings)).toEqual(
        withoutVerificationTimestamp(before.bindings).filter(
          (binding) => binding.identity_id !== affectedIdentityId
        )
      );
      expect(after.bindings.map((binding) => binding.identity_id).sort()).toEqual(
        expectedHealthyBindingIds.sort()
      );

      // Once reconciliation removes the binding, ordinary talk uses the unbound
      // foreground waiter. Bound this probe instead of awaiting its 180s default.
      const waitingTalk = await fixture.runJsonCli<CommandError & { requestId: string }>([
        'talk',
        'MalformedCurrent',
        'unbound-wait-must-not-route',
        '--timeout',
        '20ms',
      ]);
      expect(waitingTalk.code).toBe(4);
      const waitingOutput = json(waitingTalk);
      expect(waitingOutput).toMatchObject({ status: 'timeout', error: { code: 'TIMEOUT' } });
      expect(waitingOutput.error.suggestion).toContain(
        `tmt inbox --identity '${affectedIdentityId}' --json`
      );
      expect(
        fixture.events().some((event) => event.message === 'unbound-wait-must-not-route')
      ).toBe(false);

      // This assertion inspects queue acceptance rather than waiting for a reply.
      const affectedTalk = await fixture.runJsonCli<{ requestId: string }>([
        'talk',
        'MalformedCurrent',
        'affected-must-not-route',
        '--detach',
      ]);
      expect(affectedTalk.code).toBe(0);
      expect(json(affectedTalk)).toMatchObject({ status: 'queued', offline: true });
      expect(json(affectedTalk)).toMatchObject({
        recipientIdentityId: affectedIdentityId,
        notification: 'not_attempted',
        waitingFor: 'recipient_inbox_pull',
      });
      const database = new Database(path.join(fixture.globalDir, 'tmux-team.db'), {
        readonly: true,
      });
      try {
        const attempt = database.prepare(
          'SELECT recipient_identity_id, status, wait_active, wake_state FROM request_attempts WHERE request_id = ?'
        );
        for (const requestId of [waitingOutput.requestId, json(affectedTalk).requestId]) {
          expect(attempt.get(requestId)).toEqual({
            recipient_identity_id: affectedIdentityId,
            status: 'queued',
            wait_active: 0,
            wake_state: 'not_attempted',
          });
        }
      } finally {
        database.close();
      }
      expect(fixture.events().some((event) => event.message === 'affected-must-not-route')).toBe(
        false
      );
      await talkToIdentity(
        fixture,
        'HealthyCurrent',
        fixture.panePid,
        [peerPane.pid, affectedPane.pid],
        'healthy-peer'
      );
      await talkToIdentity(
        fixture,
        'HealthyPeer',
        peerPane.pid,
        [fixture.panePid, affectedPane.pid],
        'peer-healthy'
      );
    });
  }, 45_000);
});
