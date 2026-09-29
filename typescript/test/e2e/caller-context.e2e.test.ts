import { durableIdentity } from './identity-state-oracle.js';
import Database from 'better-sqlite3';
import { describe, expect, it } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { withE2EFixture, type CliResult, type E2EFixture } from './harness.js';
import { readRealTmuxCli, releaseRealTmuxCli, spawnRealTmuxCli } from './real-tmux-caller.js';

interface CommandError {
  error: { code: string; message: string };
}

interface DurableState {
  identities: unknown[];
  bindings: unknown[];
  profiles: unknown[];
}

function json<T>(result: CliResult<T>): T {
  expect(result.stdout.trim(), result.stderr).not.toBe('');
  expect(result.stderr).toBe('');
  expect(result.json).toBeDefined();
  return result.json as T;
}

function databaseFile(fixture: E2EFixture): string {
  return path.join(fixture.globalDir, 'tmux-team.db');
}

function durableState(fixture: E2EFixture): DurableState {
  const file = databaseFile(fixture);
  if (!fs.existsSync(file)) return { identities: [], bindings: [], profiles: [] };
  const database = new Database(file, { readonly: true });
  try {
    return {
      identities: database.prepare('SELECT * FROM identities ORDER BY canonical_name').all(),
      bindings: database.prepare('SELECT * FROM bindings ORDER BY identity_id').all(),
      profiles: database.prepare('SELECT * FROM role_profiles ORDER BY identity_id').all(),
    };
  } finally {
    database.close();
  }
}

function expectExactError<T>(
  result: CliResult<T>,
  code: string,
  message: string,
  exitCode: number
) {
  expect(result.code).toBe(exitCode);
  expect(result.stderr).toBe('');
  expect(result.json).toEqual({ error: { code, message } });
}

async function seedIdentity(fixture: E2EFixture, name: string, profile: string): Promise<void> {
  const bound = await fixture.runJsonCli(['name', name]);
  expect(bound.code, bound.stderr || bound.stdout).toBe(0);
  const assigned = await fixture.runJsonCli(['role', 'set', profile]);
  expect(assigned.code, assigned.stderr || assigned.stdout).toBe(0);
}

