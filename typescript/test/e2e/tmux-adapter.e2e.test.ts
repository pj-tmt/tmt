import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { expectJsonResult } from './cli-assertions.js';
import { withE2EFixture, type E2EFixture, type E2EFixtureOptions } from './harness.js';
import { readRealTmuxCli, releaseRealTmuxCli, spawnRealTmuxCli } from './real-tmux-caller.js';

interface Snapshot {
  server: { serverId: string; socketPath: string; serverPid: number; serverStartTime: string };
  panes: { id: string; panePid: number; target: string; marker: unknown }[];
}

interface Counted {
  commandCount: number;
}

function fixtureOptions(): E2EFixtureOptions {
  const probe = process.env.TMT_TEST_TMUX_PROBE;
  if (!probe) throw new Error('Native tmux E2E requires the Docker-built adapter probe.');
  // The existing selector validates the descriptor before fixture allocation.
  return { mode: 'input-log', executableEnv: { TMT_TEST_CLI: probe } };
}

describe('tmux adapter probe: caller evidence, inventory and metadata', { concurrent: false }, () => {
  it('cleans the native-selected fixture after an intentional scenario failure', async () => {
    const fixtures: E2EFixture[] = [];
    await expect(
      withE2EFixture(async (fixture) => {
        fixtures.push(fixture);
        expectJsonResult(await fixture.runJsonCli(['snapshot']));
        throw new Error('intentional native adapter scenario failure');
      }, fixtureOptions())
    ).rejects.toThrow('intentional native adapter scenario failure');
    expect(fixtures).toHaveLength(1);
    for (const fixture of fixtures) {
      expect(fs.existsSync(fixture.root)).toBe(false);
      expect(fs.existsSync(fixture.socketRoot)).toBe(false);
      expect(fixture.serverProcessIsRunning()).toBe(false);
      expect(fixture.mockProcessIsRunning()).toBe(false);
    }
  });
  it.each([
    { name: 'complete', stripTmux: false, stripPane: false },
    { name: 'missing-pane', stripTmux: false, stripPane: true },
    { name: 'missing-tmux', stripTmux: true, stripPane: false },
    { name: 'missing-both', stripTmux: true, stripPane: true },
  ])(
    'verifies real descendant caller evidence: $name',
    async (context) => {
      await withE2EFixture(async (fixture) => {
        const real = await spawnRealTmuxCli(fixture, ['caller'], context);
        const peer = await fixture.createMockPane('ambient-peer');
        fixture.tmux(['select-pane', '-t', peer.pane]);
        await releaseRealTmuxCli(fixture, real);
        const result = readRealTmuxCli<Counted & { pane: string }>(real);
        expect(result.code).toBe(0);
        expect(result.stderr).toBe('');
        expect(result.stdout.pane).toBe(real.pane);
        expect(result.stdout.commandCount).toBeGreaterThan(0);
        if (context.name === 'complete') expect(result.stdout.commandCount).toBe(1);
        expect(fixture.paneMetadata(real.pane)).toBe('');
        expect(fixture.paneMetadata(peer.pane)).toBe('');
        expect(fs.existsSync(path.join(fixture.globalDir, 'tmux-team.db'))).toBe(false);
      }, fixtureOptions());
    },
    20_000
  );

  it('rejects malformed and conflicting context without choosing the active pane', async () => {
    await withE2EFixture(async (fixture) => {
      const malformed = expectJsonResult(
        await fixture.runJsonCli<Counted & { pane: null }>(['caller'], {
          caller: { tmux: 'invalid', pane: fixture.pane },
        })
      );
      expect(malformed).toEqual({ pane: null, commandCount: 0 });
      const conflict = expectJsonResult(
        await fixture.runJsonCli<Counted & { pane: null }>(['caller'], {
          caller: { tmux: `/unrelated/socket,${fixture.serverPid},0` },
        })
      );
      expect(conflict).toEqual({ pane: null, commandCount: 1 });
      const outside = expectJsonResult(
        await fixture.runJsonCli<Counted & { pane: null }>(['caller'], { outsideTmux: true })
      );
      expect(outside.pane).toBeNull();
      expect(outside.commandCount).toBeGreaterThan(1);
    }, fixtureOptions());
  });

  it('keeps scoped and empty snapshots bounded in a mostly-unbound server', async () => {
    await withE2EFixture(async (fixture) => {
      for (let index = 0; index < 30; index++) {
        fixture.tmux(['new-window', '-d', '-t', 'e2e:', '-n', `unbound-${index}`, 'sleep', '300']);
      }
      const initial = expectJsonResult(await fixture.runJsonCli<Snapshot & Counted>(['snapshot']));
      expect(initial.panes).toHaveLength(31);
      expect(initial.panes.every((pane) => pane.marker === null)).toBe(true);
      const full = expectJsonResult(await fixture.runJsonCli<Snapshot & Counted>(['snapshot']));
      expect(full.commandCount).toBe(2);
      const batched = expectJsonResult(
        await fixture.runJsonCli<Snapshot & Counted>([
          'snapshot',
          ...full.panes.map((pane) => pane.id),
        ])
      );
      expect(batched.panes).toEqual(full.panes);
      expect(batched.commandCount).toBe(2);
      const scoped = expectJsonResult(
        await fixture.runJsonCli<Snapshot & Counted>(['snapshot', fixture.pane])
      );
      expect(scoped.commandCount).toBe(2);
      expect(scoped.panes.map((pane) => pane.id)).toEqual([fixture.pane]);
      const empty = expectJsonResult(await fixture.runJsonCli<Snapshot & Counted>(['server']));
      expect(empty.commandCount).toBe(2);
      expect(empty.panes).toEqual([]);
      const missing = expectJsonResult(
        await fixture.runJsonCli<Snapshot & Counted>(['snapshot', '%999999'])
      );
      expect(missing.commandCount).toBe(3);
      expect(missing.panes).toEqual([]);
      expect(missing.server).toEqual(full.server);
      const target = expectJsonResult(
        await fixture.runJsonCli<Counted & { pane: string }>([
          'target',
          fixture.paneTarget(fixture.pane),
        ])
      );
      expect(target).toEqual({ pane: fixture.pane, commandCount: 1 });
    }, fixtureOptions());
  }, 20_000);

  it.each(['grouped', 'linked'] as const)(
    'deduplicates %s rows and prefers a real attached target',
    async (mode) => {
      await withE2EFixture(async (fixture) => {
        if (mode === 'grouped') fixture.tmux(['new-session', '-d', '-t', 'e2e', '-s', 'second']);
        else {
          fixture.tmux(['new-session', '-d', '-s', 'second', 'sleep', '300']);
          fixture.tmux(['link-window', '-s', 'e2e:0', '-t', 'second:']);
        }
        await fixture.attachSessionClient('second');
        const rows = fixture
          .tmux([
            'list-panes',
            '-a',
            '-F',
            '#{pane_id}|#{session_name}:#{window_index}.#{pane_index}|#{session_attached}',
          ])
          .trim()
          .split('\n')
          .map((line) => line.split('|'))
          .filter(([id]) => id === fixture.pane);
        expect(rows).toHaveLength(2);
        expect(rows[0]?.[2]).toBe('0');
        const attached = rows.find((row) => Number(row[2]) > 0);
        expect(attached).toBeDefined();
        for (const args of [['snapshot'], ['snapshot', fixture.pane]]) {
          const snapshot = expectJsonResult(await fixture.runJsonCli<Snapshot & Counted>(args));
          const panes = snapshot.panes.filter((pane) => pane.id === fixture.pane);
          expect(panes).toHaveLength(1);
          expect(panes[0]).toMatchObject({ panePid: fixture.panePid, target: attached?.[1] });
        }
      }, fixtureOptions());
    },
    20_000
  );

  it('preserves opaque metadata and only clears the selected binding marker', async () => {
    await withE2EFixture(async (fixture) => {
      const snapshot = expectJsonResult(await fixture.runJsonCli<Snapshot>(['server']));
      const opaque = { version: 1, future: { literal: '__TMT_FIELD_4f1c__', enabled: true } };
      fixture.tmux([
        'set-option',
        '-p',
        '-t',
        fixture.pane,
        '@tmux-team.agent',
        JSON.stringify(opaque),
      ]);
      const published = expectJsonResult(
        await fixture.runJsonCli<Counted & { changed: boolean }>([
          'set-marker',
          fixture.pane,
          'Alice',
          'alice',
          'identity-1',
          'binding-1',
          snapshot.server.serverId,
          String(fixture.panePid),
        ])
      );
      expect(published).toEqual({ changed: true, commandCount: 2 });
      const expected = {
        ...opaque,
        globalIdentity: {
          name: 'Alice',
          canonicalName: 'alice',
          identityId: 'identity-1',
          bindingId: 'binding-1',
          serverId: snapshot.server.serverId,
          panePid: fixture.panePid,
        },
      };
      expect(JSON.parse(fixture.paneMetadata())).toEqual(expected);
      const observed = expectJsonResult(
        await fixture.runJsonCli<Snapshot>(['snapshot', fixture.pane])
      );
      expect(observed.panes[0]?.marker).toEqual(expected.globalIdentity);
      const before = fixture.paneMetadata();
      expect(
        expectJsonResult(await fixture.runJsonCli(['clear-marker', fixture.pane, 'wrong']))
      ).toEqual({ changed: false, commandCount: 1 });
      expect(fixture.paneMetadata()).toBe(before);
      expect(
        expectJsonResult(await fixture.runJsonCli(['clear-marker', fixture.pane, 'binding-1']))
      ).toEqual({ changed: true, commandCount: 2 });
      expect(JSON.parse(fixture.paneMetadata())).toEqual(opaque);
      // tmux show-options -q may treat a missing pane as empty metadata. An
      // already-absent marker is an idempotent no-op, not proof of an IO error.
      expect(
        expectJsonResult(await fixture.runJsonCli(['clear-marker', '%999999', 'binding-1']))
      ).toEqual({ changed: false, commandCount: 1 });
      const failedWrite = await fixture.runJsonCli([
        'set-marker',
        '%999999',
        'Alice',
        'alice',
        'identity-1',
        'binding-1',
        snapshot.server.serverId,
        String(fixture.panePid),
      ]);
      expect(failedWrite.code).toBe(1);
      expect(failedWrite.json).toMatchObject({
        error: {
          code: 'TMUX_ERROR',
          message: expect.stringContaining('Could not write pane metadata'),
        },
      });
      const failedRead = await fixture.runJsonCli(['clear-marker', fixture.pane, 'binding-1'], {
        withoutTmux: true,
      });
      expect(failedRead.code).toBe(1);
      expect(failedRead.json).toMatchObject({
        error: {
          code: 'TMUX_ERROR',
          message: expect.stringContaining('Could not read pane metadata'),
        },
        commandCount: 1,
      });
      expect(JSON.parse(fixture.paneMetadata())).toEqual(opaque);
      expect(fs.existsSync(path.join(fixture.globalDir, 'tmux-team.db'))).toBe(false);
    }, fixtureOptions());
  });

  it('distinguishes live, inaccessible and reaped foreign servers without mutating them', async () => {
    await withE2EFixture(async (local) => {
      let deadSocket = '';
      let deadPid = 0;
      await withE2EFixture(async (foreign) => {
        const expected = expectJsonResult(await foreign.runJsonCli<Snapshot>(['snapshot']));
        const probe = expectJsonResult(
          await local.runJsonCli<Counted & { status: string; snapshot: Snapshot }>([
            'probe',
            foreign.socketPath,
            String(foreign.serverPid),
          ])
        );
        expect(probe.status).toBe('live');
        expect(probe.commandCount).toBe(1);
        expect(probe.snapshot).toEqual({ server: expected.server, panes: expected.panes });
        expect(probe.snapshot.server.socketPath).not.toBe(local.socketPath);
        const unknown = expectJsonResult(
          await local.runJsonCli<Counted & { status: string }>([
            'probe',
            `${foreign.socketPath}.missing`,
            String(foreign.serverPid),
          ])
        );
        expect(unknown).toEqual({ status: 'unknown', commandCount: 1 });
        expect(foreign.paneMetadata()).toBe('');
        deadSocket = foreign.socketPath;
        deadPid = foreign.serverPid;
      }, fixtureOptions());
      const dead = expectJsonResult(
        await local.runJsonCli<Counted & { status: string }>(['probe', deadSocket, String(deadPid)])
      );
      expect(dead).toEqual({ status: 'dead', commandCount: 1 });
    }, fixtureOptions());
  });
});
