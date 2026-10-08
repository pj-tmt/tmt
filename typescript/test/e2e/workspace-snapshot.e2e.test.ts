import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { describe, expect, it } from 'vite-plus/test';
import { withE2EFixture, type E2EFixture } from './harness.js';
import { writeExecutable } from '../support/executable-fixture.mjs';

type Snapshot = {
  version: number;
  server: { socket: string; process: { pid: number; start: string } };
  sessions: Array<{
    id: string;
    name: string;
    windows: Array<{ index: number; window: string; active: boolean }>;
  }>;
  windows: Array<{
    id: string;
    layout: string;
    visibleLayout: string;
    width: number;
    height: number;
    activePane: string;
  }>;
  panes: Array<{
    id: string;
    cwd: string;
    identity: null | { id: string; name: string; lifetime: string; binding: string };
    command: null | { argv: string[]; owner: { pid: number; start: string } };
  }>;
};
const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;
const snapshotPath = (fixture: E2EFixture) =>
  path.join(
    fixture.globalDir,
    'workspace',
    createHash('sha256').update(fixture.socketPath).digest('hex'),
    'latest.json'
  );
const readSnapshot = (fixture: E2EFixture) =>
  JSON.parse(fs.readFileSync(snapshotPath(fixture), 'utf8')) as Snapshot;

describe('event-driven workspace recovery snapshots', () => {
  it('captures linked layouts and exact identities, then reflects committed unbind', async () => {
    await withE2EFixture(async (fixture) => {
      fixture.tmux(['new-session', '-d', '-s', 'linked']);
      fixture.tmux(['link-window', '-s', 'e2e:0', '-t', 'linked:9']);
      const bound = await fixture.runJsonCli<{ id: string }>(['name', 'Snapshot Seat', '-s']);
      expect(bound.code, bound.stderr).toBe(0);
      const snapshot = readSnapshot(fixture);
      expect(snapshot.version).toBe(1);
      expect(snapshot.server.socket).toBe(fixture.socketPath);
      expect(snapshot.server.process.pid).toBe(fixture.serverPid);
      expect(snapshot.server.process.start).not.toBe('');
      const pane = snapshot.panes.find((pane) => pane.id === fixture.pane)!;
      expect(pane.identity).toMatchObject({
        id: bound.json!.id,
        name: 'Snapshot Seat',
        lifetime: 'saved',
      });
      expect(pane.cwd).toBe(fixture.workspace);
      expect(pane.command).toBeNull();
      const links = snapshot.sessions
        .flatMap((session) => session.windows)
        .filter(
          (link) =>
            snapshot.windows.find((window) => window.id === link.window)?.activePane ===
            fixture.pane
        );
      expect(links).toHaveLength(2);
      expect(links[0].window).toBe(links[1].window);
      expect(snapshot.windows.filter((window) => window.id === links[0].window)).toHaveLength(1);
      expect(
        snapshot.windows.every(
          (window) => window.layout && window.visibleLayout && window.width > 0 && window.height > 0
        )
      ).toBe(true);
      const mode = fs.statSync(snapshotPath(fixture)).mode & 0o777;
      expect(mode).toBe(0o600);
      expect((await fixture.runJsonCli(['unbind'])).code).toBe(0);
      expect(
        readSnapshot(fixture).panes.find((pane) => pane.id === fixture.pane)!.identity
      ).toBeNull();
      expect((await fixture.runJsonCli(['identity', 'show', 'Snapshot Seat'])).json).toMatchObject({
        id: bound.json!.id,
        lifetime: 'saved',
      });
    });
  });

  it('keeps bytes while disabled and captures events even with interval zero', async () => {
    await withE2EFixture(async (fixture) => {
      expect((await fixture.runJsonCli(['name', 'Snapshot Policy', '-s'])).code).toBe(0);
      const before = fs.readFileSync(snapshotPath(fixture));
      expect(
        (
          await fixture.runJsonCli([
            'config',
            'set',
            '--global',
            'workspace.snapshotEnabled',
            'false',
          ])
        ).code
      ).toBe(0);
      expect((await fixture.runJsonCli(['unbind'])).code).toBe(0);
      expect(fs.readFileSync(snapshotPath(fixture))).toEqual(before);
      expect(
        (
          await fixture.runJsonCli([
            'config',
            'set',
            '--global',
            'workspace.snapshotIntervalMs',
            '0',
          ])
        ).code
      ).toBe(0);
      expect(
        (
          await fixture.runJsonCli([
            'config',
            'set',
            '--global',
            'workspace.snapshotEnabled',
            'true',
          ])
        ).code
      ).toBe(0);
      expect((await fixture.runJsonCli(['name', 'Snapshot Policy'])).code).toBe(0);
      expect((await fixture.runJsonCli(['unbind'])).code).toBe(0);
      expect(
        readSnapshot(fixture).panes.find((pane) => pane.id === fixture.pane)!.identity
      ).toBeNull();
    });
  });

  it('records arbitrary foreground extensions with literal args and excludes an ended owner', async () => {
    await withE2EFixture(async (fixture) => {
      const directory = path.join(fixture.root, 'extension-bin');
      fs.mkdirSync(directory);
      const ready = path.join(fixture.root, 'extension-ready');
      writeExecutable(
        path.join(directory, 'tmt-workspace-example'),
        `#!/bin/sh\nprintf '%s' "$$" > ${quote(`${ready}.tmp`)}\nmv ${quote(`${ready}.tmp`)} ${quote(ready)}\nread -r release\n`
      );
      const command = [
        'env',
        `PATH=${directory}:${process.env.PATH}`,
        `TMUX_TEAM_HOME=${fixture.globalDir}`,
        fixture.executables.cli.executable,
        'workspace-example',
        'ui',
        '--tabs=agents,requests',
        'literal $(data)',
      ]
        .map(quote)
        .join(' ');
      const pane = fixture
        .tmux([
          'new-window',
          '-d',
          '-P',
          '-F',
          '#{pane_id}',
          '-t',
          'e2e',
          '-n',
          'external',
          command,
        ])
        .trim();
      await fixture.waitFor(() => fs.existsSync(ready), 5000, 'external foreground ready');
      const owner = Number(fs.readFileSync(ready, 'utf8'));
      expect(readSnapshot(fixture).panes.find((value) => value.id === pane)?.command).toMatchObject(
        {
          argv: ['tmt', 'workspace-example', 'ui', '--tabs=agents,requests', 'literal $(data)'],
          owner: { pid: owner },
        }
      );
      expect((await fixture.runJsonCli(['name', 'Snapshot Trigger'])).code).toBe(0);
      expect(
        readSnapshot(fixture).panes.find((value) => value.id === pane)?.command?.owner.pid
      ).toBe(owner);
      fixture.tmux(['send-keys', '-t', pane, 'release', 'Enter']);
      await fixture.waitFor(() => !fs.existsSync(`/proc/${owner}`), 5000, 'external owner ended');
      expect((await fixture.runJsonCli(['unbind'])).code).toBe(0);
      expect(readSnapshot(fixture).panes.some((value) => value.command?.owner.pid === owner)).toBe(
        false
      );
    });
  });
});
