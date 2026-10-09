import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { describe, expect, it } from 'vite-plus/test';
import { runCli, withSandbox } from '../support/cli-process.js';
import { installTmuxTripwire } from './tmux-tripwire.js';

function snapshot(socket: string) {
  return {
    version: 1,
    capturedAtMs: 1,
    server: { socket, process: { pid: 10, start: 'native-start' }, id: null },
    sessions: [
      { id: '$1', name: 'work', windows: [{ index: 0, window: '@1', active: true }] },
      { id: '$2', name: 'linked', windows: [{ index: 2, window: '@1', active: true }] },
    ],
    windows: [
      {
        id: '@1',
        name: 'tools',
        layout: 'abcd,80x24,0,0,1',
        visibleLayout: 'abcd,80x24,0,0,1',
        width: 80,
        height: 24,
        activePane: '%1',
      },
    ],
    panes: [
      {
        id: '%1',
        window: '@1',
        index: 0,
        left: 0,
        top: 0,
        width: 80,
        height: 24,
        cwd: '/work',
        identity: null,
        command: null,
      },
    ],
  };
}

describe('read-only workspace restore preview', () => {
  it('retains topology and literal extension argv without creating storage, publishing or invoking tmux/providers', async () => {
    await withSandbox(async (sandbox) => {
      const tripwire = installTmuxTripwire(sandbox);
      const socket = path.join(sandbox.root, 'gone-socket');
      const directory = path.join(
        sandbox.globalDir,
        'workspace',
        createHash('sha256').update(socket).digest('hex')
      );
      fs.mkdirSync(directory, { recursive: true, mode: 0o700 });
      const latest = path.join(directory, 'latest.json');
      const saved = snapshot(socket);
      const command = {
        argv: ['tmt', 'example', 'ui', '--tabs=a,b', '$(never executed)'],
        owner: { pid: 20, start: 'owner-start' },
      };
      fs.writeFileSync(latest, JSON.stringify(saved), { mode: 0o600 });
      const before = fs.readFileSync(latest);
      // Provider coordinates cannot cause caller-session learning in this inspection.
      sandbox.env.CODEX_THREAD_ID = 'unknown-thread';
      sandbox.env.CLAUDE_CODE_SESSION_ID = 'unknown-session';
      sandbox.env.CLAUDE_PID = '20';
      const result = await runCli(sandbox, ['workspace', 'show', '--socket', socket, '--json']);
      expect(result.status).toBe(0);
      expect(result.stderr).toBe('');
      const plan = JSON.parse(result.stdout);
      expect(plan.snapshot).toEqual(saved);
      expect(plan.sessions).toEqual([
        { session: '$1', action: 'create' },
        { session: '$2', action: 'create' },
      ]);
      expect(plan.panes).toEqual([{ pane: '%1', action: 'shell', identity: null, resume: null }]);
      expect(fs.readFileSync(latest)).toEqual(before);
      expect(fs.readdirSync(directory)).toEqual(['latest.json']);
      expect(fs.existsSync(sandbox.database)).toBe(false);
      expect(fs.existsSync(tripwire)).toBe(false);
      fs.writeFileSync(
        latest,
        JSON.stringify({ ...saved, panes: [{ ...saved.panes[0], command }] })
      );
      const human = await runCli(sandbox, ['workspace', 'show', '--socket', socket]);
      expect(human.status).toBe(0);
      expect(human.stdout).toMatch(/\(captured \d+d ago\)/);
      expect(human.stdout).not.toContain('(captured 1 ms)');
      expect(human.stdout).toContain('relaunch recorded command');
      expect(human.stdout).toContain('--tabs=a,b');
      const json = await runCli(sandbox, ['workspace', 'show', '--socket', socket, '--json']);
      expect(JSON.parse(json.stdout).snapshot.panes[0].command).toEqual(command);
      expect(JSON.parse(json.stdout).snapshot.capturedAtMs).toBe(saved.capturedAtMs);
      // A wall-clock rollback cannot turn recovery age into an underflow.
      const future = { ...saved, capturedAtMs: Date.now() + 86_400_000 };
      fs.writeFileSync(latest, JSON.stringify(future));
      const futureHuman = await runCli(sandbox, ['workspace', 'show', '--socket', socket]);
      expect(futureHuman.status).toBe(0);
      expect(futureHuman.stdout).toContain('(captured just now)');
      expect(fs.existsSync(tripwire)).toBe(false);
    });
  });

  it('refuses absent, malformed or symlinked snapshots without initializing or repairing them', async () => {
    await withSandbox(async (sandbox) => {
      const tripwire = installTmuxTripwire(sandbox);
      delete sandbox.env.TMUX;
      const selection = await runCli(sandbox, ['workspace', 'show', '--json']);
      expect(selection.status).toBe(1);
      expect(JSON.parse(selection.stdout).error.code).toBe('WORKSPACE_SELECTION_REQUIRED');
      const socket = path.join(sandbox.root, 'gone-socket');
      const directory = path.join(
        sandbox.globalDir,
        'workspace',
        createHash('sha256').update(socket).digest('hex')
      );
      const args = ['workspace', 'show', '--socket', socket, '--json'];
      expect((await runCli(sandbox, args)).status).toBe(1);
      expect(fs.existsSync(directory)).toBe(false);
      fs.mkdirSync(directory, { recursive: true, mode: 0o700 });
      const latest = path.join(directory, 'latest.json');
      fs.writeFileSync(latest, '{"version":999}');
      expect((await runCli(sandbox, args)).status).toBe(1);
      expect(fs.readFileSync(latest, 'utf8')).toBe('{"version":999}');
      fs.unlinkSync(latest);
      const foreign = path.join(sandbox.root, 'foreign.json');
      fs.writeFileSync(foreign, JSON.stringify(snapshot(socket)));
      fs.symlinkSync(foreign, latest);
      expect((await runCli(sandbox, args)).status).toBe(1);
      expect(fs.lstatSync(latest).isSymbolicLink()).toBe(true);
      expect(fs.existsSync(sandbox.database)).toBe(false);
      expect(fs.existsSync(tripwire)).toBe(false);
    });
  });
});
