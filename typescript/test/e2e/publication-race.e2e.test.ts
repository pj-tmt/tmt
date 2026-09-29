import Database from 'better-sqlite3';
import { describe, expect, it } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { E2EFixture, withE2EFixture, type CliProcess, type CliResult } from './harness.js';
import { processTreeHasOpenFile } from './process-file-oracle.js';

interface IdentityListItem {
  name: string;
  pane: string;
}

interface IdentityList {
  identities: IdentityListItem[];
}

interface DurableCounts {
  identities: number;
  bindings: number;
  profiles: number;
}

interface DurableIdentity {
  id: string;
  name: string;
  canonical_name: string;
}

interface DurableProfile {
  identity_id: string;
  content: string;
}

function databaseFile(fixture: E2EFixture): string {
  return path.join(fixture.globalDir, 'tmux-team.db');
}

function durableCounts(fixture: E2EFixture): DurableCounts {
  const database = new Database(databaseFile(fixture), { readonly: true, timeout: 0 });
  try {
    return {
      identities: (
        database.prepare('SELECT COUNT(*) AS count FROM identities').get() as { count: number }
      ).count,
      bindings: (
        database.prepare('SELECT COUNT(*) AS count FROM bindings').get() as { count: number }
      ).count,
      profiles: (
        database.prepare('SELECT COUNT(*) AS count FROM role_profiles').get() as { count: number }
      ).count,
    };
  } finally {
    database.close();
  }
}

function durableIdentity(fixture: E2EFixture, canonicalName: string): DurableIdentity {
  const database = new Database(databaseFile(fixture), { readonly: true, timeout: 0 });
  try {
    return database
      .prepare('SELECT id, name, canonical_name FROM identities WHERE canonical_name = ?')
      .get(canonicalName) as DurableIdentity;
  } finally {
    database.close();
  }
}

function identityIsRetired(fixture: E2EFixture, canonicalName: string): boolean {
  const database = new Database(databaseFile(fixture), { readonly: true, timeout: 0 });
  try {
    const row = database
      .prepare('SELECT retired_at_ms FROM identities WHERE canonical_name = ?')
      .get(canonicalName) as { retired_at_ms: number | null } | undefined;
    return typeof row?.retired_at_ms === 'number';
  } finally {
    database.close();
  }
}

function durableProfile(fixture: E2EFixture, identityId: string): DurableProfile {
  const database = new Database(databaseFile(fixture), { readonly: true, timeout: 0 });
  try {
    return database
      .prepare('SELECT identity_id, content FROM role_profiles WHERE identity_id = ?')
      .get(identityId) as DurableProfile;
  } finally {
    database.close();
  }
}

function writerIsLocked(fixture: E2EFixture): boolean {
  const database = new Database(databaseFile(fixture), { timeout: 0 });
  database.pragma('busy_timeout = 0');
  try {
    database.exec('BEGIN IMMEDIATE');
    database.exec('ROLLBACK');
    return false;
  } catch (error) {
    const code = (error as { code?: unknown }).code;
    if (code !== 'SQLITE_BUSY' && code !== 'SQLITE_LOCKED') throw error;
    return true;
  } finally {
    database.close();
  }
}

function json<T>(result: CliResult<T>): T {
  expect(result.stderr, result.stdout).toBe('');
  expect(result.json, result.stdout).toBeDefined();
  return result.json as T;
}

async function waitForSuccess<T>(process: CliProcess<T>): Promise<T> {
  const result = await process.result;
  expect(result.code, result.stderr || result.stdout).toBe(0);
  return json(result);
}

async function assertPublished(fixture: E2EFixture, name: string): Promise<void> {
  const whoami = await fixture.runJsonCli<{ bound: boolean; name?: string }>(['whoami']);
  expect(whoami.code).toBe(0);
  expect(json(whoami)).toMatchObject({ bound: true, name });

  const list = await fixture.runJsonCli<IdentityList>(['list']);
  expect(list.code).toBe(0);
  expect(json(list).identities).toEqual(
    expect.arrayContaining([expect.objectContaining({ name, pane: fixture.pane })])
  );

  const message = `publication-${name}`;
  const talk = await fixture.runJsonCli<{ status: string; response: string }>([
    'talk',
    name,
    message,
    '--timeout',
    '5',
  ]);
  expect(talk.code, talk.stderr || talk.stdout).toBe(0);
  expect(json(talk)).toMatchObject({
    status: 'completed',
    response: expect.stringContaining(`mock-agent response: ${message}`),
  });
  await fixture.waitForEvent(
    (event) =>
      event.event === 'submitted' && event.pid === fixture.panePid && event.message === message
  );
}