describe.sequential('strict caller context', () => {
  it.each([
    ['missing TMUX_PANE', { stripPane: true }],
    ['missing TMUX and TMUX_PANE', { stripTmux: true, stripPane: true }],
  ])(
    'discovers the originating pane from a real tmux child process with %s',
    async (
      _label,
      contextOptions: { readonly stripTmux?: boolean; readonly stripPane?: boolean }
    ) => {
      await withE2EFixture(async (fixture) => {
        const real = await spawnRealTmuxCli(fixture, ['name', 'RealCaller'], {
          name: contextOptions.stripTmux ? 'caller-both-missing' : 'caller-pane-missing',
          ...contextOptions,
        });

        const peer = await fixture.createMockPane('active-peer');
        fixture.tmux(['select-pane', '-t', peer.pane]);
        await releaseRealTmuxCli(fixture, real);

        const identity = durableIdentity(fixture, 'RealCaller');
        const result = readRealTmuxCli<{
          bound: boolean;
          id: string;
          name: string;
          pane: string;
          lifetime: 'temporary' | 'saved';
        }>(real);
        expect(result).toEqual({
          code: 0,
          stdout: {
            bound: true,
            id: identity.id,
            name: 'RealCaller',
            pane: real.pane,
            lifetime: 'temporary',
          },
          stderr: '',
        });
        const binding = durableState(fixture).bindings.find(
          (row) => (row as { pane_id?: string }).pane_id === real.pane
        ) as { pane_id: string; pane_pid: number } | undefined;
        expect(binding).toMatchObject({ pane_id: real.pane, pane_pid: real.panePid });
        expect(JSON.parse(fixture.paneMetadata(real.pane))).toMatchObject({
          globalIdentity: { panePid: real.panePid },
        });
        expect(fixture.paneMetadata(peer.pane)).toBe('');
        expect(fixture.mockProcessIsRunning(real.panePid)).toBe(true);
      });
    },
    20_000
  );

  it('resolves the this alias from the real pane ancestry with both variables missing', async () => {
    let panePid = 0;
    let fixtureRef: E2EFixture | undefined;
    await withE2EFixture(async (fixture) => {
      fixtureRef = fixture;
      const real = await spawnRealTmuxCli(fixture, ['this', 'AliasCaller'], {
        name: 'caller-this-both-missing',
        stripTmux: true,
        stripPane: true,
      });
      panePid = real.panePid;
      const peer = await fixture.createMockPane('alias-active-peer');
      fixture.tmux(['select-pane', '-t', peer.pane]);
      await releaseRealTmuxCli(fixture, real);

      const identity = durableIdentity(fixture, 'AliasCaller');
      const result = readRealTmuxCli<{
        bound: boolean;
        id: string;
        name: string;
        pane: string;
        lifetime: 'temporary' | 'saved';
      }>(real);
      expect(result).toEqual({
        code: 0,
        stdout: {
          bound: true,
          id: identity.id,
          name: 'AliasCaller',
          pane: real.pane,
          lifetime: 'temporary',
        },
        stderr: '',
      });
      expect(fixture.paneMetadata(peer.pane)).toBe('');
    });
    expect(panePid).toBeGreaterThan(0);
    expect(fixtureRef?.mockProcessIsRunning(panePid)).toBe(false);
  }, 20_000);

  it('does not report a stripped descendant as outside tmux in human output', async () => {
    await withE2EFixture(async (fixture) => {
      const real = await spawnRealTmuxCli(fixture, ['whoami'], {
        name: 'human-caller-both-missing',
        stripTmux: true,
        stripPane: true,
        json: false,
      });
      const bound = await fixture.runJsonCli(['add', real.pane, 'HumanCaller'], {
        outsideTmux: true,
      });
      expect(bound.code, bound.stderr || bound.stdout).toBe(0);
      await releaseRealTmuxCli(fixture, real);

      const result = readRealTmuxCli<string>(real);
      expect(result.code).toBe(0);
      expect(result.stderr).toBe('');
      expect(result.stdout).toBe(`HumanCaller (temporary) on pane ${real.pane}\n`);
      expect(result.stdout).not.toContain('Not running inside tmux. Some features may not work.');
    });
  }, 20_000);

  it('uses real socket and selected-pane evidence for every implicit identity and role operation', async () => {
    await withE2EFixture(async (fixture) => {
      await seedIdentity(fixture, 'Current', 'initial caller profile');

      const current = durableIdentity(fixture, 'Current');
      const whoami = await fixture.runJsonCli<{
        bound: boolean;
        id: string;
        name: string;
        pane: string;
        lifetime: 'temporary' | 'saved';
      }>(['whoami']);
      expect(whoami.code).toBe(0);
      expect(json(whoami)).toEqual({
        interfaceKind: 'container',
        sessionState: 'unknown',
        bound: true,
        id: current.id,
        name: 'Current',
        pane: fixture.pane,
        lifetime: 'temporary',
      });

      const shown = await fixture.runJsonCli<{
        identity: { name: string };
        role: { content: string };
      }>(['role', 'show']);
      expect(shown.code).toBe(0);
      expect(json(shown)).toMatchObject({
        identity: { name: 'Current' },
        role: { content: 'initial caller profile' },
      });

      const updated = await fixture.runJsonCli(['role', 'set', 'updated caller profile']);
      expect(updated.code).toBe(0);
      const cleared = await fixture.runJsonCli(['role', 'clear']);
      expect(cleared.code).toBe(0);
      const empty = await fixture.runJsonCli<{ role: null }>(['role', 'show']);
      expect(empty.code).toBe(0);
      expect(json(empty).role).toBeNull();

      const nameIdempotent = await fixture.runJsonCli(['name', 'Current']);
      expect(nameIdempotent.code).toBe(0);
      const idempotent = await fixture.runJsonCli(['this', 'Current']);
      expect(idempotent.code).toBe(0);

      const peer = await fixture.createMockPane('peer');
      const peerBound = await fixture.runJsonCli(['add', peer.pane, 'Peer']);
      expect(peerBound.code).toBe(0);
      const peerIdentity = durableIdentity(fixture, 'Peer');
      const peerWhoami = await fixture.runJsonCli<{
        bound: boolean;
        id: string;
        name: string;
        pane: string;
        lifetime: 'temporary' | 'saved';
      }>(['whoami'], { pane: peer.pane });
      expect(peerWhoami.code).toBe(0);
      expect(json(peerWhoami)).toEqual({
        interfaceKind: 'container',
        sessionState: 'unknown',
        bound: true,
        id: peerIdentity.id,
        name: 'Peer',
        pane: peer.pane,
        lifetime: 'temporary',
      });
      const peerRole = await fixture.runJsonCli<{ identity: { name: string } }>(['role', 'show'], {
        pane: peer.pane,
      });
      expect(peerRole.code).toBe(0);
      expect(json(peerRole).identity.name).toBe('Peer');

      const unbound = await fixture.runJsonCli(['unbind']);
      expect(unbound.code).toBe(0);
      const finalWhoami = await fixture.runJsonCli<{ bound: boolean }>(['whoami']);
      expect(finalWhoami.code).toBe(0);
      expect(json(finalWhoami)).toEqual({
        bound: false,
        pane: fixture.pane,
        interfaceKind: 'container',
        sessionState: 'unknown',
      });
    });
  }, 30_000);

  it('rejects stale, aliased, and foreign caller evidence without durable or presentation mutation', async () => {
    await withE2EFixture(async (fixture) => {
      await seedIdentity(fixture, 'Stable', 'must survive invalid callers');
      const before = durableState(fixture);
      const metadataBefore = fixture.paneMetadata();
      const titleBefore = fixture.paneTitle();
      const callers = [
        { label: 'missing caller context', caller: { tmux: null, pane: null } },
        { label: 'stale pane', caller: { pane: '%99999' } },
        { label: 'pane alias', caller: { pane: 'e2e:0.0' } },
        {
          label: 'foreign socket evidence',
          caller: {
            tmux: `${fixture.socketPath}-foreign,${fixture.serverPid},${fixture.paneSessionId()}`,
          },
        },
        {
          label: 'foreign server pid evidence',
          caller: {
            tmux: `${fixture.socketPath},${fixture.serverPid + 1},${fixture.paneSessionId()}`,
          },
        },
      ];

      for (const { label, caller } of callers) {
        const commands = [['name', 'Replacement'], ['this', 'Replacement'], ['whoami'], ['unbind']];
        for (const args of commands) {
          const result = await fixture.runJsonCli<CommandError>(args, { caller });
          expectExactError(
            result,
            'PANE_NOT_FOUND',
            'Not running inside a resolvable tmux pane.',
            3
          );
          expect(durableState(fixture), `${label}: ${args.join(' ')}`).toEqual(before);
          expect(fixture.paneMetadata(), `${label}: metadata`).toBe(metadataBefore);
          expect(fixture.paneTitle(), `${label}: title`).toBe(titleBefore);
        }

        for (const operation of ['show', 'set', 'clear'] as const) {
          const args =
            operation === 'set' ? ['role', operation, 'should not write'] : ['role', operation];
          const result = await fixture.runJsonCli<CommandError>(args, { caller });
          expectExactError(
            result,
            'IDENTITY_REQUIRED',
            'An identity is required; use --identity or run from a verified bound pane.',
            1
          );
          expect(durableState(fixture), `${label}: role ${operation}`).toEqual(before);
          expect(fixture.paneMetadata(), `${label}: role metadata`).toBe(metadataBefore);
          expect(fixture.paneTitle(), `${label}: role title`).toBe(titleBefore);
        }
      }
    });
  }, 45_000);

  it('rejects an outside implicit caller before bootstrapping fresh storage', async () => {
    await withE2EFixture(async (fixture) => {
      const titleBefore = fixture.paneTitle();
      const callerModes = [
        { label: 'missing caller', options: { outsideTmux: true } },
        { label: 'stale caller', options: { caller: { pane: '%99999' } } },
      ];
      for (const { label, options } of callerModes) {
        const identityCommands = [
          ['whoami'],
          ['name', 'MustNotBind'],
          ['this', 'MustNotBind'],
          ['unbind'],
        ];
        for (const args of identityCommands) {
          const result = await fixture.runJsonCli<CommandError>(args, options);
          expectExactError(
            result,
            'PANE_NOT_FOUND',
            'Not running inside a resolvable tmux pane.',
            3
          );
          expect(fs.existsSync(databaseFile(fixture)), `${label}: database`).toBe(false);
          expect(fixture.paneMetadata(), `${label}: metadata`).toBe('');
          expect(fixture.paneTitle(), `${label}: title`).toBe(titleBefore);
        }

        for (const operation of ['show', 'set', 'clear'] as const) {
          const args =
            operation === 'set' ? ['role', operation, 'must not write'] : ['role', operation];
          const role = await fixture.runJsonCli<CommandError>(args, options);
          expectExactError(
            role,
            'IDENTITY_REQUIRED',
            'An identity is required; use --identity or run from a verified bound pane.',
            1
          );
          expect(fs.existsSync(databaseFile(fixture)), `${label}: role database`).toBe(false);
          expect(fixture.paneMetadata(), `${label}: role metadata`).toBe('');
          expect(fixture.paneTitle(), `${label}: role title`).toBe(titleBefore);
        }
      }
    });
  }, 15_000);

  it('allows explicit targets and offline explicit roles without caller context', async () => {
    await withE2EFixture(async (fixture) => {
      const peer = await fixture.createMockPane('peer');
      const added = await fixture.runJsonCli(['add', peer.pane, 'Peer'], {
        outsideTmux: true,
      });
      expect(added.code).toBe(0);
      const peerIdentity = durableIdentity(fixture, 'Peer');
      expect(added.json).toEqual({
        bound: true,
        id: peerIdentity.id,
        name: 'Peer',
        pane: peer.pane,
        lifetime: 'temporary',
      });

      const message = 'explicit outside caller';
      const talk = await fixture.runJsonCli<{ response: string }>(
        ['talk', 'Peer', message, '--timeout', '5'],
        { outsideTmux: true }
      );
      expect(talk.code).toBe(0);
      expect(talk.json?.response).toContain(`mock-agent response: ${message}`);
      await fixture.waitForEvent(
        (event) =>
          event.event === 'submitted' && event.pid === peer.pid && event.message === message
      );

      const checked = await fixture.runJsonCli<{ output: string }>(['check', 'Peer', '20'], {
        outsideTmux: true,
      });
      expect(checked.code).toBe(0);
      expect(checked.json?.output).toContain(`mock-agent summary: ${message}`);

      const offlineSet = await fixture.runJsonCli(
        ['role', 'set', 'offline explicit profile', '--identity', 'Peer'],
        {
          withoutTmux: true,
        }
      );
      expect(offlineSet.code).toBe(0);
      const offlineShow = await fixture.runJsonCli<{ role: { content: string } }>(
        ['role', 'show', '--identity', 'Peer'],
        { withoutTmux: true }
      );
      expect(offlineShow.code).toBe(0);
      expect(offlineShow.json?.role.content).toBe('offline explicit profile');
      const offlineClear = await fixture.runJsonCli(['role', 'clear', '--identity', 'Peer'], {
        withoutTmux: true,
      });
      expect(offlineClear.code).toBe(0);
      const offlineEmpty = await fixture.runJsonCli<{ role: null }>(
        ['role', 'show', '--identity', 'Peer'],
        { withoutTmux: true }
      );
      expect(offlineEmpty.code).toBe(0);
      expect(offlineEmpty.json?.role).toBeNull();
      expect(fs.existsSync(fixture.forbiddenTmuxLogPath)).toBe(false);
    });
  }, 30_000);
});
