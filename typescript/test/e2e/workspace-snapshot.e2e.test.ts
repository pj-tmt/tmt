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
  capturedAtMs: number;
  server: { socket: string; process: { pid: number; start: string } };
  sessions: Array<{
    id: string;
    name: string;
    windows: Array<{ index: number; window: string; active: boolean }>;
  }>;
  windows: Array<{
    id: string;
    name: string;
    layout: string;
    visibleLayout: string;
    width: number;
    height: number;
    activePane: string;
  }>;
  panes: Array<{
    id: string;
    window: string;
    index: number;
    left: number;
    top: number;
    width: number;
    height: number;
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
    `TMT_HOME=${fixture.globalDir}`,
    ...Object.entries(environment).map(([key, value]) => `${key}=${value}`),
    fixture.executables.cli.executable,
    ...args,
  ]
    .map(quote)
    .join(' ');
  fixture.tmux(['send-keys', '-t', pane, '-l', `${command}; printf '%s' "$?" > ${quote(status)}`]);
  fixture.tmux(['send-keys', '-t', pane, 'Enter']);
}

// New workspace lifecycle/cost cases additionally fence every call by the exact
// fixture socket, and require a recorded created-server incarnation for cleanup.
function guardWorkspaceServer(fixture: E2EFixture) {
  const socket = fixture.socketPath;
  const invoke = fixture.tmux.bind(fixture);
  let incarnation = '';
  const assertPrivate = () => {
    const relative = path.relative(fixture.socketRoot, socket);
    expect(path.isAbsolute(socket)).toBe(true);
    expect(relative.length).toBeGreaterThan(0);
    expect(path.isAbsolute(relative) || relative.startsWith('..')).toBe(false);
    expect(path.basename(socket)).not.toBe('default');
    expect(socket).not.toBe(`/private/tmp/tmux-${process.getuid!()}/default`);
    expect(socket).not.toBe(
      path.join(process.env.TMUX_TMPDIR ?? '/tmp', `tmux-${process.getuid!()}`, 'default')
    );
  };
  const createdServer = () => {
    assertPrivate();
    incarnation = invoke(['-S', socket, 'display-message', '-p', '#{pid}|#{start_time}']).trim();
    expect(Number(incarnation.split('|')[0])).toBe(fixture.serverPid);
  };
  createdServer();
  fixture.tmux = (args) => {
    assertPrivate();
    if (args.includes('kill-server'))
      expect(invoke(['-S', socket, 'display-message', '-p', '#{pid}|#{start_time}']).trim()).toBe(
        incarnation
      );
    return invoke(['-S', socket, ...args]);
  };
  return { createdServer };
}