describe.sequential('crash-safe identity publication', () => {
  it('keeps an unpublished binding out of committed discovery while an observer starts', async () => {
    await withE2EFixture(
      async (fixture) => {
        const binder = fixture.runCliProcess(['--json', 'name', 'Published']);
        await fixture.waitForMetadataBarrier('entered');

        expect(durableCounts(fixture)).toMatchObject({ bindings: 0, profiles: 0 });
        expect(writerIsLocked(fixture)).toBe(true);

        let observerSettled = false;
        const observer = fixture.runCliProcess<IdentityList>(['--json', 'list']);
        void observer.result.then(
          () => {
            observerSettled = true;
          },
          () => {
            observerSettled = true;
          }
        );
        await fixture.waitFor(
          () => !observerSettled && processTreeHasOpenFile(observer.pid, databaseFile(fixture)),
          900,
          'observer to open SQLite while publication is locked'
        );
        expect(observerSettled).toBe(false);
        expect(writerIsLocked(fixture)).toBe(true);
        expect(durableCounts(fixture)).toMatchObject({ bindings: 0 });

        fixture.releaseMetadataBarrier();
        await waitForSuccess(binder);
        const listed = await waitForSuccess<IdentityList>(observer);
        expect(listed.identities).toEqual(
          expect.arrayContaining([
            expect.objectContaining({ name: 'Published', pane: fixture.pane }),
          ])
        );

        expect(durableCounts(fixture)).toMatchObject({ identities: 1, bindings: 1 });
        await assertPublished(fixture, 'Published');
      },
      { metadataBarrier: { phase: 'before' } }
    );
  }, 12_000);

  it.each([
    {
      label: 'same name on another pane',
      contender: (fixture: E2EFixture, pane: string) => ['--json', 'add', pane, 'Anchor'],
      expectedCode: 'NAME_ALREADY_ACTIVE',
      expectedBindings: 1,
      expectedIdentities: 1,
    },
    {
      label: 'different name on the same pane',
      contender: (_fixture: E2EFixture, _pane: string) => ['--json', 'name', 'Challenger'],
      expectedCode: 'PANE_ALREADY_BOUND',
      expectedBindings: 1,
      expectedIdentities: 2,
    },
    {
      label: 'same canonical name on the same pane',
      contender: (_fixture: E2EFixture, _pane: string) => ['--json', 'name', 'ａｎｃｈｏｒ'],
      expectedCode: undefined,
      expectedBindings: 1,
      expectedIdentities: 1,
    },
    {
      label: 'different name on another pane',
      contender: (fixture: E2EFixture, pane: string) => ['--json', 'add', pane, 'Challenger'],
      expectedCode: undefined,
      expectedBindings: 2,
      expectedIdentities: 2,
    },
  ])(
    'serializes $label without deleting the winner',
    async ({ contender, expectedCode, expectedBindings, expectedIdentities }) => {
      await withE2EFixture(
        async (fixture) => {
          const peer = await fixture.createMockPane('peer');
          const binder = fixture.runCliProcess<{ id: string }>(['--json', 'name', 'Anchor']);
          await fixture.waitForMetadataBarrier('entered');
          expect(writerIsLocked(fixture)).toBe(true);

          const contenderProcess = fixture.runCliProcess(contender(fixture, peer.pane));
          await fixture.waitFor(
            () => processTreeHasOpenFile(contenderProcess.pid, databaseFile(fixture)),
            900,
            'independent contender to open SQLite before publication commit'
          );
          expect(writerIsLocked(fixture)).toBe(true);

          fixture.releaseMetadataBarrier();
          const winner = await waitForSuccess<{ id: string }>(binder);
          const contenderResult = await contenderProcess.result;
          if (expectedCode) {
            expect(contenderResult.code).toBe(5);
            expect(json(contenderResult)).toMatchObject({ error: { code: expectedCode } });
          } else {
            expect(contenderResult.code, contenderResult.stderr || contenderResult.stdout).toBe(0);
            if (expectedIdentities === 1) {
              expect(json(contenderResult)).toMatchObject({ id: winner.id });
            }
          }

          expect(durableCounts(fixture)).toMatchObject({
            identities: expectedIdentities,
            bindings: expectedBindings,
          });
          expect(durableIdentity(fixture, 'anchor').id).toBe(winner.id);
          if (expectedCode === 'PANE_ALREADY_BOUND') {
            // The refused contender's separately committed creation is retired.
            expect(identityIsRetired(fixture, 'challenger')).toBe(true);
          }
          await assertPublished(fixture, 'Anchor');
        },
        { metadataBarrier: { phase: 'before' } }
      );
    },
    12_000
  );

  it('rolls back a killed post-metadata publication while retaining profile state for rebind', async () => {
    await withE2EFixture(async (fixture) => {
      expect((await fixture.runJsonCli(['name', 'Preserved', '-s'])).code).toBe(0);
      const profile = await fixture.runJsonCli(['role', 'set', 'Keep this profile']);
      expect(profile.code).toBe(0);
      expect((await fixture.runJsonCli(['unbind'])).code).toBe(0);
      const identityBefore = durableIdentity(fixture, 'preserved');
      const profileBefore = durableProfile(fixture, identityBefore.id);
      expect(profileBefore.content).toBe('Keep this profile');
      expect(durableCounts(fixture)).toMatchObject({ identities: 1, bindings: 0, profiles: 1 });

      fixture.enableMetadataBarrier({ phase: 'after' });
      const killed = fixture.runCliProcess(['--json', 'name', 'Preserved']);
      await fixture.waitForMetadataBarrier('applied');
      expect(fixture.paneMetadata()).toContain('Preserved');
      expect(durableCounts(fixture)).toMatchObject({ identities: 1, bindings: 0, profiles: 1 });

      killed.kill('SIGKILL');
      const killedResult = await killed.result;
      expect(killedResult.code).not.toBe(0);
      fixture.releaseMetadataBarrier();

      const whoami = await fixture.runJsonCli<{ bound: boolean }>(['whoami']);
      expect(whoami.code).toBe(0);
      expect(json(whoami)).toEqual({
        bound: false,
        pane: fixture.pane,
        interfaceKind: 'container',
        sessionState: 'unknown',
      });
      const listAfterKill = await fixture.runJsonCli<IdentityList>(['list']);
      expect(listAfterKill.code).toBe(0);
      expect(json(listAfterKill).identities).toEqual([
        {
          id: identityBefore.id,
          name: 'Preserved',
          canonicalName: 'preserved',
          lifetime: 'saved',
          presence: 'offline',
          pane: null,
          command: '',
        },
      ]);
      expect(durableCounts(fixture)).toMatchObject({ identities: 1, bindings: 0, profiles: 1 });
      expect(durableIdentity(fixture, 'preserved')).toEqual(identityBefore);
      expect(durableProfile(fixture, identityBefore.id)).toEqual(profileBefore);

      const restored = await fixture.runJsonCli(['name', 'Preserved']);
      expect(restored.code, restored.stderr || restored.stdout).toBe(0);
      const restoredProfile = await fixture.runJsonCli<{ role: { content: string } }>([
        'role',
        'show',
        '--identity',
        'Preserved',
      ]);
      expect(restoredProfile.code).toBe(0);
      expect(json(restoredProfile).role.content).toBe('Keep this profile');
      expect(durableIdentity(fixture, 'preserved')).toEqual(identityBefore);
      expect(durableProfile(fixture, identityBefore.id)).toEqual(profileBefore);
      expect(durableCounts(fixture)).toMatchObject({ identities: 1, bindings: 1, profiles: 1 });
    });
  }, 12_000);

  it('serializes unbind metadata removal against a replacement bind', async () => {
    await withE2EFixture(async (fixture) => {
      expect((await fixture.runJsonCli(['name', 'OldIdentity'])).code).toBe(0);
      const profile = await fixture.runJsonCli(['role', 'set', 'Retain during replacement']);
      expect(profile.code).toBe(0);
      const identityBefore = durableIdentity(fixture, 'oldidentity');
      const profileBefore = durableProfile(fixture, identityBefore.id);

      fixture.enableMetadataBarrier({ phase: 'after', operation: 'clear' });
      const unbinder = fixture.runCliProcess(['--json', 'unbind']);
      await fixture.waitForMetadataBarrier('applied');
      expect(fixture.paneMetadata()).toBe('');
      expect(durableCounts(fixture)).toMatchObject({ bindings: 1, profiles: 1 });
      expect(writerIsLocked(fixture)).toBe(true);

      const contenderProgress = path.join(fixture.root, 'replacement-started');
      const replacement = fixture.runCliProcess(['--json', 'name', 'NewIdentity'], {
        progressFile: contenderProgress,
      });
      await fixture.waitFor(
        () => fs.existsSync(contenderProgress),
        900,
        'replacement binder to invoke tmux'
      );

      fixture.releaseMetadataBarrier();
      const unbound = await waitForSuccess(unbinder);
      expect(unbound).toMatchObject({ unbound: true, name: 'OldIdentity' });
      await waitForSuccess(replacement);
      expect(durableCounts(fixture)).toMatchObject({ bindings: 1, profiles: 1 });

      const whoami = await fixture.runJsonCli<{ bound: boolean; name: string }>(['whoami']);
      expect(whoami.code).toBe(0);
      expect(json(whoami)).toMatchObject({ bound: true, name: 'NewIdentity' });
      const list = await fixture.runJsonCli<IdentityList>(['list']);
      expect(list.code).toBe(0);
      expect(json(list).identities).toEqual([
        expect.objectContaining({ name: 'NewIdentity', pane: fixture.pane }),
      ]);
      expect(durableIdentity(fixture, 'oldidentity')).toEqual(identityBefore);
      expect(durableProfile(fixture, identityBefore.id)).toEqual(profileBefore);
      await assertPublished(fixture, 'NewIdentity');
    });
  }, 12_000);

  it('cleans a paused CLI process when a scenario throws', async () => {
    const fixture = new E2EFixture();
    let childPid = 0;
    const run = async (): Promise<void> => {
      try {
        await fixture.start({ metadataBarrier: { phase: 'before' } });
        const child = fixture.runCliProcess(['--json', 'name', 'Cleanup']);
        childPid = child.pid;
        await fixture.waitForMetadataBarrier('entered');
        throw new Error('intentional scenario failure');
      } finally {
        await fixture.stop();
      }
    };

    await expect(run()).rejects.toThrow('intentional scenario failure');
    expect(() => process.kill(-childPid, 0)).toThrow();
  }, 12_000);
});
