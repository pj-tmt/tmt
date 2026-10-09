import fs from 'node:fs';
import path from 'node:path';
import net from 'node:net';
import { createHash } from 'node:crypto';
import Database from 'better-sqlite3';
import { describe, expect, it } from 'vite-plus/test';
import { withE2EFixture, type E2EFixture } from './harness.js';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { waitForFileContent } from './wait-for-file.js';
import { installTmuxTrace } from './tmux-trace.js';

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
    identity: null | {
      id: string;
      name: string;
      lifetime: string;
      binding: string;
      harness: string | null;
      session: string | null;
    };
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

function submitForeground(
  fixture: E2EFixture,
  pane: string,
  args: readonly string[],
  status: string,
  environment: Record<string, string> = {}
) {
  const command = [
    'env',
    `TMUX_TEAM_HOME=${fixture.globalDir}`,
    ...Object.entries(environment).map(([key, value]) => `${key}=${value}`),
    fixture.executables.cli.executable,
    ...args,
  ]
    .map(quote)
    .join(' ');
  fixture.tmux(['send-keys', '-t', pane, '-l', `${command}; printf '%s' "$?" > ${quote(status)}`]);
  fixture.tmux(['send-keys', '-t', pane, 'Enter']);
}

describe('event-driven workspace recovery snapshots', () => {
  it('previews current durable resume state without changing layout, snapshot or identity data', async () => {
    await withE2EFixture(async (fixture) => {
      const bound = await fixture.runJsonCli<{ id: string }>(['name', 'Preview Old Name', '-s']);
      expect(bound.code, bound.stderr).toBe(0);
      const original = JSON.parse(fs.readFileSync(snapshotPath(fixture), 'utf8')) as Snapshot;
      const synthetic = {
        ...original,
        sessions: [
          ...original.sessions,
          { id: '$999999', name: 'Preview Missing Session', windows: original.sessions[0].windows },
        ],
      };
      expect(
        (await fixture.runJsonCli(['rename', 'Preview Old Name', 'Preview Current Name'])).code
      ).toBe(0);
      // The recovery input intentionally predates the current name/preferences.
      fs.writeFileSync(snapshotPath(fixture), JSON.stringify(synthetic));
      const database = new Database(path.join(fixture.globalDir, 'tmux-team.db'));
      try {
        database
          .prepare(`INSERT INTO identity_session_preferences
          (identity_id, preferred_harness, remembered_harness, runtime_mode, provider_session_id, resume_pending_at_ms, channel)
          VALUES (?, 'codex', 'codex', 'independent', 'current-conversation', 2, 1)
          ON CONFLICT(identity_id) DO UPDATE SET preferred_harness='codex', remembered_harness='codex', runtime_mode='independent',
          provider_session_id='current-conversation', resume_pending_at_ms=2, channel=1`)
          .run(bound.json!.id);
        const before = database
          .prepare('SELECT * FROM identity_session_preferences WHERE identity_id=?')
          .get(bound.json!.id);
        const bytes = fs.readFileSync(snapshotPath(fixture));
        const topology = () =>
          fixture.tmux([
            'list-panes',
            '-a',
            '-F',
            '#{session_id}:#{window_id}:#{window_layout}:#{pane_id}:#{pane_pid}:#{pane_current_path}',
          ]);
        const beforeTopology = topology();
        const trace = installTmuxTrace(fixture);
        trace.clear();
        const result = await fixture.runJsonCli<{
          snapshot: Snapshot;
          sessions: Array<{ session: string; action: string }>;
          panes: Array<{
            pane: string;
            action: string;
            identity: { id: string; name: string };
            resume: { session: string; resumePendingAtMs: number };
          }>;
        }>(['workspace', 'show', '--socket', fixture.socketPath], { outsideTmux: true });
        expect(result.code, result.stderr).toBe(0);
        expect(result.json!.snapshot).toEqual(synthetic);
        expect(result.json!.sessions).toEqual([
          ...original.sessions.map((session) => ({ session: session.id, action: 'skip_existing' })),
          { session: '$999999', action: 'create' },
        ]);
        expect(result.json!.panes.find((pane) => pane.pane === fixture.pane)).toMatchObject({
          action: 'resumable',
          identity: { id: bound.json!.id, name: 'Preview Current Name', channel: true },
          resume: { session: 'current-conversation', resumePendingAtMs: 2 },
        });
        expect(trace.commands()).toEqual(['list-sessions']);
        expect(fs.readFileSync(snapshotPath(fixture))).toEqual(bytes);
        expect(topology()).toBe(beforeTopology);
        expect(
          database
            .prepare('SELECT * FROM identity_session_preferences WHERE identity_id=?')
            .get(bound.json!.id)
        ).toEqual(before);
        database
          .prepare('UPDATE identity_session_preferences SET stale_at_ms=3 WHERE identity_id=?')
          .run(bound.json!.id);
        const human = await fixture.runCli(['workspace', 'show', '--socket', fixture.socketPath]);
        expect(human.code, human.stderr).toBe(0);
        expect(human.stdout).toContain('stale remembered session');
        expect(human.stdout).not.toContain('--retry');
        expect(fs.readFileSync(snapshotPath(fixture))).toEqual(bytes);
      } finally {
        database.close();
      }
    });
  });

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
        identity: { id: bound.json!.id, lifetime: 'saved' },
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
      expect((await fixture.runJsonCli(['name', 'Before Dispatch'])).code).toBe(0);
      const directory = path.join(fixture.root, 'extension-bin');
      fs.mkdirSync(directory);
      const ready = path.join(fixture.root, 'extension-ready');
      const gate = path.join(fixture.socketRoot, 'extension-release.sock');
      const done = path.join(fixture.root, 'extension-ended');
      writeExecutable(
        path.join(directory, 'tmt-workspace-example'),
        `#!/usr/bin/env node
const fs = require('node:fs');
const net = require('node:net');
const publish = (file, text) => {
  fs.writeFileSync(file + '.tmp', text);
  fs.renameSync(file + '.tmp', file);
};
const server = net.createServer((peer) => {
  let request = '';
  peer.on('data', (bytes) => { request += bytes.toString(); });
  peer.once('end', () => {
    if (request !== 'release') process.exit(97);
    peer.end('released');
    server.close(() => {
      publish(${JSON.stringify(done)}, 'released');
      process.exit(0);
    });
  });
});
server.listen(${JSON.stringify(gate)}, () => {
  publish(${JSON.stringify(ready)}, String(process.pid));
});
`
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
      const previousSnapshot = fs.readFileSync(snapshotPath(fixture));
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
          '-c',
          fixture.workspace,
          `${command}; exec /bin/sh`,
        ])
        .trim();
      // Keep a live shell after the extension exits. A dead remain-on-exit
      // pane has no current cwd and cannot form a valid recovery snapshot.
      const panePid = fixture.tmux(['display-message', '-p', '-t', pane, '#{pane_pid}']).trim();
      await fixture.waitFor(() => fs.existsSync(ready), 5000, 'external foreground ready');
      const owner = Number(fs.readFileSync(ready, 'utf8'));
      // Dispatch writes only an advisory marker. Snapshot IO waits for the next
      // eligible event, which independently verifies native owner evidence.
      expect(fs.readFileSync(snapshotPath(fixture))).toEqual(previousSnapshot);
      // Repeating this exact binding is an admitted native event; selecting a
      // different identity would be a real binding conflict.
      expect((await fixture.runJsonCli(['name', 'Before Dispatch'])).code).toBe(0);
      expect(readSnapshot(fixture).panes.find((value) => value.id === pane)?.command).toMatchObject(
        {
          argv: ['tmt', 'workspace-example', 'ui', '--tabs=agents,requests', 'literal $(data)'],
          owner: { pid: owner },
        }
      );
      expect(
        readSnapshot(fixture).panes.find((value) => value.id === pane)?.command?.owner.pid
      ).toBe(owner);
      // Acknowledged IPC proves the fake consumed its release. Every completion
      // observation shares the original five-second exit budget.
      const exitDeadline = Date.now() + 5000;
      const remaining = () => Math.max(0, exitDeadline - Date.now());
      const release = net.createConnection(gate);
      let response = '';
      let closed = false;
      let failure: Error | undefined;
      release.once('connect', () => release.end('release'));
      release.on('data', (bytes) => {
        response += bytes.toString();
      });
      release.once('error', (error) => {
        failure = error;
      });
      release.once('close', () => {
        closed = true;
      });
      try {
        await fixture.waitFor(() => closed, remaining(), 'external release acknowledged');
        if (failure) throw failure;
        expect(response).toBe('released');
        expect(await waitForFileContent(done, { timeoutMs: remaining() })).toBe('released');
        await fixture.waitFor(
          () => !fs.existsSync(`/proc/${owner}`),
          remaining(),
          'external owner ended'
        );
        await fixture.waitFor(
          () =>
            fixture
              .tmux([
                'display-message',
                '-p',
                '-t',
                pane,
                '#{pane_pid}|#{pane_dead}|#{pane_current_command}|#{pane_current_path}',
              ])
              .trim() === `${panePid}|0|sh|${fixture.workspace}`,
          remaining(),
          'surviving shell owns the same recoverable pane'
        );
      } finally {
        release.destroy();
      }
      expect((await fixture.runJsonCli(['unbind'])).code).toBe(0);
      const refreshed = readSnapshot(fixture);
      expect(refreshed.panes.find((value) => value.id === fixture.pane)?.identity).toBeNull();
      expect(refreshed.panes.find((value) => value.id === pane)?.cwd).toBe(fixture.workspace);
      expect(refreshed.panes.some((value) => value.command?.owner.pid === owner)).toBe(false);
    });
  });

  it('captures launch and resume events and admits only session boundaries for hook capture', async () => {
    await withE2EFixture(async (fixture) => {
      const pane = fixture.createShellPane('snapshot-provider').pane;
      const session = '33333333-3333-4333-8333-333333333333';
      const scenario = path.join(fixture.root, 'provider-scenario.json');
      const report = path.join(fixture.root, 'provider-report.json');
      const checkpoint = path.join(fixture.root, 'provider-checkpoint');
      const home = path.join(fixture.root, 'provider-home');
      fs.mkdirSync(home);
      const step = (event: string, source: string) => ({
        // Inspect the admitted worker result independently of optional parent
        // capture headroom. The Rust output test exercises its bounded host port.
        args: ['__hook', 'claude', '--worker', '--work-budget-ms', '2000'],
        input: {
          hook_event_name: event,
          session_id: session,
          ...(event === 'SessionStart' ? { source } : { reason: source }),
        },
      });
      const writeScenario = (source: string) => {
        fs.writeFileSync(
          scenario,
          JSON.stringify([
            { args: ['whoami', '--json'], checkpoint },
            step('SessionStart', source),
            { args: ['whoami', '--json'] },
            step('UserPromptSubmit', ''),
            step('Stop', ''),
            step('SessionEnd', 'other'),
          ])
        );
      };
      writeExecutable(
        path.join(fixture.wrapperDir, 'claude'),
        `#!/bin/sh\nexec /opt/tmt-tests/claude ${quote(fixture.executables.cli.executable)} ${quote(scenario)} ${quote(report)}\n`
      );
      for (const [source, args] of [
        ['startup', ['run', '-s', 'Snapshot Runner', 'claude']],
        ['resume', ['resume', 'Snapshot Runner']],
      ] as const) {
        writeScenario(source);
        fs.rmSync(checkpoint, { force: true });
        fs.rmSync(report, { force: true });
        const status = path.join(fixture.root, `${source}.status`);
        submitForeground(fixture, pane, args, status, { HOME: home });
        expect(
          await waitForFileContent(checkpoint, { description: `${source} provider ready` })
        ).toBe('ready');
        // The scripted child holds before any CLI callback. Wait for the native
        // launch admission independently, as the normal hook parent would do.
        const db = new Database(path.join(fixture.globalDir, 'tmux-team.db'), { readonly: true });
        try {
          await fixture.waitFor(
            () => {
              const state = db
                .prepare('SELECT runtime_state FROM bindings WHERE pane_id = ?')
                .get(pane) as { runtime_state: string } | undefined;
              return state?.runtime_state === 'running';
            },
            5000,
            `${source} native launch admitted`
          );
        } finally {
          db.close();
        }
        // No hook parent has captured yet: this is the pre-spawn launch event.
        expect(
          readSnapshot(fixture).panes.find((value) => value.id === pane)?.identity
        ).toMatchObject({
          name: 'Snapshot Runner',
          lifetime: 'saved',
          session: source === 'startup' ? null : session,
        });
        fs.writeFileSync(checkpoint, 'continue');
        expect(
          await waitForFileContent(status, {
            // Six finite CLI calls run after the checkpoint, including four
            // admitted workers with their existing two-second work budgets.
            timeoutMs: 10_000,
            description: `${source} provider exit`,
          })
        ).toBe('0');
        const results = JSON.parse(fs.readFileSync(report, 'utf8')) as Array<{
          code: number;
          stdout: string;
          stderr: string;
        }>;
        expect(results).toHaveLength(6);
        expect(results.every((value) => value.code === 0 && value.stderr === '')).toBe(true);
        const identity = JSON.parse(results[2].stdout);
        expect(
          JSON.parse(JSON.parse(results[1].stdout).context).hookSpecificOutput.additionalContext
        ).toContain(identity.id);
        expect(JSON.parse(results[5].stdout).context).toBe('');
        for (const index of [1, 5]) {
          expect(JSON.parse(results[index].stdout).workspace).toMatchObject({
            socket: fixture.socketPath,
            pid: fixture.serverPid,
          });
        }
        for (const index of [3, 4]) {
          expect(JSON.parse(results[index].stdout).workspace).toBeNull();
        }
        // Completion capture independently reflects the durable hook session.
        expect(
          readSnapshot(fixture).panes.find((value) => value.id === pane)?.identity
        ).toMatchObject({
          id: identity.id,
          harness: 'claude',
          session,
        });
      }
    });
  });

  it('clears the exact external marker after failed exec and preserves the bound identity', async () => {
    await withE2EFixture(async (fixture) => {
      const pane = fixture.createShellPane('snapshot-exec-failure').pane;
      const bound = await fixture.runJsonCli<{ id: string }>(['name', 'Snapshot Failure', '-s'], {
        pane,
      });
      expect(bound.code, bound.stderr).toBe(0);
      const directory = path.join(fixture.root, 'broken-extension');
      fs.mkdirSync(directory);
      writeExecutable(
        path.join(directory, 'tmt-workspace-broken'),
        '#!/nonexistent-workspace-interpreter\n'
      );
      // Absence cannot pass if marker preparation silently skipped: failed exec
      // can clear only the exact marker it replaced this old value with.
      fixture.tmux(['set-option', '-p', '-t', pane, '@tmt.workspace-command', 'previous-dispatch']);
      const status = path.join(fixture.root, 'failed-exec.status');
      submitForeground(fixture, pane, ['workspace-broken', 'literal argument'], status, {
        PATH: `${directory}:${process.env.PATH}`,
      });
      expect(
        await waitForFileContent(status, { description: 'failed extension exec settled' })
      ).toBe('1');
      expect(
        fixture.tmux(['show-options', '-p', '-qv', '-t', pane, '@tmt.workspace-command']).trim()
      ).toBe('');
      const recorded = readSnapshot(fixture).panes.find((value) => value.id === pane)!;
      expect(recorded.command).toBeNull();
      expect(recorded.identity).toMatchObject({ id: bound.json!.id, name: 'Snapshot Failure' });
      expect((await fixture.runJsonCli(['whoami'], { pane })).json).toMatchObject({
        id: bound.json!.id,
      });
    });
  });
});