describe('event-driven workspace recovery snapshots', () => {
  it('retains the pre-crash generation after restart and restores that protected layout by default', async () => {
    await withRestoreFixture(async (fixture) => {
      const socket = fixture.socketPath;
      const guard = guardWorkspaceServer(fixture);
      const panes = [fixture.pane];
      for (let index = 1; index < 4; index++)
        panes.push((await fixture.createMockPane(`recovery-${index}`)).pane);
      fixture.tmux(['rename-session', '-t', 'e2e', 'pre-crash']);
      for (const [index, pane] of panes.entries()) {
        const named = await fixture.runJsonCli(['name', `Recovery ${index}`, '-s'], { pane });
        expect(named.code, named.stderr + named.stdout).toBe(0);
      }
      const rich = readSnapshot(fixture);
      expect(rich.panes).toHaveLength(4);
      expect(rich.panes.filter((pane) => pane.identity)).toHaveLength(4);
      const richBytes = fs.readFileSync(snapshotPath(fixture));
      const digest = createHash('sha256').update(richBytes).digest('hex');
      const richFile = path.join(path.dirname(snapshotPath(fixture)), `auto-${digest}.json`);
      expect(fs.readFileSync(richFile)).toEqual(richBytes);
      const oldPid = fixture.serverPid;
      await fixture.restartServer();
      guard.createdServer();
      expect(fixture.serverPid).not.toBe(oldPid);
      const named = await fixture.runJsonCli(['name', 'After Restart', '-s']);
      expect(named.code, named.stderr + named.stdout).toBe(0);
      expect(readSnapshot(fixture).panes).toHaveLength(1);
      const listing = await fixture.runJsonCli<{
        snapshots: Array<{
          generation: string;
          panes: number;
          agents: number;
          protected: boolean;
          shrink: boolean;
        }>;
      }>(['workspace', 'ls', '--socket', socket]);
      expect(listing.code, listing.stderr + listing.stdout).toBe(0);
      expect(listing.json!.snapshots[0]).toMatchObject({
        panes: 1,
        agents: 1,
        shrink: true,
        protected: false,
      });
      expect(
        listing.json!.snapshots.find((entry) => entry.generation === `g:${digest}`)
      ).toMatchObject({ panes: 4, agents: 4, protected: true, shrink: false });
      expect(fs.readFileSync(richFile)).toEqual(richBytes);
      const preview = await fixture.runJsonCli<{ generation: string; snapshot: Snapshot }>(
        ['workspace', 'show', '--socket', socket],
        { outsideTmux: true }
      );
      expect(preview.code, preview.stderr + preview.stdout).toBe(0);
      expect(preview.json!.generation).toBe(`g:${digest}`);
      expect(preview.json!.snapshot).toEqual(rich);
      const restored = await fixture.runJsonCli<LayoutRestore & { generation: string }>(
        ['workspace', 'restore', '--layout-only', '--socket', socket],
        { outsideTmux: true }
      );
      expect(restored.code, restored.stderr + restored.stdout).toBe(0);
      expect(restored.json!.generation).toBe(`g:${digest}`);
      expect(restored.json!.sessions).toContainEqual(
        expect.objectContaining({ name: 'pre-crash', action: 'created' })
      );
      expect(
        fixture
          .tmux(['list-panes', '-t', 'pre-crash', '-a', '-F', '#{session_name}'])
          .trim()
          .split('\n')
          .filter((name) => name === 'pre-crash')
      ).toHaveLength(4);
      expect(fs.readFileSync(richFile)).toEqual(richBytes);
    });
  });

  it('measures unchanged capture cost on 32 panes including child CPU, one topology read and no recovery rewrite', async () => {
    await withRestoreFixture(async (fixture) => {
      guardWorkspaceServer(fixture);
      const bound = [fixture.pane];
      for (let index = 1; index < 8; index++)
        bound.push((await fixture.createMockPane(`meter-agent-${index}`)).pane);
      for (let index = 8; index < 31; index++) fixture.createShellPane(`meter-shell-${index}`);
      const meter = fixture.createShellPane('capture-meter').pane;
      for (const [index, pane] of bound.entries())
        expect((await fixture.runJsonCli(['name', `Meter ${index}`, '-s'], { pane })).code).toBe(0);
      expect(readSnapshot(fixture).panes).toHaveLength(32);
      const directory = path.dirname(snapshotPath(fixture));
      const recovery = () =>
        fs
          .readdirSync(directory)
          .filter((name) => name !== 'capture-attempt.json')
          .sort()
          .map((name) => {
            const filename = path.join(directory, name);
            const metadata = fs.statSync(filename);
            return {
              name,
              bytes: fs.readFileSync(filename).toString('base64'),
              inode: metadata.ino,
              mtimeMs: metadata.mtimeMs,
            };
          });
      const before = recovery();
      const samples = 12;
      const script = path.join(fixture.root, 'capture-cost.py');
      const result = path.join(fixture.root, 'capture-cost.json');
      const status = path.join(fixture.root, 'capture-cost.status');
      const psLog = path.join(fixture.root, 'capture-ps.log');
      const ps = path.join(fixture.wrapperDir, 'ps');
      expect(fs.existsSync(ps)).toBe(false);
      writeExecutable(
        ps,
        `#!/bin/sh\nprintf '%s\\n' "$*" >> ${quote(psLog)}\nexec /usr/bin/ps "$@"\n`
      );
      fs.writeFileSync(
        script,
        `
import json, os, pathlib, resource, subprocess, sys, time
root=pathlib.Path(${JSON.stringify(directory)})
result=pathlib.Path(${JSON.stringify(result)})
env=dict(os.environ)
env.pop('TMUX',None);env.pop('TMUX_PANE',None)
env['TMT_HOME']=${JSON.stringify(fixture.globalDir)}
command=${JSON.stringify([fixture.executables.cli.executable, ...fixture.executables.cli.args, 'api'])}
request=json.dumps({'version':1,'operation':'workspace.capture','input':{}})
if len(sys.argv)>1 and sys.argv[1]=='changed':
    completed=subprocess.run(command,input=request,text=True,capture_output=True,env=env,timeout=3)
    assert completed.returncode==0,(completed.returncode,completed.stdout,completed.stderr)
    assert completed.stderr=='',completed.stderr
    assert json.loads(completed.stdout)=={'attempted':1,'published':1,'skipped':0},completed.stdout
    positive=pathlib.Path(str(result)+'.positive')
    stage=pathlib.Path(str(positive)+'.tmp');stage.write_text(completed.stdout);stage.replace(positive)
    raise SystemExit(0)
samples=[]
for index in range(${samples}):
    stamp=root/'capture-attempt.json'
    if stamp.exists():
        value=json.loads(stamp.read_text());value['attemptedAtMs']=0
        stamp.write_text(json.dumps(value))
    before=resource.getrusage(resource.RUSAGE_CHILDREN);start=time.monotonic()
    completed=subprocess.run(command,input=request,text=True,capture_output=True,env=env,timeout=3)
    elapsed=time.monotonic()-start;after=resource.getrusage(resource.RUSAGE_CHILDREN)
    assert completed.returncode==0,(completed.returncode,completed.stdout,completed.stderr)
    assert completed.stderr=='',completed.stderr
    assert json.loads(completed.stdout)=={'attempted':1,'published':0,'skipped':0},completed.stdout
    samples.append({'cpuMs':1000*((after.ru_utime+after.ru_stime)-(before.ru_utime+before.ru_stime)),'wallMs':1000*elapsed})
value={'panes':32,'agents':8,'periodMs':30000,'samples':samples}
stage=pathlib.Path(str(result)+'.tmp');stage.write_text(json.dumps(value));stage.replace(result)
`
      );
      const trace = installTmuxTrace(fixture);
      trace.clear();
      const command = ['env', `TMT_HOME=${fixture.globalDir}`, 'python3', script]
        .map(quote)
        .join(' ');
      fixture.tmux([
        'send-keys',
        '-t',
        meter,
        '-l',
        `${command}; printf '%s' "$?" > ${quote(status)}`,
      ]);
      fixture.tmux(['send-keys', '-t', meter, 'Enter']);
      expect(
        await waitForFileContent(status, {
          timeoutMs: 20_000,
          description: 'isolated capture CPU samples',
        })
      ).toBe('0');
      const measured = JSON.parse(fs.readFileSync(result, 'utf8')) as {
        samples: Array<{ cpuMs: number; wallMs: number }>;
      };
      expect(measured.samples).toHaveLength(samples);
      const cpu = measured.samples.map((sample) => sample.cpuMs);
      const mean = cpu.reduce((sum, value) => sum + value, 0) / samples;
      console.log(
        'workspace capture cost',
        JSON.stringify({
          panes: 32,
          agents: 8,
          samples,
          meanCpuMs: mean,
          maxCpuMs: Math.max(...cpu),
          cpuPercentAt30s: mean / 300,
          targetPercent: 0.2,
        })
      );
      expect(
        Math.max(...cpu),
        'each due attempt must use less than 1% of one CPU at 30s'
      ).toBeLessThan(300);
      expect(trace.commands().filter((command) => command === 'list-panes')).toHaveLength(samples);
      expect(fs.existsSync(psLog) ? fs.readFileSync(psLog, 'utf8') : '').toBe('');
      expect(recovery()).toEqual(before);
      // An attempted callback can fail silently. A changed-layout positive
      // control proves this same real API path can acquire and publish content.
      fixture.tmux(['rename-window', '-t', meter, 'capture-publish-control']);
      const stamp = path.join(directory, 'capture-attempt.json');
      const attempt = JSON.parse(fs.readFileSync(stamp, 'utf8'));
      fs.writeFileSync(stamp, JSON.stringify({ ...attempt, attemptedAtMs: 0 }));
      const changedCommand = ['env', `TMT_HOME=${fixture.globalDir}`, 'python3', script, 'changed']
        .map(quote)
        .join(' ');
      fixture.tmux(['send-keys', '-t', meter, '-l', changedCommand]);
      fixture.tmux(['send-keys', '-t', meter, 'Enter']);
      expect(
        JSON.parse(
          await waitForFileContent(`${result}.positive`, {
            timeoutMs: 20_000,
            description: 'changed capture publishes through the real API',
          })
        )
      ).toEqual({ attempted: 1, published: 1, skipped: 0 });
      const changedBytes = fs.readFileSync(snapshotPath(fixture));
      expect(changedBytes).not.toEqual(
        Buffer.from(before.find((entry) => entry.name === 'latest.json')!.bytes, 'base64')
      );
      expect(
        readSnapshot(fixture).windows.some((window) => window.name === 'capture-publish-control')
      ).toBe(true);
      const changedDigest = createHash('sha256').update(changedBytes).digest('hex');
      expect(fs.readFileSync(path.join(directory, `auto-${changedDigest}.json`))).toEqual(
        changedBytes
      );
      const catalogue = JSON.parse(
        fs.readFileSync(path.join(directory, 'generations.json'), 'utf8')
      );
      expect(catalogue.entries.at(-1).id).toBe(`g:${changedDigest}`);
      expect(trace.commands().filter((command) => command === 'list-panes')).toHaveLength(
        samples + 1
      );
    });
  });

  it('previews current durable resume state without changing layout, snapshot or identity data', async () => {
    await withE2EFixture(async (fixture) => {
      const bound = await fixture.runJsonCli<{ id: string }>(['name', 'Preview Old Name', '-s']);
      expect(bound.code, bound.stderr + bound.stdout).toBe(0);
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
      recoveryInput(fixture, synthetic);
      const database = new Database(path.join(fixture.globalDir, 'tmt.db'));
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
        expect(result.code, result.stderr + result.stdout).toBe(0);
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
        expect(human.code, human.stderr + human.stdout).toBe(0);
        expect(human.stdout).toContain('stale remembered session');
        expect(human.stdout).not.toContain('--retry');
        expect(fs.readFileSync(snapshotPath(fixture))).toEqual(bytes);
      } finally {
        database.close();
      }
    });
  });

  it('captures Unicode topology under C and preserves recovery state on multiline refusal', async () => {
    await withE2EFixture(async (fixture) => {
      const session = '日本語 café';
      const window = '窓 λ';
      const cwd = path.join(fixture.workspace, '雪 café');
      fs.mkdirSync(cwd);
      const pane = fixture
        .tmux([
          '-u',
          'new-session',
          '-d',
          '-P',
          '-F',
          '#{pane_id}',
          '-s',
          session,
          '-n',
          window,
          '-c',
          cwd,
          'sleep',
          '300',
        ])
        .trim();
      const bound = await fixture.runJsonCli<{ id: string }>(['name', 'C Locale Seat 雪', '-s'], {
        locale: 'C',
      });
      expect(bound.code, bound.stderr + bound.stdout).toBe(0);
      const snapshot = readSnapshot(fixture);
      expect(snapshot.panes.find((value) => value.id === fixture.pane)?.identity).toMatchObject({
        id: bound.json!.id,
        name: 'C Locale Seat 雪',
      });
      const savedSession = snapshot.sessions.find((value) => value.name === session)!;
      expect(savedSession).toBeDefined();
      expect(
        snapshot.windows.find((value) => value.id === savedSession.windows[0].window)?.name
      ).toBe(window);
      expect(snapshot.panes.find((value) => value.id === pane)?.cwd).toBe(cwd);
      const bytes = fs.readFileSync(snapshotPath(fixture));
      const database = new Database(path.join(fixture.globalDir, 'tmt.db'), {
        readonly: true,
      });
      const durable = () => ({
        identities: database.prepare('SELECT * FROM identities ORDER BY id').all(),
        bindings: database.prepare('SELECT * FROM bindings ORDER BY id').all(),
        preferences: database
          .prepare('SELECT * FROM identity_session_preferences ORDER BY identity_id')
          .all(),
      });
      try {
        const before = durable();
        const preview = await fixture.runJsonCli<{
          sessions: Array<{ session: string; action: string }>;
        }>(['workspace', 'show', '--socket', fixture.socketPath], {
          outsideTmux: true,
          locale: 'C',
        });
        expect(preview.code, preview.stderr + preview.stdout).toBe(0);
        expect(preview.json!.sessions).toEqual(
          snapshot.sessions.map((value) => ({ session: value.id, action: 'skip_existing' }))
        );
        expect(fs.readFileSync(snapshotPath(fixture))).toEqual(bytes);
        expect(durable()).toEqual(before);
        const multiline = path.join(fixture.workspace, 'unsupported\npath');
        fs.mkdirSync(multiline);
        fixture.tmux(['new-window', '-d', '-t', savedSession.id, '-c', multiline, 'sleep', '300']);
        // An admitted idempotent binding event attempts capture. Literal multiline
        // topology is unsupported; it must preserve the previous recovery document.
        const repeated = await fixture.runJsonCli(['name', 'C Locale Seat 雪', '-s'], {
          locale: 'C',
        });
        expect(repeated.code, repeated.stderr + repeated.stdout).toBe(0);
        expect(fs.readFileSync(snapshotPath(fixture))).toEqual(bytes);
        const after = durable();
        expect(after.identities).toEqual(before.identities);
        expect(after.preferences).toEqual(before.preferences);
        // The successful binding event owns a verification timestamp touch;
        // unsupported capture must not change binding identity/runtime fields.
        const withoutVerificationTime = (rows: unknown[]) =>
          rows.map((row) => {
            const { last_verified_at: _verified, ...fields } = row as Record<string, unknown>;
            return fields;
          });
        expect(withoutVerificationTime(after.bindings)).toEqual(
          withoutVerificationTime(before.bindings)
        );
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
      expect(bound.code, bound.stderr + bound.stdout).toBe(0);
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
        `TMT_HOME=${fixture.globalDir}`,
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

  it('captures admitted launch, resume and provider hook observations', async () => {
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
        const db = new Database(path.join(fixture.globalDir, 'tmt.db'), { readonly: true });
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
        for (const index of [1, 3, 4, 5]) {
          expect(JSON.parse(results[index].stdout).workspace).toMatchObject({
            socket: fixture.socketPath,
            pid: fixture.serverPid,
          });
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
      expect(bound.code, bound.stderr + bound.stdout).toBe(0);
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

describe('rate-gated workspace command refresh', () => {
  it('captures a manual layout edit only after the configured interval, with zero retaining event-only mode', async () => {
    await withE2EFixture(async (fixture) => {
      expect((await fixture.runCli(['name', 'Refresh Seat', '-s'])).code).toBe(0);
      const shell = fixture.createShellPane('refresh-command-shell').pane;
      // A large server is where a capture costs a caller real time.
      for (let index = 0; index < 60; index++) fixture.createShellPane(`refresh-bulk-${index}`);
      const latest = snapshotPath(fixture);
      const initial = readSnapshot(fixture);
      const originalWindow = initial.panes.find((pane) => pane.id === fixture.pane)!.window;
      fixture.tmux(['rename-window', '-t', originalWindow, 'manual-edit']);
      // Controlled timestamp avoids waiting for a real clock interval.
      initial.capturedAtMs = Date.now() + 60_000;
      recoveryInput(fixture, initial);
      const fresh = fs.readFileSync(latest, 'utf8');
      let sequence = 0;
      // A changing command and a talk-to-pane command keep the snapshot fresh.
      const refreshing = ['check', fixture.pane, '1', '--json'];
      // Pure reads are extensions' hot paths and never wait for a capture.
      const reads = [
        ['identity', 'list', '--json'],
        ['room', 'list', '--json'],
        ['ls', '--json'],
      ];
      const run = async (args: readonly string[]) => {
        const status = path.join(fixture.root, `refresh-status-${sequence++}`);
        submitForeground(fixture, shell, args, status);
        expect(
          await waitForFileContent(status, { description: 'inspection command settled' })
        ).toBe('0');
      };
      await run(refreshing);
      expect(fs.readFileSync(latest, 'utf8')).toBe(fresh);
      initial.capturedAtMs = 1;
      recoveryInput(fixture, initial);
      const stale = fs.readFileSync(latest, 'utf8');
      for (const read of reads) {
        await run(read);
        expect(fs.readFileSync(latest, 'utf8'), read.join(' ')).toBe(stale);
      }
      await run(refreshing);
      const updated = readSnapshot(fixture);
      expect(updated.capturedAtMs).toBeGreaterThan(1);
      expect(updated.panes.length).toBeGreaterThanOrEqual(initial.panes.length + 60);
      expect(updated.windows.find((window) => window.id === originalWindow)!.name).toBe(
        'manual-edit'
      );
      expect(updated.panes.find((pane) => pane.id === fixture.pane)!.identity!.name).toBe(
        'Refresh Seat'
      );
      expect(
        (await fixture.runCli(['config', 'set', '--global', 'workspace.snapshotIntervalMs', '0']))
          .code
      ).toBe(0);
      updated.capturedAtMs = 1;
      recoveryInput(fixture, updated);
      const eventOnly = fs.readFileSync(latest, 'utf8');
      fixture.tmux(['rename-window', '-t', originalWindow, 'event-only-edit']);
      await run(refreshing);
      expect(fs.readFileSync(latest, 'utf8')).toBe(eventOnly);
      expect((await fixture.runCli(['unbind'])).code).toBe(0);
      expect(
        readSnapshot(fixture).windows.find((window) => window.id === originalWindow)!.name
      ).toBe('event-only-edit');
    });
  });

  it('does not trust inherited pane coordinates from a process outside its native ancestry', async () => {
    await withE2EFixture(async (fixture) => {
      expect((await fixture.runCli(['name', 'Ancestor Seat', '-s'])).code).toBe(0);
      const latest = snapshotPath(fixture);
      const prior = readSnapshot(fixture);
      prior.capturedAtMs = 1;
      recoveryInput(fixture, prior);
      const bytes = fs.readFileSync(latest, 'utf8');
      const result = await fixture.runJsonCli(['identity', 'list']);
      expect(result.code).toBe(0);
      expect(result.stderr).toBe('');
      expect(fs.readFileSync(latest, 'utf8')).toBe(bytes);
    });
  });
});

type LayoutRestore = {
  status: 'completed' | 'partial';
  layoutOnly: boolean;
  sessions: Array<{
    recorded: string;
    name: string;
    action: string;
    native: string | null;
    retainedBootstrap: string | null;
  }>;
  windows: Array<{ recorded: string; native: string }>;
  panes: Array<{ recorded: string; native: string }>;
  failures: string[];
};

type FullRestore = LayoutRestore & {
  revival: Array<{
    recorded: string;
    native: string | null;
    name: string;
    status: string;
    message: string;
  }>;
};

describe('full workspace restore', () => {
  it('refuses startup after its created shell is replaced and preserves that live process', async () => {
    await withRestoreFixture(async (fixture) => {
      expect((await fixture.runJsonCli(['name', 'Startup Guard Anchor', '-s'])).code).toBe(0);
      const saved = readSnapshot(fixture);
      saved.sessions.forEach((session) => {
        session.name = `guard-${session.name}`;
      });
      saved.panes.forEach((pane) => {
        pane.identity = null;
        pane.command = null;
      });
      saved.panes[0].command = {
        argv: ['tmt', 'workspace-refused', 'ui'],
        owner: saved.server.process,
      };
      const launched = path.join(fixture.root, 'unexpected-launch');
      writeExecutable(
        path.join(fixture.wrapperDir, 'tmt-workspace-refused'),
        `#!/bin/sh\nprintf started > ${quote(launched)}\n`
      );
      const wrapper = path.join(fixture.wrapperDir, 'tmux');
      const inner = path.join(fixture.wrapperDir, 'tmux-startup-inner');
      const replacement = path.join(fixture.root, 'replacement.json');
      fs.renameSync(wrapper, inner);
      writeExecutable(
        wrapper,
        `#!${process.execPath}
const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const args = process.argv.slice(2);
const decoded = args.map(arg => arg.replace(/\\\\([0-3][0-7]{2})/g, (_, octal) => String.fromCharCode(parseInt(octal, 8))));
if (decoded.some(arg => arg.startsWith('"respawn-pane"')) && !fs.existsSync(${JSON.stringify(replacement)})) {
  const pane = args[args.indexOf('-t') + 1];
  const swapped = spawnSync(${JSON.stringify(inner)}, ['-u', '-S', ${JSON.stringify(fixture.socketPath)}, 'respawn-pane', '-k', '-t', pane, '/bin/sleep', '30'], { encoding: 'utf8', timeout: 5000 });
  if (swapped.status !== 0) process.exit(97);
  const observed = spawnSync(${JSON.stringify(inner)}, ['-u', '-S', ${JSON.stringify(fixture.socketPath)}, 'display-message', '-p', '-t', pane, '#{pane_pid}'], { encoding: 'utf8', timeout: 5000 });
  fs.writeFileSync(${JSON.stringify(replacement)}, JSON.stringify({ pane, pid: Number(observed.stdout.trim()) }));
}
const result = spawnSync(${JSON.stringify(inner)}, args, { encoding: 'utf8', timeout: 5000 });
process.stdout.write(result.stdout || ''); process.stderr.write(result.stderr || ''); process.exit(result.status ?? 98);
`
      );
      recoveryInput(fixture, saved);
      let replacementProcess: { pane: string; pid: number } | undefined;
      try {
        const result = await fixture.runJsonCli<FullRestore>(
          ['workspace', 'restore', '--socket', fixture.socketPath],
          { outsideTmux: true }
        );
        expect(result.code, result.stderr + result.stdout).toBe(1);
        expect(result.json!.status).toBe('partial');
        expect(result.json!.revival[0].status).toBe('needs_you');
        replacementProcess = JSON.parse(fs.readFileSync(replacement, 'utf8'));
        expect(fixture.mockProcessIsRunning(replacementProcess!.pid)).toBe(true);
        expect(
          fixture
            .tmux(['display-message', '-p', '-t', replacementProcess!.pane, '#{pane_pid}'])
            .trim()
        ).toBe(String(replacementProcess!.pid));
        expect(fs.existsSync(launched)).toBe(false);
      } finally {
        if (!replacementProcess && fs.existsSync(replacement))
          replacementProcess = JSON.parse(fs.readFileSync(replacement, 'utf8'));
        if (replacementProcess) {
          fixture.tmux(['kill-pane', '-t', replacementProcess.pane]);
          const pid = replacementProcess.pid;
          await fixture.waitFor(
            () => !fixture.mockProcessIsRunning(pid),
            2000,
            'owned replacement process cleaned up'
          );
        }
      }
    });
  });

  it('starts exact resume and literal board argv, leaves missing/stale/changed identities as shells and skips existing sessions on rerun', async () => {
    await withRestoreFixture(async (fixture) => {
      const labels = [
        'Exact Agent',
        'Stale Agent',
        'Missing Agent',
        'Replaced Agent',
        'Renamed Agent',
        'No Session',
        'board',
        'shell',
      ];
      for (let index = 1; index < labels.length; index++)
        fixture.createShellPane(`revival-${index}`);
      expect((await fixture.runJsonCli(['name', 'Recovery Anchor', '-s'])).code).toBe(0);
      const saved = readSnapshot(fixture);
      const annotation = saved.panes.find((pane) => pane.identity)?.identity;
      expect(annotation).not.toBeNull();
      const records = new Map<string, string>();
      for (const name of labels.slice(0, 6)) {
        const created = await fixture.runJsonCli<{ identity: { id: string } }>([
          'identity',
          'create',
          name,
        ]);
        expect(created.code, created.stderr + created.stdout).toBe(0);
        records.set(name, created.json!.identity.id);
      }
      const session = '33333333-3333-4333-8333-333333333333';
      const db = new Database(path.join(fixture.globalDir, 'tmt.db'));
      try {
        for (const name of labels.slice(0, 5)) {
          db.prepare(`INSERT INTO identity_session_preferences
            (identity_id, preferred_harness, remembered_harness, runtime_mode, provider_session_id, stale_at_ms)
            VALUES (?, 'claude', 'claude', 'default', ?, ?)`).run(
            records.get(name),
            session,
            name === 'Stale Agent' ? 1 : null
          );
        }
      } finally {
        db.close();
      }
      // Frozen recovery input deliberately retains the original UUID and name.
      expect((await fixture.runCli(['rm', 'Missing Agent', '--force'])).code).toBe(0);
      expect((await fixture.runCli(['rm', 'Replaced Agent', '--force'])).code).toBe(0);
      expect((await fixture.runCli(['identity', 'create', 'Replaced Agent'])).code).toBe(0);
      expect((await fixture.runCli(['mv', 'Renamed Agent', 'New Agent Name'])).code).toBe(0);
      saved.sessions.forEach((session) => {
        session.name = `restored-${session.name}`;
      });
      saved.panes.forEach((pane, index) => {
        pane.identity =
          index < 6
            ? { ...annotation!, id: records.get(labels[index])!, name: labels[index] }
            : null;
        pane.command =
          index === 6
            ? {
                argv: [
                  'tmt',
                  'workspace-board',
                  'ui',
                  '--tabs=agents,requests',
                  "literal '$()#{pid};\n",
                ],
                owner: saved.server.process,
              }
            : null;
      });
      const reports = [
        path.join(fixture.root, 'resume-argv.json'),
        path.join(fixture.root, 'board-argv.json'),
      ];
      const gates = [path.join(fixture.root, 'resume.sock'), path.join(fixture.root, 'board.sock')];
      const done = reports.map((report) => `${report}.done`);
      for (const [index, executable] of ['claude', 'tmt-workspace-board'].entries()) {
        writeExecutable(
          path.join(fixture.wrapperDir, executable),
          `#!${process.execPath}
const fs = require('node:fs');
const net = require('node:net');
const server = net.createServer(peer => {
  let text = '';
  peer.on('data', bytes => { text += bytes; });
  peer.once('end', () => {
    if (text !== 'release') process.exit(97);
    peer.end('released');
    server.close(() => { fs.writeFileSync(${JSON.stringify(done[index])}, 'done'); process.exit(0); });
  });
});
server.listen(${JSON.stringify(gates[index])}, () => {
  fs.writeFileSync(${JSON.stringify(reports[index])}, JSON.stringify({ pid: process.pid, argv: process.argv.slice(2) }));
});
`
        );
      }
      recoveryInput(fixture, saved);
      const livePids: number[] = [];
      let resumeStarted = false;
      let revivedPanes: string[] = [];
      let shellCommand = '';
      try {
        const result = await fixture.runJsonCli<FullRestore>(
          ['workspace', 'restore', '--socket', fixture.socketPath],
          { outsideTmux: true }
        );
        expect(result.code, result.stderr + result.stdout).toBe(0);
        expect(result.json!.layoutOnly).toBe(false);
        resumeStarted = result.json!.revival[0].status === 'started';
        revivedPanes = [0, 6].map(
          (index) =>
            result.json!.panes.find((pane) => pane.recorded === saved.panes[index].id)!.native
        );
        // tmux can report the shell executable name (bash/dash) for /bin/sh.
        // Observe the untouched default-shell pane instead of inferring from its path.
        const shellPane = result.json!.panes.find(
          (pane) => pane.recorded === saved.panes[7].id
        )!.native;
        shellCommand = fixture
          .tmux(['display-message', '-p', '-t', shellPane, '#{pane_current_command}'])
          .trim();
        expect(shellCommand).not.toBe('');
        expect(result.json!.revival.map((pane) => [pane.name, pane.status])).toEqual([
          ['Exact Agent', 'started'],
          ['Stale Agent', 'needs_you'],
          ['Missing Agent', 'needs_you'],
          ['Replaced Agent', 'needs_you'],
          ['Renamed Agent', 'needs_you'],
          ['No Session', 'needs_you'],
          ['workspace-board', 'relaunched'],
          [saved.panes[7].id, 'skipped'],
        ]);
        for (const report of reports)
          await fixture.waitFor(
            () => fs.existsSync(report),
            5000,
            'restored command causal output'
          );
        const agent = JSON.parse(fs.readFileSync(reports[0], 'utf8')) as {
          pid: number;
          argv: string[];
        };
        const board = JSON.parse(fs.readFileSync(reports[1], 'utf8')) as {
          pid: number;
          argv: string[];
        };
        livePids.push(agent.pid, board.pid);
        expect(agent.argv[agent.argv.indexOf('--resume') + 1]).toBe(session);
        expect(board.argv).toEqual(saved.panes[6].command!.argv.slice(2));
        // Direct startup is causal; it also traverses the generic marker owner.
        const boardPane = result.json!.panes.find(
          (pane) => pane.recorded === saved.panes[6].id
        )!.native;
        expect(
          fixture
            .tmux(['show-option', '-p', '-qv', '-t', boardPane, '@tmt.workspace-command'])
            .trim()
        ).not.toBe('');
        for (const index of [1, 2, 3, 4, 5, 7]) {
          const pane = result.json!.panes.find(
            (pane) => pane.recorded === saved.panes[index].id
          )!.native;
          expect(
            fixture.tmux(['display-message', '-p', '-t', pane, '#{pane_current_command}']).trim()
          ).toBe(shellCommand);
        }
        // Launch events may refresh the latest snapshot. Restore the exact
        // frozen input to prove rerun's existing-session skip independently.
        recoveryInput(fixture, saved);
        const repeat = await fixture.runJsonCli<FullRestore>(
          ['workspace', 'restore', '--socket', fixture.socketPath],
          { outsideTmux: true }
        );
        expect(repeat.code, repeat.stderr + repeat.stdout).toBe(0);
        expect(repeat.json!.panes).toEqual([]);
        expect(
          repeat.json!.revival.every((pane) => pane.message === 'skipped (existing session)')
        ).toBe(true);
        expect(JSON.parse(fs.readFileSync(reports[0], 'utf8')).pid).toBe(agent.pid);
        expect(JSON.parse(fs.readFileSync(reports[1], 'utf8')).pid).toBe(board.pid);
      } finally {
        // Shut down only the fixture-owned IPC children, then require both the
        // child and its foreground tmt owner to settle before fixture deletion.
        for (const report of reports) {
          if (fs.existsSync(report)) livePids.push(JSON.parse(fs.readFileSync(report, 'utf8')).pid);
        }
        for (const [index, gate] of gates.entries()) {
          if (!fs.existsSync(gate)) continue;
          await new Promise<void>((resolve, reject) => {
            const peer = net.createConnection(gate);
            peer.setTimeout(5000, () => peer.destroy(new Error('release timed out')));
            let response = '';
            peer.once('connect', () => peer.end('release'));
            peer.on('data', (bytes) => {
              response += bytes;
            });
            peer.once('error', reject);
            peer.once('close', () =>
              response === 'released' ? resolve() : reject(new Error('release not acknowledged'))
            );
          });
          await fixture.waitFor(() => fs.existsSync(done[index]), 5000, 'restored command exited');
        }
        await fixture.waitFor(
          () => livePids.every((pid) => !fixture.mockProcessIsRunning(pid)),
          5000,
          'restored provider/board process absence'
        );
        if (resumeStarted) {
          const oracle = new Database(path.join(fixture.globalDir, 'tmt.db'), {
            readonly: true,
          });
          try {
            await fixture.waitFor(
              () => {
                const row = oracle
                  .prepare('SELECT runtime_state FROM bindings WHERE identity_id = ?')
                  .get(records.get('Exact Agent')) as { runtime_state: string } | undefined;
                return row?.runtime_state === 'ended';
              },
              5000,
              'foreground resume completion stored'
            );
          } finally {
            oracle.close();
          }
        }
      }
      // Provider and board exit must preserve their exact restored panes.
      const shell = fixture.tmux(['show-options', '-gv', 'default-shell']).trim();
      for (const pane of revivedPanes) {
        const expected = `0|${shellCommand}`;
        let observed = '';
        try {
          await fixture.waitFor(
            () => {
              observed = fixture
                .tmux([
                  'display-message',
                  '-p',
                  '-t',
                  pane,
                  '#{pane_dead}|#{pane_current_command}|#{pane_pid}|#{pane_start_command}',
                ])
                .trim();
              return observed.split('|').slice(0, 2).join('|') === expected;
            },
            5000,
            'restored pane returns to the user shell'
          );
        } catch (cause) {
          throw new Error(
            `Shell return: ${JSON.stringify({
              pane,
              defaultShell: shell,
              controlCommand: shellCommand,
              observed,
            })}`,
            { cause }
          );
        }
        expect(observed.split('|').slice(0, 2).join('|')).toBe(expected);
      }
    });
  });
});

// Synthetic codec/restore and command-cadence scenarios explicitly exercise
// legacy stores. Production automatic generations remain immutable and indexed;
// overwriting only the legacy projection must never substitute for that index.
function recoveryInput(fixture: E2EFixture, snapshot: Snapshot | string) {
  fs.writeFileSync(
    snapshotPath(fixture),
    typeof snapshot === 'string' ? snapshot : JSON.stringify(snapshot),
    { mode: 0o600 }
  );
  fs.rmSync(path.join(path.dirname(snapshotPath(fixture)), 'generations.json'), { force: true });
}

async function withRestoreFixture(callback: (fixture: E2EFixture) => Promise<void>) {
  let observed: E2EFixture | undefined;
  let ownedPids: number[] = [];
  try {
    await withE2EFixture(async (fixture) => {
      observed = fixture;
      try {
        await callback(fixture);
      } finally {
        if (fixture.serverIsRunning()) {
          ownedPids = fixture
            .tmux(['-u', 'list-panes', '-a', '-F', '#{pane_pid}'])
            .trim()
            .split('\n')
            .map(Number);
          ownedPids.push(Number(fixture.tmux(['-u', 'display-message', '-p', '#{pid}']).trim()));
        }
      }
    });
  } finally {
    if (observed) {
      const fixture = observed;
      await fixture.waitFor(
        () => ownedPids.every((pid) => !fixture.mockProcessIsRunning(pid)),
        2000,
        'restored server and pane processes cleaned up'
      );
      expect(fs.existsSync(fixture.socketPath)).toBe(false);
      expect(fs.existsSync(fixture.root)).toBe(false);
    }
  }
}

function twoMissingLinkedSessions(fixture: E2EFixture): Snapshot {
  const saved = readSnapshot(fixture);
  const original = saved.sessions.find((session) => session.name === 'e2e')!;
  return {
    ...saved,
    sessions: [
      original,
      ...['restore-first', 'restore-shared'].map((name, index) => ({
        id: `$${9000 + index}`,
        name,
        windows: original.windows,
      })),
    ],
  };
}

function injectRestoreObservation(fixture: E2EFixture, fault: 'link' | 'pid') {
  const wrapper = path.join(fixture.wrapperDir, 'tmux');
  const inner = path.join(fixture.wrapperDir, 'tmux-restore-inner');
  const trace = path.join(fixture.root, 'restore-fault.log');
  fs.renameSync(wrapper, inner);
  writeExecutable(
    wrapper,
    `#!${process.execPath}
const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const args = process.argv.slice(2);
const decoded = args.map(arg => arg.replace(/\\\\([0-3][0-7]{2})/g, (_, octal) => String.fromCharCode(parseInt(octal, 8))));
const branch = decoded.find(arg => arg.startsWith('"link-window"') || (arg.startsWith('"display-message"') && arg.includes('pane_current_command')));
if (branch) fs.appendFileSync(${JSON.stringify(trace)}, branch + '\\n');
if (${JSON.stringify(fault)} === 'link' && branch?.startsWith('"link-window"')) process.exit(97);
const result = spawnSync(${JSON.stringify(inner)}, args, { encoding: 'utf8', timeout: 5000 });
let output = result.stdout || '';
if (${JSON.stringify(fault)} === 'pid' && branch?.includes('pane_current_command')) {
  const fields = output.split('__TMT_FIELD_4f1c__'); fields[1] = '2147483647'; output = fields.join('__TMT_FIELD_4f1c__');
}
process.stdout.write(output);
process.stderr.write(result.stderr || '');
process.exit(result.status ?? 98);
`,
    0o755
  );
  return trace;
}

// Fixture observations preserve literal bytes; restore CLI clients exercise the C locale.
describe('layout-only workspace restore', () => {
  // Exact layouts and independent pane rectangles captured from private tmux
  // 3.8 producers during #4189/#4402, never generated by the codec under test.
  for (const input of [
    {
      label: 'four-pane producer',
      layout:
        '{"V":2,"L":{"t":"v","w":120,"h":40,"x":0,"y":0,"c":[{"t":"h","w":120,"h":19,"x":0,"y":0,"c":[{"t":"p","w":59,"h":19,"x":0,"y":0,"a":true,"i":0,"I":"%0"},{"t":"p","w":60,"h":19,"x":60,"y":0,"i":1,"I":"%3"}]},{"t":"h","w":120,"h":20,"x":0,"y":20,"c":[{"t":"p","w":59,"h":20,"x":0,"y":20,"i":2,"I":"%2"},{"t":"p","w":60,"h":20,"x":60,"y":20,"i":3,"I":"%1"}]}]}}',
      width: 120,
      height: 40,
      activePane: '%0',
      panes: [
        { id: '%0', index: 0, left: 0, top: 0, width: 59, height: 19 },
        { id: '%1', index: 3, left: 60, top: 20, width: 60, height: 20 },
        { id: '%2', index: 2, left: 0, top: 20, width: 59, height: 20 },
        { id: '%3', index: 1, left: 60, top: 0, width: 60, height: 19 },
      ],
    },
    {
      label: 'reversed-index producer',
      layout:
        '{"V":2,"L":{"t":"h","w":80,"h":24,"x":0,"y":0,"c":[{"t":"p","w":29,"h":24,"x":0,"y":0,"i":1,"I":"%1"},{"t":"p","w":50,"h":24,"x":30,"y":0,"a":true,"i":0,"I":"%0"}]}}',
      width: 80,
      height: 24,
      activePane: '%0',
      panes: [
        { id: '%0', index: 0, left: 30, top: 0, width: 50, height: 24 },
        { id: '%1', index: 1, left: 0, top: 0, width: 29, height: 24 },
      ],
    },
  ]) {
    it(`restores the ${input.label} layout on the live encoding with each pane's geometry and cwd`, async () => {
      await withRestoreFixture(async (fixture) => {
        guardWorkspaceServer(fixture);
        expect((await fixture.runJsonCli(['name', 'Retained V2 Control', '-s'])).code).toBe(0);
        const saved = readSnapshot(fixture);
        const original = fixture.tmux([
          'list-panes',
          '-a',
          '-F',
          '#{pane_id}|#{pane_pid}|#{window_layout}',
        ]);
        saved.sessions[0].name = 'retained-v2';
        const window = saved.windows[0];
        Object.assign(window, {
          layout: input.layout,
          visibleLayout: input.layout,
          width: input.width,
          height: input.height,
          activePane: input.activePane,
        });
        saved.panes = input.panes.map((pane) => {
          const cwd = path.join(fixture.workspace, `pane-${pane.index}`);
          fs.mkdirSync(cwd);
          return { ...pane, window: window.id, cwd, identity: null, command: null };
        });
        recoveryInput(fixture, saved);
        const bytes = fs.readFileSync(snapshotPath(fixture));
        const result = await fixture.runJsonCli<LayoutRestore>(
          ['workspace', 'restore', '--layout-only', '--socket', fixture.socketPath],
          { outsideTmux: true }
        );
        expect(result.code, result.stderr + result.stdout).toBe(0);
        expect(
          result.json!.sessions.find((session) => session.name === 'retained-v2')!.action
        ).toBe('created');
        expect(result.json!.panes).toHaveLength(input.panes.length);
        for (const pane of saved.panes) {
          const mapping = result.json!.panes.find((mapped) => mapped.recorded === pane.id)!;
          expect(mapping, pane.id).toBeDefined();
          expect(
            fixture
              .tmux([
                'display-message',
                '-p',
                '-t',
                mapping.native,
                '#{session_name}|#{pane_left}|#{pane_top}|#{pane_width}|#{pane_height}|#{pane_current_path}',
              ])
              .trim()
          ).toBe(`retained-v2|${pane.left}|${pane.top}|${pane.width}|${pane.height}|${pane.cwd}`);
        }
        expect(
          fixture.tmux(['list-panes', '-t', 'e2e', '-F', '#{pane_id}|#{pane_pid}|#{window_layout}'])
        ).toBe(original);
        expect(fs.readFileSync(snapshotPath(fixture))).toEqual(bytes);
      });
    });
  }

  for (const serverExit of ['clean', 'crash'] as const) {
    it(`starts a fresh server after ${serverExit} exit outside tmux and restores geometry, cwd, links, selection and zoom without revival`, async () => {
      await withRestoreFixture(async (fixture) => {
        const special = path.join(fixture.root, `cwd '$#(unused) ending;`);
        fs.mkdirSync(special);
        fixture.tmux([
          '-u',
          'new-window',
          '-d',
          '-t',
          'e2e:4',
          '-n',
          "Literal ##(unused) '$name\\;",
          '-c',
          special.replaceAll('#', '##').replace(/;$/, '\\;'),
          '/bin/sh',
          '-i',
        ]);
        const split = fixture
          .tmux([
            '-u',
            'split-window',
            '-d',
            '-h',
            '-l',
            '100',
            '-P',
            '-F',
            '#{pane_id}',
            '-t',
            'e2e:4',
            '-c',
            fixture.workspace,
            '/bin/sh',
            '-i',
          ])
          .trim();
        fixture.tmux([
          '-u',
          'split-window',
          '-d',
          '-v',
          '-l',
          '20',
          '-t',
          split,
          '-c',
          fixture.root,
          '/bin/sh',
          '-i',
        ]);
        fixture.tmux(['-u', 'select-pane', '-t', split]);
        fixture.tmux(['-u', 'resize-pane', '-Z', '-t', split]);
        fixture.tmux(['-u', 'new-session', '-d', '-s', 'linked']);
        fixture.tmux(['-u', 'link-window', '-s', 'e2e:4', '-t', 'linked:9']);
        expect((await fixture.runJsonCli(['name', 'Restore Remembered Identity', '-s'])).code).toBe(
          0
        );
        const saved = readSnapshot(fixture);
        // Restore also accepts literal cwd bytes from valid synthetic recovery input;
        // capture's line-delimited host read is a separate contract.
        const newlineCwd = path.join(fixture.root, 'cwd with\nnewline;');
        fs.mkdirSync(newlineCwd);
        for (const pane of saved.panes) {
          if (pane.cwd === special) pane.cwd = newlineCwd;
        }
        recoveryInput(fixture, saved);
        const bytes = fs.readFileSync(snapshotPath(fixture));
        const database = path.join(fixture.globalDir, 'tmt.db');
        const databaseBytes = fs.readFileSync(database);
        const oldId = fixture.tmux(['-u', 'show-option', '-s', '-v', '@tmt.server-id']).trim();
        const oldPid = fixture.serverPid;
        const oldPanePids = fixture
          .tmux(['-u', 'list-panes', '-a', '-F', '#{pane_pid}'])
          .trim()
          .split('\n')
          .map(Number);
        if (serverExit === 'crash') process.kill(oldPid, 'SIGKILL');
        else fixture.tmux(['-u', 'kill-server']);
        await fixture.waitFor(
          () =>
            !fixture.mockProcessIsRunning(oldPid) &&
            oldPanePids.every((pid) => !fixture.mockProcessIsRunning(pid)),
          2000,
          'old private server exit'
        );
        // tmux may leave its dead socket inode even after clean process exit.
        // This fixture owns the exact socket; make the absent-socket case explicit.
        if (serverExit === 'clean' && fs.existsSync(fixture.socketPath)) {
          expect(fs.lstatSync(fixture.socketPath).isSocket()).toBe(true);
          fs.unlinkSync(fixture.socketPath);
        }
        expect(fs.existsSync(fixture.socketPath)).toBe(serverExit === 'crash');
        const result = await fixture.runJsonCli<LayoutRestore>(
          ['workspace', 'restore', '--layout-only', '--socket', fixture.socketPath],
          { outsideTmux: true, locale: 'C' }
        );
        expect(result.code, result.stderr + result.stdout).toBe(0);
        expect(result.json!.status).toBe('completed');
        expect(result.json!.sessions.every((session) => session.action === 'created')).toBe(true);
        fixture.serverPid = Number(fixture.tmux(['-u', 'display-message', '-p', '#{pid}']).trim());
        expect(fixture.serverPid).not.toBe(oldPid);
        expect(fixture.tmux(['-u', 'show-option', '-s', '-v', '@tmt.server-id']).trim()).not.toBe(
          oldId
        );
        expect(fs.readFileSync(snapshotPath(fixture))).toEqual(bytes);
        expect(fs.readFileSync(database)).toEqual(databaseBytes);
        for (const session of saved.sessions) {
          const restored = result.json!.sessions.find((entry) => entry.recorded === session.id)!;
          const links = fixture
            .tmux([
              '-u',
              'list-windows',
              '-t',
              restored.native!,
              '-F',
              '#{window_index}:#{window_id}:#{window_active}',
            ])
            .trim()
            .split('\n');
          expect(links).toEqual(
            session.windows.map(
              (link) =>
                `${link.index}:${result.json!.windows.find((window) => window.recorded === link.window)!.native}:${Number(link.active)}`
            )
          );
        }
        for (const pane of saved.panes) {
          const restored = result.json!.panes.find((entry) => entry.recorded === pane.id)!;
          expect(
            fixture
              .tmux(['-u', 'display-message', '-p', '-t', restored.native, '#{pane_current_path}'])
              .slice(0, -1)
          ).toBe(pane.cwd);
          expect(
            fixture
              .tmux([
                '-u',
                'display-message',
                '-p',
                '-t',
                restored.native,
                '#{pane_left}:#{pane_top}:#{pane_width}:#{pane_height}',
              ])
              .trim()
          ).toBe(`${pane.left}:${pane.top}:${pane.width}:${pane.height}`);
          expect(
            fixture
              .tmux(['-u', 'display-message', '-p', '-t', restored.native, '#{@tmt.agent}'])
              .trim()
          ).toBe('');
          expect(
            fixture
              .tmux([
                '-u',
                'display-message',
                '-p',
                '-t',
                restored.native,
                '#{pane_current_command}',
              ])
              .trim()
          ).toBe(
            path.basename(fixture.tmux(['-u', 'show-options', '-g', '-v', 'default-shell']).trim())
          );
        }
        for (const window of saved.windows) {
          const restored = result.json!.windows.find((entry) => entry.recorded === window.id)!;
          expect(
            fixture
              .tmux(['-u', 'display-message', '-p', '-t', restored.native, '#{window_name}'])
              .slice(0, -1)
          ).toBe(window.name);
          const restoreOldIds = (layout: string) =>
            layout.slice(5).replace(/(\d+x\d+,\d+,\d+),(\d+)/g, (_, cell: string, id: string) => {
              const pane = result.json!.panes.find((pane) => pane.native === `%${id}`)!;
              return `${cell},${pane.recorded.slice(1)}`;
            });
          expect(
            restoreOldIds(
              fixture
                .tmux(['-u', 'display-message', '-p', '-t', restored.native, '#{window_layout}'])
                .trim()
            )
          ).toBe(window.layout.slice(5));
          expect(
            restoreOldIds(
              fixture
                .tmux([
                  '-u',
                  'display-message',
                  '-p',
                  '-t',
                  restored.native,
                  '#{window_visible_layout}',
                ])
                .trim()
            )
          ).toBe(window.visibleLayout.slice(5));
          expect(
            fixture
              .tmux([
                '-u',
                'display-message',
                '-p',
                '-t',
                restored.native,
                '#{window_width}x#{window_height}:#{window_zoomed_flag}',
              ])
              .trim()
          ).toBe(
            `${window.width}x${window.height}:${Number(window.layout !== window.visibleLayout)}`
          );
        }
        const topology = fixture.tmux([
          '-u',
          'list-panes',
          '-a',
          '-F',
          '#{session_id}:#{window_id}:#{window_layout}:#{pane_id}:#{pane_pid}:#{pane_current_path}',
        ]);
        const again = await fixture.runJsonCli<LayoutRestore>(
          ['workspace', 'restore', '--layout-only', '--socket', fixture.socketPath],
          { outsideTmux: true, locale: 'C' }
        );
        expect(again.code, again.stdout).toBe(0);
        expect(again.json!.sessions.every((session) => session.action === 'skip_existing')).toBe(
          true
        );
        expect(again.json!.panes).toEqual([]);
        expect(
          fixture.tmux([
            '-u',
            'list-panes',
            '-a',
            '-F',
            '#{session_id}:#{window_id}:#{window_layout}:#{pane_id}:#{pane_pid}:#{pane_current_path}',
          ])
        ).toBe(topology);
      });
    });
  }

  it('skips an existing session whole and restores two missing sessions sharing one single-pane window', async () => {
    await withRestoreFixture(async (fixture) => {
      expect((await fixture.runJsonCli(['name', 'Preserved Restore Pane', '-s'])).code).toBe(0);
      const saved = twoMissingLinkedSessions(fixture);
      recoveryInput(fixture, saved);
      // A configured login shell must survive restore; explicit /bin/sh would fail.
      fixture.tmux(['-u', 'set-option', '-g', 'default-shell', '/usr/bin/zsh']);
      fixture.tmux(['-u', 'set-option', '-g', 'default-command', '']);
      fixture.tmux([
        '-u',
        'set-option',
        '-p',
        '-t',
        fixture.pane,
        '@restore-user-data',
        'unchanged',
      ]);
      const topology = fixture.tmux([
        '-u',
        'list-panes',
        '-t',
        'e2e',
        '-F',
        '#{window_layout}:#{pane_id}:#{pane_pid}:#{pane_current_path}:#{@tmt.agent}:#{@restore-user-data}',
      ]);
      const result = await fixture.runJsonCli<LayoutRestore>(
        ['workspace', 'restore', '--layout-only', '--socket', fixture.socketPath],
        { outsideTmux: true, locale: 'C' }
      );
      expect(result.code, result.stdout).toBe(0);
      expect(result.json!.sessions.map((session) => session.action)).toEqual([
        'skip_existing',
        'created',
        'created',
      ]);
      expect(result.json!.windows).toHaveLength(1);
      expect(result.json!.panes).toHaveLength(1);
      expect(result.json!.panes[0].native).not.toBe(fixture.pane);
      const defaultShell = fixture.tmux(['-u', 'show-options', '-g', '-v', 'default-shell']).trim();
      expect(defaultShell).toBe('/usr/bin/zsh');
      expect(
        fixture
          .tmux([
            '-u',
            'display-message',
            '-p',
            '-t',
            result.json!.panes[0].native,
            '#{pane_current_command}',
          ])
          .trim()
      ).toBe(path.basename(defaultShell));
      expect(fixture.tmux(['-u', 'show-options', '-g', '-v', 'default-command']).trim()).toBe('');

      for (const name of ['restore-first', 'restore-shared']) {
        expect(fixture.tmux(['-u', 'list-windows', '-t', name, '-F', '#{window_id}']).trim()).toBe(
          result.json!.windows[0].native
        );
      }
      expect(result.json!.sessions.every((session) => session.retainedBootstrap === null)).toBe(
        true
      );
      expect(
        fixture.tmux([
          '-u',
          'list-panes',
          '-t',
          'e2e',
          '-F',
          '#{window_layout}:#{pane_id}:#{pane_pid}:#{pane_current_path}:#{@tmt.agent}:#{@restore-user-data}',
        ])
      ).toBe(topology);
    });
  });

  for (const fault of ['link', 'pid'] as const) {
    it(`retains the bootstrap after ${fault === 'link' ? 'failed linking' : 'fresh pid mismatch'} and leaves existing panes untouched`, async () => {
      await withRestoreFixture(async (fixture) => {
        expect((await fixture.runJsonCli(['name', 'Bootstrap Failure Preserve', '-s'])).code).toBe(
          0
        );
        recoveryInput(fixture, twoMissingLinkedSessions(fixture));
        const original = fixture.tmux([
          '-u',
          'list-panes',
          '-t',
          'e2e',
          '-F',
          '#{pane_id}:#{pane_pid}:#{window_layout}:#{@tmt.agent}',
        ]);
        const trace = injectRestoreObservation(fixture, fault);
        const result = await fixture.runJsonCli<LayoutRestore>(
          ['workspace', 'restore', '--layout-only', '--socket', fixture.socketPath],
          { outsideTmux: true, locale: 'C' }
        );
        expect(result.code).toBe(1);
        expect(result.json!.status).toBe('partial');
        const partial = result.json!.sessions.find((session) => session.name === 'restore-shared')!;
        expect(partial.action).toBe('partial');
        expect(partial.retainedBootstrap).toMatch(/^%\d+$/);
        const panes = fixture
          .tmux(['-u', 'list-panes', '-s', '-t', 'restore-shared', '-F', '#{pane_id}'])
          .trim()
          .split('\n');
        expect(panes).toContain(partial.retainedBootstrap);
        expect(panes).toHaveLength(fault === 'link' ? 1 : 2);
        expect(fs.readFileSync(trace, 'utf8')).toContain('"link-window"');
        if (fault === 'pid')
          expect(fs.readFileSync(trace, 'utf8')).toContain('pane_current_command');
        expect(
          fixture.tmux([
            '-u',
            'list-panes',
            '-t',
            'e2e',
            '-F',
            '#{pane_id}:#{pane_pid}:#{window_layout}:#{@tmt.agent}',
          ])
        ).toBe(original);
        const again = await fixture.runJsonCli<LayoutRestore>(
          ['workspace', 'restore', '--layout-only', '--socket', fixture.socketPath],
          { outsideTmux: true, locale: 'C' }
        );
        expect(again.code, again.stdout).toBe(0);
        expect(again.json!.sessions.every((session) => session.action === 'skip_existing')).toBe(
          true
        );
        expect(
          fixture
            .tmux(['-u', 'list-panes', '-s', '-t', 'restore-shared', '-F', '#{pane_id}'])
            .trim()
            .split('\n')
        ).toEqual(panes);
      });
    });
  }
});

describe('named workspace selection and digest admission', () => {
  it('preserves named bytes across capture, refuses changed latest before effects and restores frozen input after replacement', async () => {
    await withRestoreFixture(async (fixture) => {
      expect((await fixture.runJsonCli(['name', 'Guard Snapshot Identity', '-s'])).code).toBe(0);
      const original = readSnapshot(fixture);
      original.sessions[0].name = 'guard-frozen';
      const bytes = `${JSON.stringify(original)}\n`;
      recoveryInput(fixture, bytes);
      const selected = ['--socket', fixture.socketPath];
      const saved = await fixture.runJsonCli<{ digest: string }>([
        'workspace',
        'save',
        'work',
        ...selected,
      ]);
      expect(saved.code, saved.stderr + saved.stdout).toBe(0);
      const digest = createHash('sha256').update(bytes).digest('hex');
      expect(saved.json!.digest).toBe(digest);
      const named = path.join(path.dirname(snapshotPath(fixture)), 'saved-work.json');
      expect(fs.readFileSync(named, 'utf8')).toBe(bytes);
      // A committed binding change captures latest independently of refresh cadence.
      expect(
        (await fixture.runJsonCli(['mv', 'Guard Snapshot Identity', 'Guard Renamed Identity'])).code
      ).toBe(0);
      expect(fs.readFileSync(snapshotPath(fixture), 'utf8')).not.toBe(bytes);
      expect(fs.readFileSync(named, 'utf8')).toBe(bytes);
      const changed = { ...original, capturedAtMs: original.capturedAtMs + 1 };
      recoveryInput(fixture, changed);
      const database = path.join(fixture.globalDir, 'tmt.db');
      const databaseBytes = fs.readFileSync(database);
      const trace = installTmuxTrace(fixture);
      const refused = await fixture.runJsonCli<{ error: { code: string } }>(
        ['workspace', 'restore', '--expect', digest, ...selected],
        { outsideTmux: true }
      );
      expect(refused.code).toBe(5);
      expect(refused.json!.error.code).toBe('WORKSPACE_SNAPSHOT_CHANGED');
      expect(trace.commands()).toEqual([]);
      expect(fs.readFileSync(database)).toEqual(databaseBytes);
      expect(fs.readFileSync(snapshotPath(fixture), 'utf8')).toBe(JSON.stringify(changed));
      const positive = await fixture.runJsonCli<LayoutRestore>(
        ['workspace', 'restore', 'work', '--layout-only', '--expect', digest, ...selected],
        { outsideTmux: true }
      );
      expect(positive.code, positive.stderr + positive.stdout).toBe(0);
      expect(
        positive.json!.sessions.some(
          (session) => session.name === 'guard-frozen' && session.action === 'created'
        )
      ).toBe(true);
      expect(fixture.tmux(['has-session', '-t', 'e2e'])).toBe('');
      expect(fs.readFileSync(database)).toEqual(databaseBytes);
      // The first live host read is a deterministic barrier after acquisition.
      recoveryInput(fixture, bytes);
      const replacement = structuredClone(original);
      replacement.sessions[0].name = 'guard-replacement';
      const wrapper = path.join(fixture.wrapperDir, 'tmux');
      const delegate = path.join(fixture.wrapperDir, 'tmux-guard-inner');
      fs.renameSync(wrapper, delegate);
      const replaced = path.join(fixture.root, 'guard-replaced');
      writeExecutable(
        wrapper,
        `#!${process.execPath}
const fs = require('node:fs');
const cp = require('node:child_process');
if (!fs.existsSync(${JSON.stringify(replaced)})) {
  fs.writeFileSync(${JSON.stringify(snapshotPath(fixture))}, ${JSON.stringify(JSON.stringify(replacement))});
  fs.writeFileSync(${JSON.stringify(replaced)}, 'replaced');
}
const result = cp.spawnSync(${JSON.stringify(delegate)}, process.argv.slice(2), { env: process.env, stdio: 'inherit', timeout: 5000 });
process.exit(result.status ?? 98);
`,
        0o755
      );
      const frozen = await fixture.runJsonCli<LayoutRestore>(
        ['workspace', 'restore', '--layout-only', '--expect', digest, ...selected],
        { outsideTmux: true }
      );
      expect(frozen.code, frozen.stderr + frozen.stdout).toBe(0);
      expect(fs.readFileSync(replaced, 'utf8')).toBe('replaced');
      expect(
        frozen.json!.sessions.some(
          (session) => session.name === 'guard-frozen' && session.action === 'skip_existing'
        )
      ).toBe(true);
      expect(frozen.json!.sessions.some((session) => session.name === 'guard-replacement')).toBe(
        false
      );
      expect(fixture.tmux(['list-sessions', '-F', '#{session_name}'])).not.toContain(
        'guard-replacement'
      );
      expect(fs.readFileSync(named, 'utf8')).toBe(bytes);
      expect(fs.readFileSync(database)).toEqual(databaseBytes);
    });
  });
});
