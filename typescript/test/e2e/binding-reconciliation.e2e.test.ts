import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import { expectJsonResult, unknownActivity, withoutAddress } from './cli-assertions.js';
import { withE2EFixture } from './harness.js';
import { durableState } from './identity-state-oracle.js';
import { readRealTmuxCli, releaseRealTmuxCli, spawnRealTmuxCli } from './real-tmux-caller.js';
import { installTmuxTrace } from './tmux-trace.js';
import { processTreeHasOpenFile } from './process-file-oracle.js';

const inputLog = { mode: 'input-log' } as const;

interface Identity {
  id: string;
  name: string;
  canonicalName: string;
  lifetime: 'temporary' | 'saved';
}
interface Bound extends Omit<Identity, 'canonicalName'> {
  bound: true;
  pane: string;
}
interface Listed extends Identity {
  presence: 'active' | 'offline' | 'unknown';
  pane: string | null;
  command: string;
  target?: string;
  cwd?: string;
}
interface Listing {
  identities: Listed[];
}

describe('binding publication, reconciliation and presentation', { concurrent: false }, () => {
  it('aligns human list columns for live Unicode identities and long pane paths without changing JSON', async () => {
    await withE2EFixture(async (fixture) => {
      // Fullwidth fixture data exposes byte/character-count padding bugs.
      const wideName = 'Ａｌｉｃｅ';
      const longName = 'long-reviewer-agent';
      const workspace = fixture.createWorkspace('long-repository-folder-for-column-alignment');
      const peer = await fixture.createMockPane('table-peer', workspace);
      const wide = expectJsonResult(await fixture.runJsonCli<Bound>(['name', wideName]));
      const long = expectJsonResult(await fixture.runJsonCli<Bound>(['add', peer.pane, longName]));
      const before = expectJsonResult(withoutAddress(await fixture.runJsonCli<Listing>(['ls'])));
      expect(before.identities).toEqual([
        expect.objectContaining({
          id: wide.id,
          name: wideName,
          cwd: fixture.workspace,
          pane: fixture.pane,
        }),
        expect.objectContaining({ id: long.id, name: longName, cwd: workspace, pane: peer.pane }),
      ]);
      const human = await fixture.runCli(['ls']);
      expect(human.code, human.stderr).toBe(0);
      expect(human.stderr).toBe('');
      expect(human.stdout).not.toContain('\t');
      const lines = human.stdout.trimEnd().split('\n');
      // Agent-first: one TEMPORARY section, no header row, rows sorted by name.
      expect(lines).toHaveLength(3);
      expect(lines[0]).toBe('TEMPORARY 2');
      const starts = new Set<string>();
      for (const [index, row] of before.identities.entries()) {
        const line = lines[index + 1]!;
        expect(line).toBe(line.trimEnd());
        // This known five-fullwidth-character name occupies ten columns.
        // Substitution is an independent fixture oracle, not a second renderer.
        const columns = line.replace(wideName, '1234567890');
        const name = row.name === wideName ? '1234567890' : row.name;
        expect(columns.slice(5, 5 + name.length)).toBe(name);
        const address = columns.indexOf(`tmux:${row.pane}`);
        expect(address).toBeGreaterThan(5);
        // The folder follows the address; home abbreviation depends on the CLI's HOME.
        const after = address + `tmux:${row.pane}`.length;
        const cwd = after + columns.slice(after).search(/\S/);
        expect(columns.slice(cwd)).toMatch(
          // A mock pane runs a shell, so the row ends with its `shell` action.
          new RegExp(`${path.basename(row.cwd!)}(  shell)?$`)
        );
        starts.add(`${address}:${cwd}`);
      }
      // Both rows share one layout: every column starts at the same place.
      expect(starts.size).toBe(1);
      expect(expectJsonResult(withoutAddress(await fixture.runJsonCli<Listing>(['ls'])))).toEqual(
        before
      );
      expect(
        durableState(fixture)
          .bindings.map((row) => row.identity_id)
          .sort()
      ).toEqual([wide.id, long.id].sort());
    }, inputLog);
  });

  it('coordinates a concurrent global observer with uncommitted publication and rechecks after lock acquisition', async () => {
    await withE2EFixture(async (fixture) => {
      fixture.enableMetadataBarrier({ phase: 'after' });
      const binder = fixture.runCliProcess<Bound>(['--json', 'name', 'Concurrent']);
      await fixture.waitForMetadataBarrier('applied', { child: binder });
      expect(durableState(fixture).bindings).toEqual([]);
      expect(JSON.parse(fixture.paneMetadata()).globalIdentity.name).toBe('Concurrent');
      const trace = installTmuxTrace(fixture);
      // Outside context avoids the harness's own pane-session lookup entering
      // this CLI-only trace; global observation itself needs no caller pane.
      const observer = fixture.runCliProcess<Listing>(['--json', 'ls'], { outsideTmux: true });
      let settled = false;
      void observer.result.then(() => {
        settled = true;
      });
      await fixture.waitFor(
        () =>
          !settled &&
          processTreeHasOpenFile(observer.pid, path.join(fixture.globalDir, 'tmux-team.db')),
        900,
        'native observer to open the locked database'
      );
      expect(settled).toBe(false);
      expect(trace.invocations()).toEqual([]);
      expect(durableState(fixture).bindings).toEqual([]);
      fixture.releaseMetadataBarrier();
      const bound = expectJsonResult(await binder.result);
      expect(expectJsonResult(await observer.result).identities).toEqual([
        expect.objectContaining({ id: bound.id, presence: 'active', pane: fixture.pane }),
      ]);
      expect(durableState(fixture).bindings).toEqual([
        expect.objectContaining({ identity_id: bound.id }),
      ]);
    }, inputLog);
  });

  it('fails a barrier wait at once, with the exit status and output, when the CLI exits first', async () => {
    await withE2EFixture(async (fixture) => {
      fixture.enableMetadataBarrier({ phase: 'after' });
      // An unknown command fails before any pane metadata is written, so the CLI can never reach the barrier.
      const early = fixture.runCliProcess(['--json', 'no-such-command']);
      const started = Date.now();
      const error = await fixture.waitForMetadataBarrier('applied', { child: early }).then(
        () => undefined,
        (rejection: unknown) => rejection
      );
      expect(error).toBeInstanceOf(Error);
      expect((error as Error).message).toMatch(
        /The CLI exited with code [1-9]\d* before metadata barrier 'applied'\.\nstdout: /
      );
      expect((error as Error).message).toContain('no-such-command');
      expect(Date.now() - started).toBeLessThan(5_000);
      expect((await early.result).code).not.toBe(0);
    }, inputLog);
  });

  it('fails closed on a copied server UUID instead of confusing equal pane IDs across sockets', async () => {
    await withE2EFixture(async (first) => {
      const alice = expectJsonResult(await first.runJsonCli<Bound>(['name', 'Alice']));
      const metadata = first.paneMetadata();
      const serverId = JSON.parse(metadata).globalIdentity.serverId;
      await withE2EFixture(
        async (second) => {
          second.tmux(['set-option', '-s', '@tmt.server-id', serverId]);
          expect(second.pane).toBe(first.pane);
          const result = await second.runJsonCli<{ error: { code: string } }>(['name', 'Bob']);
          expect(result.code).toBe(1);
          expect(result.json?.error.code).toBe('RECONCILIATION_FAILED');
          expect(first.paneMetadata()).toBe(metadata);
          expect(second.paneMetadata()).toBe('');
          expect(durableState(first).bindings).toEqual([
            expect.objectContaining({ identity_id: alice.id, socket_path: first.socketPath }),
          ]);
          const list = expectJsonResult(
            withoutAddress(await second.runJsonCli<Listing>(['ls']))
          ).identities;
          expect(list.map((row) => [row.name, row.presence])).toEqual([
            ['Alice', 'active'],
            ['Bob', 'offline'],
          ]);
        },
        { ...inputLog, globalDir: first.globalDir }
      );
    }, inputLog);
  });

  it('preserves unknown evidence even under force and only detaches marker-mismatched temporary rows', async () => {
    await withE2EFixture(async (fixture) => {
      const initial = expectJsonResult(await fixture.runJsonCli<Bound>(['name', 'Uncertain']));
      const original = fixture.paneMetadata();
      const serverId = JSON.parse(original).globalIdentity.serverId;
      const before = durableState(fixture);
      fixture.tmux(['set-option', '-s', '@tmt.server-id', '123e4567-e89b-42d3-a456-426614174111']);
      const rows = expectJsonResult(
        withoutAddress(await fixture.runJsonCli<Listing>(['ls']))
      ).identities;
      expect(rows).toEqual([
        {
          id: initial.id,
          name: 'Uncertain',
          canonicalName: 'uncertain',
          lifetime: 'temporary',
          presence: 'unknown',
          pane: null,
          command: '',
          session: { activity: unknownActivity },
        },
      ]);
      const removed = await fixture.runJsonCli<{ error: { code: string } }>([
        'rm',
        'Uncertain',
        '--force',
      ]);
      expect(removed.code).toBe(1);
      expect(removed.json?.error.code).toBe('RECONCILIATION_FAILED');
      expect(durableState(fixture)).toEqual(before);
      expect(fixture.paneMetadata()).toBe(original);
      fixture.tmux(['set-option', '-s', '@tmt.server-id', serverId]);
      const mismatched = JSON.parse(original);
      mismatched.globalIdentity.bindingId = 'different-binding';
      fixture.tmux([
        'set-option',
        '-p',
        '-t',
        fixture.pane,
        '@tmt.agent',
        JSON.stringify(mismatched),
      ]);
      const scoped = expectJsonResult(
        withoutAddress(
          await fixture.runJsonCli<{ presence: string; pane: null }>(['ls', 'Uncertain'])
        )
      );
      expect(scoped).toMatchObject({ presence: 'offline', pane: null });
      expect(durableState(fixture).identities).toEqual(before.identities);
      expect(durableState(fixture).bindings).toEqual([]);
      expect(JSON.parse(fixture.paneMetadata())).toEqual(mismatched);
    }, inputLog);
  });

  it('keeps scoped reads constant and batches global discovery in a mostly unbound server', async () => {
    await withE2EFixture(async (fixture) => {
      const owner = expectJsonResult(await fixture.runJsonCli<Bound>(['name', 'Owner']));
      const trace = installTmuxTrace(fixture);
      expectJsonResult(await fixture.runJsonCli(['whoami']));
      const small = trace.invocations().length;
      for (let index = 0; index < 40; index++) {
        fixture.tmux(['new-window', '-d', '-t', 'e2e:', '-n', `idle-${index}`, 'sleep', '300']);
      }
      const peer = await fixture.createMockPane('peer');
      const second = expectJsonResult(
        await fixture.runJsonCli<Bound>(['add', peer.pane, 'Second'])
      );
      trace.clear();
      expect(expectJsonResult(await fixture.runJsonCli<Bound>(['whoami']))).toEqual({
        ...owner,
        interfaceKind: 'container',
        sessionState: 'unknown',
      });
      expect(trace.invocations()).toHaveLength(small);
      expect(
        trace
          .invocations()
          .filter((line) => line.includes('list-panes'))
          .every((line) => line.includes(' -f ') && line.includes(fixture.pane))
      ).toBe(true);
      trace.clear();
      const all = expectJsonResult(
        withoutAddress(await fixture.runJsonCli<Listing>(['ls']))
      ).identities;
      expect(all.map((row) => row.id)).toEqual([owner.id, second.id]);
      const queries = trace.invocations().filter((line) => line.includes('list-panes'));
      expect(queries).toHaveLength(1);
      expect(queries[0]).toContain(' -f ');
      expect(queries[0]).toContain(fixture.pane);
      expect(queries[0]).toContain(peer.pane);
      expect(durableState(fixture).identities).toHaveLength(2);
    }, inputLog);
  });

  it('uses global names, promotes the same UUID, and keeps saved identities offline', async () => {
    await withE2EFixture(async (fixture) => {
      const initial = expectJsonResult(await fixture.runJsonCli<Bound>(['name', 'Alice']));
      expect(initial).toEqual({
        bound: true,
        id: expect.any(String),
        name: 'Alice',
        pane: fixture.pane,
        lifetime: 'temporary',
      });
      expect(JSON.parse(fixture.paneMetadata()).globalIdentity.identityId).toBe(initial.id);
      const otherFolder = fixture.createWorkspace('other-folder');
      const saved = expectJsonResult(
        await fixture.runJsonCli<Bound>(['this', 'ALICE', '-s'], { cwd: otherFolder })
      );
      expect(saved).toEqual({ ...initial, lifetime: 'saved' });
      expect(expectJsonResult(await fixture.runJsonCli<Bound>(['name', 'alice']))).toEqual(saved);
      expect(expectJsonResult(await fixture.runJsonCli<Bound>(['whoami']))).toEqual({
        ...saved,
        interfaceKind: 'container',
        sessionState: 'unknown',
      });
      const live = expectJsonResult(
        withoutAddress(await fixture.runJsonCli<Listing>(['ls'], { cwd: otherFolder }))
      ).identities;
      expect(live).toEqual([
        {
          id: initial.id,
          name: 'Alice',
          canonicalName: 'alice',
          lifetime: 'saved',
          presence: 'active',
          pane: fixture.pane,
          command: expect.any(String),
          session: { activity: unknownActivity },
          target: fixture.paneTarget(fixture.pane),
          cwd: fixture.workspace,
        },
      ]);

      expect(expectJsonResult(await fixture.runJsonCli(['unbind']))).toEqual({
        unbound: true,
        id: initial.id,
        name: 'Alice',
        pane: fixture.pane,
        lifetime: 'saved',
        retired: false,
      });
      expect(fixture.paneMetadata()).toBe('');
      expect(
        expectJsonResult(
          withoutAddress(await fixture.runJsonCli<Listing>(['ls'], { withoutTmux: true }))
        ).identities
      ).toEqual([
        {
          id: initial.id,
          name: 'Alice',
          canonicalName: 'alice',
          lifetime: 'saved',
          presence: 'offline',
          pane: null,
          command: '',
          session: { activity: unknownActivity },
        },
      ]);
      const denied = await fixture.runJsonCli<{ error: { code: string } }>(['rm', 'Alice'], {
        withoutTmux: true,
      });
      expect(denied.code).toBe(5);
      expect(denied.json?.error.code).toBe('CONFIRMATION_REQUIRED');
      expect(
        expectJsonResult(
          await fixture.runJsonCli(['remove', 'Alice', '--force'], { withoutTmux: true })
        )
      ).toEqual({
        removed: true,
        identity: { id: initial.id, name: 'Alice', canonicalName: 'alice', lifetime: 'saved' },
      });
      expect(
        expectJsonResult(
          withoutAddress(await fixture.runJsonCli<Listing>(['ls'], { withoutTmux: true }))
        ).identities
      ).toEqual([]);
      expect(durableState(fixture).identities).toEqual([
        expect.objectContaining({ id: initial.id, retired_at_ms: expect.any(Number) }),
      ]);
      expect(fs.existsSync(fixture.forbiddenTmuxLogPath)).toBe(false);
    }, inputLog);
  });

  it('resolves pane selectors, preserves movement, retires dead temporary names, and never kills on rm', async () => {
    await withE2EFixture(async (fixture) => {
      const peer = await fixture.createMockPane('peer');
      const named = expectJsonResult(
        await fixture.runJsonCli<Bound>(['add', fixture.paneTarget(peer.pane), 'Worker'])
      );
      fixture.tmux([
        'move-window',
        '-s',
        fixture.paneTarget(peer.pane).split('.')[0]!,
        '-t',
        'e2e:8',
      ]);
      const moved = expectJsonResult(withoutAddress(await fixture.runJsonCli<Listing>(['ls'])))
        .identities[0];
      expect(moved).toMatchObject({
        id: named.id,
        pane: peer.pane,
        target: fixture.paneTarget(peer.pane),
        lifetime: 'temporary',
        presence: 'active',
      });
      fixture.tmux(['kill-pane', '-t', peer.pane]);
      expect(
        expectJsonResult(withoutAddress(await fixture.runJsonCli<Listing>(['ls']))).identities
      ).toEqual([]);
      const fresh = expectJsonResult(
        await fixture.runJsonCli<Bound>(['add', fixture.pane, 'worker'])
      );
      expect(fresh.id).not.toBe(named.id);
      const removed = expectJsonResult(
        await fixture.runJsonCli<{ removed: boolean }>(['rm', 'worker'])
      );
      expect(removed.removed).toBe(true);
      expect(fixture.mockProcessIsRunning()).toBe(true);
      expect(fixture.paneMetadata()).toBe('');
      expect(expectJsonResult(await fixture.runJsonCli(['whoami']))).toEqual({
        bound: false,
        pane: fixture.pane,
        interfaceKind: 'container',
        sessionState: 'unknown',
      });
      expect(durableState(fixture).identities).toHaveLength(2);
      expect(
        durableState(fixture).identities.every((row) => typeof row.retired_at_ms === 'number')
      ).toBe(true);
    }, inputLog);
  });

  it('retires the identity a refused bind created on an occupied pane and lets the name be reused', async () => {
    await withE2EFixture(async (fixture) => {
      const owner = expectJsonResult(await fixture.runJsonCli<Bound>(['name', 'Owner']));
      const failed = await fixture.runJsonCli<{ error: { code: string } }>(['name', 'Retry']);
      expect(failed.code).toBe(5);
      expect(failed.json?.error.code).toBe('PANE_ALREADY_BOUND');
      const state = durableState(fixture);
      const refused = state.identities.find((row) => row.canonical_name === 'retry')!;
      // The creation committed separately, but the refusal retired it at once.
      expect(refused).toMatchObject({ lifetime: 'temporary' });
      expect(typeof refused.retired_at_ms).toBe('number');
      expect(state.bindings).toEqual([expect.objectContaining({ identity_id: owner.id })]);
      const listing = expectJsonResult(withoutAddress(await fixture.runJsonCli<Listing>(['ls'])));
      expect(listing.identities.map((row) => row.name)).toEqual(['Owner']);
      const peer = await fixture.createMockPane('retry');
      const rebound = expectJsonResult(
        await fixture.runJsonCli<Bound>(['add', peer.pane, 'retry'])
      );
      expect(rebound.id).not.toBe(refused.id);
      expect(rebound).toMatchObject({ lifetime: 'temporary', pane: peer.pane });
    }, inputLog);
  });

  it.each([
    { name: 'complete', stripTmux: false, stripPane: false },
    { name: 'no-pane', stripTmux: false, stripPane: true },
    { name: 'no-tmux', stripTmux: true, stripPane: false },
    { name: 'neither', stripTmux: true, stripPane: true },
  ])(
    'binds a real descendant with $name context without guessing the selected pane',
    async (context) => {
      await withE2EFixture(async (fixture) => {
        const real = await spawnRealTmuxCli(fixture, ['name', 'Descendant'], context);
        fixture.tmux(['select-pane', '-t', fixture.pane]);
        await releaseRealTmuxCli(fixture, real);
        const result = readRealTmuxCli<Bound>(real);
        expect(result.code, result.stderr).toBe(0);
        expect(result.stdout).toMatchObject({
          bound: true,
          name: 'Descendant',
          pane: real.pane,
          lifetime: 'temporary',
        });
        expect(JSON.parse(fixture.paneMetadata(real.pane)).globalIdentity.identityId).toBe(
          result.stdout.id
        );
        expect(fixture.paneMetadata()).toBe('');
        expect(
          expectJsonResult(
            withoutAddress(await fixture.runJsonCli<Listing>(['ls']))
          ).identities.map((row) => row.pane)
        ).toEqual([real.pane]);
      }, inputLog);
    },
    20_000
  );

  it.each(['grouped', 'linked'] as const)(
    'keeps one identity and the attached presentation for %s windows',
    async (mode) => {
      await withE2EFixture(async (fixture) => {
        const identity = expectJsonResult(await fixture.runJsonCli<Bound>(['name', 'Grouped']));
        if (mode === 'grouped') fixture.tmux(['new-session', '-d', '-t', 'e2e', '-s', 'attached']);
        else {
          fixture.tmux(['new-session', '-d', '-s', 'attached', 'sleep', '300']);
          fixture.tmux(['link-window', '-s', 'e2e:0', '-t', 'attached:']);
        }
        await fixture.attachSessionClient('attached');
        const live = expectJsonResult(
          withoutAddress(await fixture.runJsonCli<Listing>(['ls']))
        ).identities;
        expect(live).toHaveLength(1);
        expect(live[0]).toMatchObject({
          id: identity.id,
          pane: fixture.pane,
          presence: 'active',
          target: expect.stringMatching(/^attached:/),
        });
        const scoped = expectJsonResult(
          withoutAddress(
            await fixture.runJsonCli<{ pane: { id: string; target: string } }>(['ls', fixture.pane])
          )
        );
        expect(scoped.pane).toMatchObject({ id: fixture.pane, target: live[0]!.target });
        expect(durableState(fixture).bindings).toHaveLength(1);
      }, inputLog);
    }
  );

  it('rolls back a failed post-publication verification and recovers an orphan marker on retry', async () => {
    await withE2EFixture(async (fixture) => {
      fixture.enableMetadataBarrier({ phase: 'after' });
      const process = fixture.runCliProcess(['--json', 'name', 'Retry']);
      await fixture.waitForMetadataBarrier('applied', { child: process });
      const orphan = JSON.parse(fixture.paneMetadata()).globalIdentity;
      const pending = durableState(fixture);
      expect(pending.bindings).toEqual([]);
      expect(pending.identities).toEqual([
        expect.objectContaining({ id: orphan.identityId, retired_at_ms: null }),
      ]);
      // Kill only this fixture-owned CLI group while its published metadata is
      // visible but the transaction is uncommitted.
      process.kill('SIGKILL');
      expect((await process.result).code).not.toBe(0);
      fixture.releaseMetadataBarrier();
      expect(durableState(fixture).bindings).toEqual([]);
      const offline = expectJsonResult(
        withoutAddress(await fixture.runJsonCli<Listing>(['ls']))
      ).identities;
      expect(offline).toEqual([
        expect.objectContaining({ id: orphan.identityId, presence: 'offline', pane: null }),
      ]);
      expect(JSON.parse(fixture.paneMetadata()).globalIdentity).toEqual(orphan);
      const retry = expectJsonResult(await fixture.runJsonCli<Bound>(['name', 'Retry']));
      expect(retry.id).toBe(orphan.identityId);
      const marker = JSON.parse(fixture.paneMetadata()).globalIdentity;
      expect(marker.bindingId).not.toBe(orphan.bindingId);
      expect(durableState(fixture).bindings).toEqual([
        expect.objectContaining({ id: marker.bindingId, identity_id: retry.id }),
      ]);
    }, inputLog);
  });

  it('does not publish success when the pane disappears during publication', async () => {
    await withE2EFixture(async (fixture) => {
      const peer = await fixture.createMockPane('vanishing');
      fixture.enableMetadataBarrier({ phase: 'before' });
      const process = fixture.runCliProcess(['--json', 'add', peer.pane, 'Vanished']);
      await fixture.waitForMetadataBarrier('entered', { child: process });
      fixture.tmux(['kill-pane', '-t', peer.pane]);
      fixture.releaseMetadataBarrier();
      const result = await process.result;
      expect(result.code).toBe(1);
      expect(result.json).toMatchObject({ error: { code: 'RECONCILIATION_FAILED' } });
      expect(result.stdout).not.toContain('"bound":true');
      expect(durableState(fixture).bindings).toEqual([]);
      expect(durableState(fixture).identities).toEqual([
        expect.objectContaining({ canonical_name: 'vanished', retired_at_ms: null }),
      ]);
      expect(fixture.paneMetadata()).toBe('');
    }, inputLog);
  });

  it('routes foreign removal by its recorded socket without touching an equal local pane ID', async () => {
    await withE2EFixture(async (first) => {
      const alice = expectJsonResult(await first.runJsonCli<Bound>(['name', 'Alice', '-s']));
      await withE2EFixture(
        async (second) => {
          const bob = expectJsonResult(await second.runJsonCli<Bound>(['name', 'Bob']));
          expect(second.pane).toBe(first.pane);
          expect(
            expectJsonResult(
              withoutAddress(await second.runJsonCli<Listing>(['ls']))
            ).identities.map((row) => [row.id, row.presence])
          ).toEqual([
            [alice.id, 'active'],
            [bob.id, 'active'],
          ]);
          expectJsonResult(await second.runJsonCli(['rm', 'Alice', '--force']));
          expect(first.paneMetadata()).toBe('');
          expect(JSON.parse(second.paneMetadata()).globalIdentity.identityId).toBe(bob.id);
          expect(first.mockProcessIsRunning()).toBe(true);
          expect(second.mockProcessIsRunning()).toBe(true);
        },
        { ...inputLog, globalDir: first.globalDir }
      );
      // Native global visibility reconciles conclusive foreign death too;
      // the installed TS current-server-only rule is not this contract.
      expect(
        expectJsonResult(withoutAddress(await first.runJsonCli<Listing>(['ls']))).identities
      ).toEqual([]);
      expect(
        durableState(first).identities.every((row) => typeof row.retired_at_ms === 'number')
      ).toBe(true);
    }, inputLog);
  });
});
