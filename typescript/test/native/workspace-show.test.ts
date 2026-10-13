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

describe('named workspace snapshots', () => {
  it('copies exact bytes, lists/selects per socket, enforces the cap and permits explicit removal', async () => {
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
      const bytes = `${JSON.stringify(snapshot(socket))}\n`;
      fs.writeFileSync(latest, bytes, { mode: 0o600 });
      const digest = createHash('sha256').update(bytes).digest('hex');
      const cli = (args: string[]) =>
        runCli(sandbox, ['workspace', ...args, '--socket', socket, '--json']);
      const saved = await cli(['save', 'work']);
      expect(saved.status).toBe(0);
      expect(JSON.parse(saved.stdout)).toEqual({
        version: 1,
        socket,
        name: 'work',
        digest,
        capturedAtMs: 1,
      });
      const named = path.join(directory, 'saved-work.json');
      expect(fs.readFileSync(named, 'utf8')).toBe(bytes);
      expect(fs.statSync(named).mode & 0o777).toBe(0o600);
      const duplicate = await cli(['save', 'work']);
      expect(duplicate.status).toBe(5);
      expect(JSON.parse(duplicate.stdout).error.code).toBe('WORKSPACE_SNAPSHOT_EXISTS');
      const preview = await cli(['show', 'work']);
      expect(preview.status).toBe(0);
      expect(JSON.parse(preview.stdout)).toMatchObject({
        name: 'work',
        digest,
        snapshot: snapshot(socket),
      });
      const updated = { ...snapshot(socket), capturedAtMs: 2 };
      fs.writeFileSync(latest, JSON.stringify(updated));
      const listed = await cli(['ls']);
      expect(listed.status).toBe(0);
      expect(JSON.parse(listed.stdout)).toEqual({
        version: 1,
        socket,
        snapshots: [
          {
            name: null,
            digest: createHash('sha256').update(JSON.stringify(updated)).digest('hex'),
            capturedAtMs: 2,
          },
          { name: 'work', digest, capturedAtMs: 1 },
        ],
      });
      expect(fs.readFileSync(named, 'utf8')).toBe(bytes);
      const other = await runCli(sandbox, [
        'workspace',
        'ls',
        '--socket',
        `${socket}-other`,
        '--json',
      ]);
      expect(other.status).toBe(0);
      expect(JSON.parse(other.stdout).snapshots).toEqual([]);
      for (let index = 0; index < 15; index++)
        expect((await cli(['save', `name-${index}`])).status).toBe(0);
      const cap = await cli(['save', 'overflow']);
      expect(cap.status).toBe(1);
      expect(JSON.parse(cap.stdout).error.code).toBe('WORKSPACE_SNAPSHOT_LIMIT');
      expect(fs.existsSync(path.join(directory, 'saved-overflow.json'))).toBe(false);
      expect((await cli(['rm', 'work'])).status).toBe(0);
      expect(fs.existsSync(named)).toBe(false);
      const missing = await cli(['show', 'work']);
      expect(missing.status).toBe(3);
      expect(JSON.parse(missing.stdout).error.code).toBe('WORKSPACE_SNAPSHOT_NOT_FOUND');
      expect((await cli(['save', 'work'])).status).toBe(0);
      expect(fs.readFileSync(named)).toEqual(fs.readFileSync(latest));
      expect(fs.readdirSync(directory).some((name) => name.endsWith('.tmp'))).toBe(false);
      expect(fs.existsSync(sandbox.database)).toBe(false);
      expect(fs.existsSync(tripwire)).toBe(false);
    });
  });

  it('rejects changed raw bytes before any restore effect, and reports invalid syntax as usage errors', async () => {
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
      fs.writeFileSync(latest, JSON.stringify(snapshot(socket)), { mode: 0o600 });
      const preview = await runCli(sandbox, ['workspace', 'show', '--socket', socket, '--json']);
      expect(preview.status).toBe(0);
      const { digest } = JSON.parse(preview.stdout);
      // Change only bytes, retaining the same valid topology and selected socket.
      fs.appendFileSync(latest, '\n');
      const changed = fs.readFileSync(latest);
      for (const flags of [[], ['--layout-only']]) {
        const result = await runCli(sandbox, [
          'workspace',
          'restore',
          ...flags,
          '--socket',
          socket,
          '--expect',
          digest,
          '--json',
        ]);
        expect(result.status).toBe(5);
        expect(JSON.parse(result.stdout).error.code).toBe('WORKSPACE_SNAPSHOT_CHANGED');
        expect(result.stderr).toBe('');
        expect(fs.readFileSync(latest)).toEqual(changed);
        expect(fs.existsSync(sandbox.database)).toBe(false);
        expect(fs.existsSync(tripwire)).toBe(false);
        expect(fs.readdirSync(directory)).toEqual(['latest.json']);
      }
      for (const args of [
        ['save', '../escape'],
        ['rm', 'latest'],
        ['restore', '--expect', 'invalid'],
        ['list'],
      ]) {
        const result = await runCli(sandbox, ['workspace', ...args, '--json']);
        expect(result.status).toBe(1);
        expect(JSON.parse(result.stdout).error.code).toBe('USAGE_ERROR');
      }
      expect(fs.existsSync(tripwire)).toBe(false);
    });
  });
});
