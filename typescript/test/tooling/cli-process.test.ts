import childProcess from 'node:child_process';
import { createHash } from 'node:crypto';
import { PassThrough } from 'node:stream';
import { writeExecutable } from '../support/executable-fixture.mjs';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import net from 'node:net';
import { syncBuiltinESMExports } from 'node:module';
import { afterEach, expect, it, vi } from 'vite-plus/test';
import { createSandbox, runCli, withSandbox } from '../support/cli-process.js';

const roots: string[] = [];
function fixture(mode = 'exit', output = 'ignore') {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'tmt-process-lifetime-'));
  roots.push(root);
  const marker = path.join(root, 'ready.json');
  const script = path.join(root, 'peer.mjs');
  writeExecutable(
    script,
    `
    import { execFileSync, spawn } from 'node:child_process';
    import fs from 'node:fs';
    if (process.argv[2] === 'child') {
      process.send('ready');
      setInterval(() => {}, 1000);
    } else {
      const child = spawn(process.execPath, [import.meta.filename, 'child'], {
        stdio: ['ignore', '${output}', '${output}', 'ipc'],
      });
      child.once('message', () => {
        const group = Number(execFileSync('/bin/ps', ['-o', 'pgid=', '-p', String(process.pid)], { encoding: 'utf8' }).trim());
        const launcherGroup = Number(execFileSync('/bin/ps', ['-o', 'pgid=', '-p', String(process.ppid)], { encoding: 'utf8' }).trim());
        const pendingMarker = process.argv[2] + '.pending';
        fs.writeFileSync(pendingMarker, JSON.stringify({ child: child.pid, group, launcherGroup }));
        fs.renameSync(pendingMarker, process.argv[2]);
        if (process.argv[3] === 'exit') process.exit(0);
        if (process.argv[3] === 'overflow') process.stdout.write('over the limit');
      });
    }
  `,
    0o644
  );
  const cli = { executable: process.execPath, args: [script, marker, mode] };
  return { root, marker, cli };
}
function alive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ESRCH') return false;
    throw error;
  }
}
async function until(condition: () => boolean) {
  const deadline = performance.now() + 2000;
  while (!condition()) {
    if (performance.now() >= deadline) throw new Error('Fixture observation timed out.');
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
}
async function cleanup(marker: string) {
  if (!fs.existsSync(marker)) return;
  const { group } = JSON.parse(fs.readFileSync(marker, 'utf8')) as { group: number };
  if (alive(-group)) process.kill(-group, 'SIGKILL');
  await until(() => !alive(-group));
}
afterEach(() => {
  for (const root of roots.splice(0)) fs.rmSync(root, { recursive: true, force: true });
});

it.skipIf(process.platform !== 'linux').each(['discovery', 'recheck'])(
  'cwd permission denial during %s keeps the guard within its inspection boundary',
  async (phase) => {
    const readlink = fs.readlinkSync.bind(fs);
    const denied = Object.assign(new Error('Simulated cwd inspection denial'), { code: 'EACCES' });
    let sandboxRoot = '';
    let inspections = 0;
    const inspection = vi.spyOn(fs, 'readlinkSync').mockImplementation((target, options) => {
      if (String(target) === `/proc/${process.pid}/cwd`) {
        inspections++;
        if (phase === 'recheck' && inspections === 1) return sandboxRoot;
        throw denied;
      }
      return readlink(target, options);
    });
    syncBuiltinESMExports();
    try {
      const result = withSandbox(async (sandbox) => {
        sandboxRoot = sandbox.root;
        roots.push(sandboxRoot);
        return 'callback completed';
      });
      if (phase === 'discovery') {
        await expect(result).resolves.toBe('callback completed');
        expect(fs.existsSync(sandboxRoot)).toBe(false);
      } else {
        const error = await result.catch((error: unknown) => error);
        expect(error).toBeInstanceOf(AggregateError);
        expect((error as AggregateError).errors).toContain(denied);
        expect((error as Error).message).toContain('retained fixture');
        expect(fs.existsSync(sandboxRoot)).toBe(true);
      }
      expect(inspections).toBe(phase === 'discovery' ? 1 : 2);
    } finally {
      inspection.mockRestore();
      syncBuiltinESMExports();
    }
  }
);

it('owns the default tmux socket directory and removes it with the sandbox', async () => {
  let socketRoot = '';
  const sentinel = fs.mkdtempSync(path.join(os.tmpdir(), 'tmt-parent-environment-'));
  roots.push(sentinel);
  fs.writeFileSync(path.join(sentinel, 'marker'), 'parent fixture');
  const pollutedKeys = [
    'TMUX',
    'TMUX_PANE',
    'TMUX_TEAM_HOME',
    'TMT_DRIVER_CALL',
    'NO_COLOR',
    'FORCE_COLOR',
    'CODEX_THREAD_ID',
    'CLAUDECODE',
    'PI_CODING_AGENT_DIR',
    'OPENCODE_CONFIG_DIR',
  ];
  for (const key of pollutedKeys) vi.stubEnv(key, 'parent fixture');
  vi.stubEnv('HOME', sentinel);
  vi.stubEnv('TMUX_TMPDIR', '/not-the-test-owned-tmux-directory');
  try {
    await withSandbox(async (sandbox) => {
      socketRoot = path.join(sandbox.root, 'tmux');
      expect(sandbox.env.TMUX_TMPDIR).toBe(socketRoot);
      expect(fs.statSync(socketRoot).isDirectory()).toBe(true);
      const result = await runCli(
        {
          ...sandbox,
          cli: {
            executable: process.execPath,
            args: ['-e', 'process.stdout.write(JSON.stringify(process.env))'],
          },
        },
        []
      );
      expect(result.status).toBe(0);
      const childEnv = JSON.parse(result.stdout);
      expect(childEnv).toMatchObject({
        HOME: sandbox.home,
        XDG_CONFIG_HOME: sandbox.xdgConfigHome,
        XDG_DATA_HOME: path.join(sandbox.root, 'xdg-data'),
        XDG_STATE_HOME: path.join(sandbox.root, 'xdg-state'),
        XDG_CACHE_HOME: path.join(sandbox.root, 'xdg-cache'),
        CODEX_HOME: path.join(sandbox.home, '.codex'),
        TMPDIR: path.join(sandbox.root, 'tmp'),
        TMUX_TMPDIR: socketRoot,
        PATH: [path.dirname(process.execPath), '/usr/bin', '/bin', '/usr/sbin', '/sbin'].join(
          path.delimiter
        ),
        LANG: 'en_US.UTF-8',
        LC_ALL: 'en_US.UTF-8',
      });
      for (const key of pollutedKeys) expect(childEnv).not.toHaveProperty(key);
      expect(fs.readdirSync(sentinel)).toEqual(['marker']);
      expect(fs.readFileSync(path.join(sentinel, 'marker'), 'utf8')).toBe('parent fixture');
      expect(result.stderr).toBe('');
    });
    expect(fs.existsSync(socketRoot)).toBe(false);
  } finally {
    vi.unstubAllEnvs();
  }
});

it('passes only the declared runtime connection variables to sandbox children', async () => {
  const sessionBus = 'unix:path=/test-owned/session-bus';
  vi.stubEnv('DBUS_SESSION_BUS_ADDRESS', sessionBus);
  const excludedKeys = [
    'DBUS_SYSTEM_BUS_ADDRESS',
    'GNOME_KEYRING_CONTROL',
    'XDG_RUNTIME_DIR',
    'FIREBASE_AUTH_EMULATOR_HOST',
    'TMT_TEST_BROWSER_CHANNEL',
  ];
  for (const key of excludedKeys) vi.stubEnv(key, 'unrelated parent value');
  const observe = () =>
    withSandbox(async (sandbox) => {
      const result = await runCli(
        {
          ...sandbox,
          cli: {
            executable: process.execPath,
            args: ['-e', 'console.log(JSON.stringify(process.env))'],
          },
        },
        []
      );
      expect(result.status).toBe(0);
      const childEnv = JSON.parse(result.stdout);
      expect(childEnv.HOME).toBe(sandbox.home);
      expect(childEnv.XDG_CONFIG_HOME).toBe(sandbox.xdgConfigHome);
      for (const key of [...excludedKeys, 'TMT_TEST_CLI', 'TMT_TEST_PEER_CLI'])
        expect(childEnv).not.toHaveProperty(key);
      return childEnv;
    });
  try {
    expect(await observe()).toHaveProperty('DBUS_SESSION_BUS_ADDRESS', sessionBus);
    vi.stubEnv('DBUS_SESSION_BUS_ADDRESS', undefined);
    expect(await observe()).not.toHaveProperty('DBUS_SESSION_BUS_ADDRESS');
  } finally {
    vi.unstubAllEnvs();
  }
});

it.each(['ignore', 'inherit'])(
  'normal parent exit cleans descendants with %s output',
  async (output) => {
    const f = fixture('exit', output);
    const sandbox = createSandbox({ TMT_TEST_CLI: JSON.stringify(f.cli) });
    roots.push(sandbox.root);
    try {
      expect(await runCli(sandbox, [])).toEqual({
        status: 0,
        signal: null,
        stdout: '',
        stderr: '',
      });
      const pids = JSON.parse(fs.readFileSync(f.marker, 'utf8')) as {
        child: number;
        group: number;
      };
      expect(alive(pids.child)).toBe(false);
      expect(alive(-pids.group)).toBe(false);
      expect(fs.existsSync(sandbox.root)).toBe(true);
    } finally {
      await cleanup(f.marker);
    }
  }
);

it('synchronous spawn rejection does not leave sandbox disposal waiting', async () => {
  let root = '';
  await withSandbox(async (sandbox) => {
    root = sandbox.root;
    await expect(runCli({ ...sandbox, cli: { executable: '\0', args: [] } }, [])).rejects.toThrow();
  });
  expect(fs.existsSync(root)).toBe(false);
});

it('disposed sandbox clones reject new runs without recreating files', async () => {
  const sandbox = await withSandbox((sandbox) => ({ ...sandbox }));
  await expect(runCli(sandbox, [])).rejects.toThrow('CLI sandbox is closing');
  expect(fs.existsSync(sandbox.root)).toBe(false);
});

it('asynchronous spawn failure preserves the error and disposes the sandbox', async () => {
  let root = '';
  await withSandbox(async (sandbox) => {
    root = sandbox.root;
    const cli = { executable: path.join(root, 'missing-command'), args: [] };
    await expect(runCli({ ...sandbox, cli }, [])).rejects.toMatchObject({ code: 'ENOENT' });
  });
  expect(fs.existsSync(root)).toBe(false);
});

it('runs beneath a PID-1-owned supervisor and preserves argv, stdin and both streams', async () => {
  await withSandbox(async (sandbox) => {
    const script = path.join(sandbox.root, 'observe-parent.mjs');
    writeExecutable(
      script,
      `import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
const grandparent = Number(execFileSync('/bin/ps', ['-o', 'ppid=', '-p', String(process.ppid)], { encoding: 'utf8' }).trim());
const group = Number(execFileSync('/bin/ps', ['-o', 'pgid=', '-p', String(process.pid)], { encoding: 'utf8' }).trim());
process.stdout.write(JSON.stringify({ grandparent, groupLeader: group === process.pid, args: process.argv.slice(2), stdin: fs.readFileSync(0, 'utf8') }));
process.stderr.write('diagnostic 雪');
process.exit(17);
`,
      0o644
    );
    const result = await runCli(
      { ...sandbox, cli: { executable: process.execPath, args: [script, 'prefix with spaces'] } },
      ['quote\"; $HOME', '雪'],
      { stdin: 'input\0雪\n' }
    );
    expect(result).toMatchObject({ status: 17, signal: null, stderr: 'diagnostic 雪' });
    expect(JSON.parse(result.stdout)).toEqual({
      grandparent: 1,
      groupLeader: true,
      args: ['prefix with spaces', 'quote\"; $HOME', '雪'],
      stdin: 'input\0雪\n',
    });
  });
});

it('keeps buffered and open stdin alive after setup exits', async () => {
  await withSandbox(async (sandbox) => {
    const script = path.join(sandbox.root, 'read-input.mjs');
    const marker = path.join(sandbox.root, 'input-ready');
    writeExecutable(
      script,
      `import fs from 'node:fs';
fs.writeFileSync(process.argv[2], 'ready');
process.stdout.write(String(fs.readFileSync(0).length));
`,
      0o644
    );
    const selected = { ...sandbox, cli: { executable: process.execPath, args: [script, marker] } };
    expect(await runCli(selected, [], { stdin: Buffer.alloc(1024 * 1024, 97) })).toEqual({
      status: 0,
      signal: null,
      stdout: String(1024 * 1024),
      stderr: '',
    });
    fs.unlinkSync(marker);
    const pending = runCli(selected, [], {
      stdin: 'partial',
      closeStdin: false,
      deadlineMs: 1000,
    });
    await until(() => fs.existsSync(marker));
    await expect(pending).rejects.toThrow('exceeded the 1000 millisecond test bound');
  });
});

it('cancellation before setup starts closes and removes the private control socket', async () => {
  const prefix = `tmt-cli-parent-${process.pid}-`;
  const makeDirectory = fs.mkdtempSync.bind(fs);
  const directories = vi.spyOn(fs, 'mkdtempSync');
  const servers = vi.spyOn(net, 'createServer');
  syncBuiltinESMExports();
  const failure = new Error('callback failed before setup');
  let root = '';
  let peerRoot = '';
  try {
    await expect(
      withSandbox((sandbox) => {
        root = sandbox.root;
        void runCli(sandbox, []);
        // Threaded workers share a PID. Another worker's live socket directory
        // is not evidence that this run leaked its own transport.
        peerRoot = makeDirectory(`/tmp/${prefix}`);
        roots.push(peerRoot);
        fs.writeFileSync(path.join(peerRoot, 'marker'), 'foreign control');
        throw failure;
      })
    ).rejects.toBe(failure);
    const controlRoots = directories.mock.calls.flatMap(([template], index) =>
      String(template) === `/tmp/${prefix}` ? [directories.mock.results[index].value] : []
    );
    expect(controlRoots).toHaveLength(1);
    expect(typeof controlRoots[0]).toBe('string');
    expect(fs.existsSync(controlRoots[0])).toBe(false);
    expect(servers).toHaveBeenCalledOnce();
    expect(servers.mock.results[0].value.listening).toBe(false);
    expect(fs.existsSync(root)).toBe(false);
    expect(fs.readFileSync(path.join(peerRoot, 'marker'), 'utf8')).toBe('foreign control');
  } finally {
    directories.mockRestore();
    servers.mockRestore();
    syncBuiltinESMExports();
  }
});

it('resolves a scenario executable on its declared PATH and preserves access errors', async () => {
  await withSandbox(async (sandbox) => {
    const bin = path.join(sandbox.root, 'bin');
    fs.mkdirSync(bin);
    const command = path.join(bin, 'scenario-command');
    writeExecutable(command, '#!/bin/sh\nprintf "%s" "$1"\n', 0o755);
    const selected = {
      ...sandbox,
      cli: { executable: 'scenario-command', args: [] },
      env: { ...sandbox.env, PATH: `${bin}${path.delimiter}${sandbox.env.PATH}` },
    };
    expect(await runCli(selected, ['argument with spaces'])).toEqual({
      status: 0,
      signal: null,
      stdout: 'argument with spaces',
      stderr: '',
    });
    fs.chmodSync(command, 0o644);
    await expect(runCli(selected, [])).rejects.toMatchObject({ code: 'EACCES' });
    fs.unlinkSync(command);
    await expect(runCli(selected, [])).rejects.toMatchObject({ code: 'ENOENT' });
  });
});

it('relays a selected CLI signal rather than converting it to an exit code', async () => {
  await withSandbox(async (sandbox) => {
    const result = await runCli(
      { ...sandbox, cli: { executable: '/bin/sh', args: ['-c', 'kill -TERM $$'] } },
      []
    );
    expect(result).toEqual({ status: null, signal: 'SIGTERM', stdout: '', stderr: '' });
  });
});

it('deadline termination stops the supervisor, CLI and descendants before disposal', async () => {
  const f = fixture('hold');
  let root = '';
  try {
    await withSandbox(async (sandbox) => {
      root = sandbox.root;
      // Observe a live CLI and descendant before timeout, so startup failure
      // cannot satisfy the deadline-cleanup assertions.
      const pending = runCli({ ...sandbox, cli: f.cli }, [], { deadlineMs: 1000 });
      await until(() => fs.existsSync(f.marker));
      await expect(pending).rejects.toThrow('exceeded the 1000 millisecond test bound');
      const { child, group, launcherGroup } = JSON.parse(fs.readFileSync(f.marker, 'utf8'));
      expect(alive(child)).toBe(false);
      expect(alive(-group)).toBe(false);
      expect(alive(-launcherGroup)).toBe(false);
    });
    expect(fs.existsSync(root)).toBe(false);
  } finally {
    await cleanup(f.marker);
  }
});

it('output overflow stops the complete group and preserves its bound error', async () => {
  const f = fixture('overflow');
  try {
    await withSandbox(async (sandbox) => {
      await expect(runCli({ ...sandbox, cli: f.cli }, [], { outputLimitBytes: 1 })).rejects.toThrow(
        'CLI subprocess exceeded the 1-byte output bound.'
      );
      const { group } = JSON.parse(fs.readFileSync(f.marker, 'utf8')) as { group: number };
      expect(alive(-group)).toBe(false);
    });
  } finally {
    await cleanup(f.marker);
  }
});

it('successful callback disposal stops concurrent runs shared through clones', async () => {
  const fixtures = [fixture('hold'), fixture('hold')];
  const runs: Promise<unknown>[] = [];
  let root = '';
  try {
    const value = await withSandbox(async (sandbox) => {
      root = sandbox.root;
      for (const f of fixtures) runs.push(runCli({ ...sandbox, cli: f.cli }, []));
      await until(() => fixtures.every((f) => fs.existsSync(f.marker)));
      return 'complete';
    });
    expect(value).toBe('complete');
    for (const f of fixtures) {
      const { group } = JSON.parse(fs.readFileSync(f.marker, 'utf8')) as { group: number };
      expect(alive(-group)).toBe(false);
    }
    expect(fs.existsSync(root)).toBe(false);
    for (const run of runs) await expect(run).rejects.toThrow('cancelled during sandbox disposal');
  } finally {
    for (const f of fixtures) await cleanup(f.marker);
    await Promise.allSettled(runs);
  }
});

it('unconfirmed group exit is bounded and retains files and the original failure', async () => {
  const f = fixture('hold');
  const failure = new Error('Original scenario failure');
  const kill = process.kill.bind(process);
  let probe: { mockRestore(): void } | undefined;
  let sandboxRoot = '';
  let pending: Promise<unknown> | undefined;
  try {
    const disposal = withSandbox(async (sandbox) => {
      sandboxRoot = sandbox.root;
      roots.push(sandboxRoot);
      pending = runCli({ ...sandbox, cli: f.cli }, []);
      await until(() => fs.existsSync(f.marker));
      const { group } = JSON.parse(fs.readFileSync(f.marker, 'utf8')) as { group: number };
      // Still deliver the real cleanup signal. Only this fixture's observation
      // is held alive to prove that absence, not a successful kill, is required.
      probe = vi
        .spyOn(process, 'kill')
        .mockImplementation((pid, signal) =>
          pid === -group && signal === 0 ? true : kill(pid, signal)
        );
      throw failure;
    });
    const error = await disposal.catch((error: unknown) => error);
    expect(error).toBeInstanceOf(AggregateError);
    expect((error as AggregateError).errors).toContain(failure);
    expect((error as Error).message).toContain('retained fixture');
    expect(fs.existsSync(sandboxRoot)).toBe(true);
    await expect(pending).rejects.toThrow('within 1000ms');
  } finally {
    probe?.mockRestore();
    await cleanup(f.marker);
    await pending?.catch(() => {});
  }
}, 5_000);

it.each(['signal', 'probe'])(
  '%s EPERM resolves only after direct close and confirmed group absence',
  async (denial) => {
    const f = fixture('exit');
    const kill = process.kill.bind(process);
    let signalled = 0;
    let absent = false;
    let probeDenied = false;
    const probe = vi.spyOn(process, 'kill').mockImplementation((pid, signal) => {
      const group = fs.existsSync(f.marker)
        ? (JSON.parse(fs.readFileSync(f.marker, 'utf8')) as { group: number }).group
        : undefined;
      if (
        denial === 'probe' &&
        !probeDenied &&
        group !== undefined &&
        pid === -group &&
        signal === 0
      ) {
        probeDenied = true;
        // Emulate independently exiting fixture members, then deny the stale probe.
        // The harness must not send a signal after this unconfirmed observation.
        kill(pid, 'SIGKILL');
        throw Object.assign(new Error('Simulated probe exit race'), { code: 'EPERM' });
      }
      if (group !== undefined && pid === -group && signal === 'SIGKILL') {
        signalled++;
        if (denial === 'probe') return kill(pid, signal);
        // Deliver the real signal, then emulate the error from a concurrent exit.
        kill(pid, signal);
        throw Object.assign(new Error('Simulated signal exit race'), { code: 'EPERM' });
      }
      try {
        return kill(pid, signal);
      } catch (error) {
        if (
          group !== undefined &&
          pid === -group &&
          signal === 0 &&
          (error as NodeJS.ErrnoException).code === 'ESRCH'
        ) {
          absent = true;
        }
        throw error;
      }
    });
    try {
      await withSandbox(async (sandbox) => {
        const result = await runCli({ ...sandbox, cli: f.cli }, []);
        expect(result.status).toBe(0);
        expect(result.signal).toBeNull();
        expect(signalled).toBe(denial === 'signal' ? 1 : 0);
        expect(probeDenied).toBe(denial === 'probe');
        expect(absent).toBe(true);
        const { child, group } = JSON.parse(fs.readFileSync(f.marker, 'utf8'));
        expect(alive(child)).toBe(false);
        expect(alive(-group)).toBe(false);
      });
    } finally {
      probe.mockRestore();
      await cleanup(f.marker);
    }
  }
);

it.each(['signal', 'probe'])(
  '%s EPERM with a live group fails bounded cleanup and retains files',
  async (denial) => {
    const f = fixture('exit');
    const kill = process.kill.bind(process);
    let signalled = 0;
    let sandboxRoot = '';
    let probeDenied = false;
    const probe = vi.spyOn(process, 'kill').mockImplementation((pid, signal) => {
      const group = fs.existsSync(f.marker)
        ? (JSON.parse(fs.readFileSync(f.marker, 'utf8')) as { group: number }).group
        : undefined;
      if (
        denial === 'probe' &&
        !probeDenied &&
        group !== undefined &&
        pid === -group &&
        signal === 0
      ) {
        probeDenied = true;
        throw Object.assign(new Error('Simulated inspection denial'), { code: 'EPERM' });
      }
      if (group !== undefined && pid === -group && signal === 'SIGKILL') {
        signalled++;
        if (denial === 'probe') return kill(pid, signal);
        throw Object.assign(new Error('Simulated permission denial'), { code: 'EPERM' });
      }
      return kill(pid, signal);
    });
    try {
      const error = await withSandbox(async (sandbox) => {
        sandboxRoot = sandbox.root;
        roots.push(sandboxRoot);
        await runCli({ ...sandbox, cli: f.cli }, []);
      }).catch((error: unknown) => error);
      expect(error).toBeInstanceOf(AggregateError);
      expect((error as Error).message).toContain('retained fixture');
      expect((error as AggregateError).errors).toEqual(
        expect.arrayContaining([
          expect.objectContaining({
            message: 'CLI process cleanup did not confirm close and group exit within 1000ms.',
          }),
        ])
      );
      expect(signalled).toBe(denial === 'signal' ? 1 : 0);
      expect(probeDenied).toBe(denial === 'probe');
      expect(fs.existsSync(sandboxRoot)).toBe(true);
      const { child, group } = JSON.parse(fs.readFileSync(f.marker, 'utf8'));
      // Linux's independent cwd guard stops residents even when group cleanup fails.
      expect(alive(child)).toBe(process.platform !== 'linux');
      expect(alive(-group)).toBe(process.platform !== 'linux');
    } finally {
      probe.mockRestore();
      await cleanup(f.marker);
    }
  },
  5_000
);

it('callback failure stops an unawaited run before removing the sandbox', async () => {
  const f = fixture('hold');
  let sandboxRoot = '';
  let pending: Promise<unknown> | undefined;
  const failure = new Error('Scenario failed deliberately');
  try {
    await expect(
      withSandbox(async (sandbox) => {
        sandboxRoot = sandbox.root;
        pending = runCli({ ...sandbox, cli: f.cli }, []);
        void pending.catch(() => {});
        await until(() => fs.existsSync(f.marker));
        throw failure;
      })
    ).rejects.toBe(failure);
    const { group } = JSON.parse(fs.readFileSync(f.marker, 'utf8')) as { group: number };
    expect(alive(-group)).toBe(false);
    expect(fs.existsSync(sandboxRoot)).toBe(false);
  } finally {
    await cleanup(f.marker);
    await pending?.catch(() => {});
  }
});

// These diagnostic controls never spawn a native fixture or send an OS signal.
function diagnosticFixture(adopt = true, confirmCleanup = true) {
  vi.useFakeTimers();
  const child = new childProcess.ChildProcess();
  const stdout = new PassThrough();
  const stderr = new PassThrough();
  Object.assign(child, { pid: 900001, stdout, stderr });
  vi.spyOn(child, 'unref').mockImplementation(() => {});
  const server = new net.Server();
  const socket = new net.Socket();
  let listening = false;
  let present = true;
  vi.spyOn(server, 'listening', 'get').mockImplementation(() => listening);
  vi.spyOn(server, 'listen').mockImplementation(() => {
    queueMicrotask(() => {
      listening = true;
      server.emit('listening');
    });
    return server;
  });
  vi.spyOn(server, 'close').mockImplementation((callback) => {
    listening = false;
    callback?.();
    return server;
  });
  vi.spyOn(socket, 'write').mockImplementation(() => true);
  vi.spyOn(socket, 'end').mockImplementation(() => socket);
  vi.spyOn(socket, 'destroy').mockImplementation(() => {
    queueMicrotask(() => socket.emit('close'));
    return socket;
  });
  const servers = vi.spyOn(net, 'createServer').mockImplementation((listener) => {
    if (typeof listener === 'function') server.on('connection', listener);
    return server;
  });
  const spawn = vi.spyOn(childProcess, 'spawn').mockImplementation(() => {
    queueMicrotask(() => {
      child.emit('spawn');
      if (adopt) {
        server.emit('connection', socket);
        socket.emit('data', JSON.stringify({ group: 900002 }) + '\n');
      }
    });
    return child;
  });
  const signals = vi.spyOn(process, 'kill').mockImplementation((pid, signal) => {
    if (![900001, 900002].includes(-pid)) throw new Error('Unexpected synthetic signal target');
    if (!present) throw Object.assign(new Error('synthetic absence'), { code: 'ESRCH' });
    if (signal === 'SIGKILL' && confirmCleanup) {
      present = false;
      queueMicrotask(() => {
        stdout.destroy();
        stderr.destroy();
        child.emit('exit', 0, null);
        child.emit('close', 0, null);
      });
    }
    return true;
  });
  const diagnostics = vi.spyOn(console, 'error').mockImplementation(() => {});
  syncBuiltinESMExports();
  return {
    stdout,
    stderr,
    signals,
    diagnostics,
    spawn,
    complete: () => {
      socket.emit('data', JSON.stringify({ status: 0, signal: null }) + '\n');
    },
    restore: () => {
      servers.mockRestore();
      spawn.mockRestore();
      signals.mockRestore();
      diagnostics.mockRestore();
      vi.restoreAllMocks();
      syncBuiltinESMExports();
      vi.useRealTimers();
    },
  };
}

it('diagnostic success preserves the exact result and emits no failure report', async () => {
  const f = diagnosticFixture();
  try {
    await withSandbox(
      async (sandbox) => {
        const pending = runCli({ ...sandbox, cli: { executable: process.execPath, args: [] } }, [
          '--version',
        ]);
        await vi.advanceTimersByTimeAsync(0);
        f.stdout.write('fixture output');
        f.complete();
        await vi.advanceTimersByTimeAsync(10);
        expect(await pending).toEqual({
          status: 0,
          signal: null,
          stdout: 'fixture output',
          stderr: '',
        });
        expect(f.diagnostics).not.toHaveBeenCalled();
        expect(f.spawn).toHaveBeenCalledOnce();
      },
      { TMT_TEST_CLI: JSON.stringify({ executable: process.execPath, args: [] }) }
    );
  } finally {
    f.restore();
  }
});

it.each([true, false])(
  'diagnostic timeout identifies adoption=%s with unchanged five-second bound',
  async (adopt) => {
    const f = diagnosticFixture(adopt);
    try {
      await withSandbox(
        async (sandbox) => {
          const secret = 'private fixture argument and output';
          const pending = runCli(
            { ...sandbox, cli: { executable: process.execPath, args: [secret] } },
            ['upgrade', '--json'],
            { stdin: secret }
          );
          const captured = pending.catch((error: unknown) => error);
          await vi.advanceTimersByTimeAsync(0);
          f.stdout.write(secret);
          f.stderr.write('partial stderr');
          await vi.advanceTimersByTimeAsync(4999);
          expect(f.diagnostics).not.toHaveBeenCalled();
          await vi.advanceTimersByTimeAsync(11);
          expect(await captured).toMatchObject({
            message: 'CLI subprocess exceeded the 5000 millisecond test bound.',
          });
          expect(f.diagnostics).toHaveBeenCalledOnce();
          const [label, text] = f.diagnostics.mock.calls[0];
          const report = JSON.parse(String(text));
          expect(label).toBe('CLI call diagnostics:');
          expect(String(text)).not.toContain(secret);
          expect(report).toMatchObject({
            callId: 1,
            sandboxRoot: sandbox.root,
            executable: process.execPath,
            deadlineMs: 5000,
            fixtureArgv: [
              {
                bytes: Buffer.byteLength(secret),
                sha256: createHash('sha256').update(secret).digest('hex'),
              },
            ],
            argv: [{ literal: 'upgrade' }, { literal: '--json' }],
            partialOutput: {
              stdout: {
                bytes: Buffer.byteLength(secret),
                sha256: createHash('sha256').update(secret).digest('hex'),
              },
              stderr: {
                bytes: 14,
                sha256: createHash('sha256').update('partial stderr').digest('hex'),
              },
            },
            cleanup: { closeObserved: true, remainingGroups: [], failed: false },
          });
          const phases = report.events.map((event: { phase: string }) => event.phase);
          expect(phases).toEqual(
            expect.arrayContaining([
              'call-start',
              'socket-listen-requested',
              'socket-listening',
              'setup-spawn-requested',
              'setup-spawn-returned',
              'deadline',
              'cleanup-start',
              'setup-exit',
              'setup-pipes-closed',
              'cleanup-settled',
            ])
          );
          expect(phases.includes('cli-group-adopted')).toBe(adopt);
          expect(phases.includes('ready-acknowledged')).toBe(adopt);
          expect(
            report.events.every(
              (event: { atMs: number }, index: number) =>
                index === 0 || event.atMs >= report.events[index - 1].atMs
            )
          ).toBe(true);
          expect(Buffer.byteLength(String(text))).toBeLessThanOrEqual(16384);
        },
        { TMT_TEST_CLI: JSON.stringify({ executable: process.execPath, args: [] }) }
      );
    } finally {
      f.restore();
    }
  }
);

it('diagnostic unconfirmed cleanup keeps the original errors and retained root', async () => {
  const f = diagnosticFixture(true, false);
  let root = '';
  try {
    const pending = withSandbox(
      async (sandbox) => {
        root = sandbox.root;
        roots.push(root);
        await runCli(sandbox, ['--version']);
      },
      { TMT_TEST_CLI: JSON.stringify({ executable: process.execPath, args: [] }) }
    );
    const captured = pending.catch((error: unknown) => error);
    await vi.advanceTimersByTimeAsync(6010);
    expect(await captured).toMatchObject({
      message: expect.stringContaining('CLI sandbox cleanup failed; retained fixture'),
    });
    expect(fs.existsSync(root)).toBe(true);
    const call = JSON.parse(
      String(f.diagnostics.mock.calls.find(([label]) => label === 'CLI call diagnostics:')?.[1])
    );
    expect(call.cleanup).toEqual({
      closeObserved: false,
      remainingGroups: [900001, 900002],
      failed: true,
    });
    expect(call.deadlineMs).toBe(5000);
    expect(f.diagnostics.mock.calls.map(([label]) => label)).toEqual([
      'CLI call diagnostics:',
      'CLI sandbox diagnostics:',
    ]);
  } finally {
    f.restore();
  }
});

it.each(['EACCES', 'EPERM'])(
  'diagnostic cwd %s denial stops inspection at discovery and recheck',
  async (code) => {
    for (const phase of ['discovery', 'recheck']) {
      const platform = Object.getOwnPropertyDescriptor(process, 'platform')!;
      const readdir = fs.readdirSync.bind(fs);
      const readlink = fs.readlinkSync.bind(fs);
      const read = fs.readFileSync.bind(fs);
      const stat = fs.statSync.bind(fs);
      const denied = Object.assign(new Error('Synthetic admitted cwd denial'), { code });
      const output = vi.spyOn(console, 'error').mockImplementation(() => {});
      const inspections: string[] = [];
      let root = '';
      let cwdReads = 0;
      Object.defineProperty(process, 'platform', { value: 'linux' });
      vi.spyOn(fs, 'readdirSync').mockImplementation((file, options) =>
        String(file) === '/proc'
          ? (['900005'] as unknown as ReturnType<typeof fs.readdirSync>)
          : readdir(file, options)
      );
      vi.spyOn(fs, 'statSync').mockImplementation((file, options) => {
        if (String(file).startsWith('/proc/')) {
          inspections.push(String(file));
          expect(String(file)).toBe('/proc/900005');
          return { uid: process.getuid!() } as fs.Stats;
        }
        return stat(file, options);
      });
      vi.spyOn(fs, 'readlinkSync').mockImplementation((file, options) => {
        if (String(file).startsWith('/proc/')) {
          inspections.push(String(file));
          expect(String(file)).toBe('/proc/900005/cwd');
          cwdReads++;
          if (phase === 'recheck' && cwdReads === 1) return fs.realpathSync(root);
          throw denied;
        }
        return readlink(file, options);
      });
      vi.spyOn(fs, 'readFileSync').mockImplementation((file, options) => {
        if (String(file).startsWith('/proc/')) {
          inspections.push(String(file));
          throw new Error('Diagnostic facts must not precede the admitted recheck');
        }
        return read(file, options);
      });
      const signals = vi.spyOn(process, 'kill').mockImplementation(() => {
        throw new Error('No signal or presence probe is admitted after cwd denial');
      });
      syncBuiltinESMExports();
      try {
        const result = withSandbox(
          (sandbox) => {
            root = sandbox.root;
            roots.push(root);
            return 'callback completed';
          },
          { TMT_TEST_CLI: JSON.stringify({ executable: process.execPath, args: [] }) }
        );
        if (phase === 'discovery') {
          await expect(result).resolves.toBe('callback completed');
          expect(fs.existsSync(root)).toBe(false);
          expect(output).not.toHaveBeenCalled();
        } else {
          const error = await result.catch((error: unknown) => error);
          expect(error).toBeInstanceOf(AggregateError);
          expect((error as AggregateError).errors).toContain(denied);
          expect((error as Error).message).toContain('retained fixture');
          expect(fs.existsSync(root)).toBe(true);
          const diagnostic = JSON.parse(String(output.mock.calls[0][1]));
          expect(diagnostic.residents).toEqual([
            { pid: 900005, observation: 'identity unavailable' },
          ]);
          expect(diagnostic.residentCount).toBe(1);
          expect(diagnostic.residentCleanup).toBe('not confirmed');
          expect(diagnostic.cleanupFailed).toBe(true);
        }
        const expected = ['/proc/900005', '/proc/900005/cwd'];
        expect(inspections).toEqual(phase === 'discovery' ? expected : [...expected, ...expected]);
        expect(cwdReads).toBe(phase === 'discovery' ? 1 : 2);
        expect(signals).not.toHaveBeenCalled();
      } finally {
        Object.defineProperty(process, 'platform', platform);
        vi.restoreAllMocks();
        syncBuiltinESMExports();
      }
    }
  }
);

it.each(['observed', 'denied', 'replacement', 'reused'])(
  'diagnostic resident %s is scoped and retains terminal-cleanup observation',
  async (kind) => {
    const platform = Object.getOwnPropertyDescriptor(process, 'platform')!;
    const readdir = fs.readdirSync.bind(fs);
    const readlink = fs.readlinkSync.bind(fs);
    const read = fs.readFileSync.bind(fs);
    const stat = fs.statSync.bind(fs);
    const output = vi.spyOn(console, 'error').mockImplementation(() => {});
    let root = '';
    let present = true;
    const facts: string[] = [];
    const inspectionOrder: string[] = [];
    Object.defineProperty(process, 'platform', { value: 'linux' });
    vi.spyOn(fs, 'readdirSync').mockImplementation((file, options) =>
      String(file) === '/proc'
        ? ((present ? ['900003', '900004'] : []) as unknown as ReturnType<typeof fs.readdirSync>)
        : readdir(file, options)
    );
    vi.spyOn(fs, 'statSync').mockImplementation((file, options) =>
      /^\/proc\/90000[34]$/.test(String(file))
        ? ({ uid: process.getuid!() } as fs.Stats)
        : stat(file, options)
    );
    vi.spyOn(fs, 'readlinkSync').mockImplementation((file, options) => {
      const name = String(file);
      if (name === '/proc/900003/cwd') {
        inspectionOrder.push('cwd');
        return kind === 'replacement' && facts.length ? '/foreign/root' : fs.realpathSync(root);
      }
      if (name === '/proc/900004/cwd') return '/foreign/root';
      if (name === '/proc/900003/exe') {
        facts.push(name);
        return '/owned/fixture';
      }
      return readlink(file, options);
    });
    vi.spyOn(fs, 'readFileSync').mockImplementation((file, options) => {
      const name = String(file);
      if (name.startsWith('/proc/')) {
        facts.push(name);
        inspectionOrder.push('stat');
        if (kind === 'denied') throw Object.assign(new Error('denied'), { code: 'EACCES' });
        return (
          '900003 (fixture with spaces) S 1 900003 900003 ' +
          Array(15).fill('0').join(' ') +
          (kind === 'reused' && facts.filter((name) => name.endsWith('/stat')).length > 1
            ? ' 54321'
            : ' 12345')
        );
      }
      return read(file, options);
    });
    const signals = vi.spyOn(process, 'kill').mockImplementation((pid, signal) => {
      expect(pid).toBe(900003);
      if (signal === 'SIGKILL') {
        inspectionOrder.push('signal');
        present = false;
        return true;
      }
      throw Object.assign(new Error('synthetic absence'), { code: 'ESRCH' });
    });
    syncBuiltinESMExports();
    try {
      expect(process.platform).toBe('linux');
      const error = await withSandbox(
        (sandbox) => {
          root = sandbox.root;
          return 'callback complete';
        },
        { TMT_TEST_CLI: JSON.stringify({ executable: process.execPath, args: [] }) }
      ).catch((error: unknown) => error);
      expect(fs.readdirSync).toHaveBeenCalledWith('/proc');
      expect(error).toMatchObject({ message: 'Sandbox callback left live processes: 900003.' });
      expect(fs.existsSync(root)).toBe(false);
      const diagnostic = JSON.parse(String(output.mock.calls[0][1]));
      expect(diagnostic.residentCount).toBe(1);
      expect(diagnostic.residentCleanup).toBe('absence confirmed by existing cleanup checks');
      expect(diagnostic.cleanupFailed).toBe(false);
      expect(diagnostic.residents[0]).toEqual(
        kind === 'observed'
          ? {
              pid: 900003,
              observation: 'endpoint identity observed',
              startTicks: '12345',
              state: 'S',
              ppid: '1',
              pgid: '900003',
              session: '900003',
              cwd: fs.realpathSync(path.dirname(root)) + path.sep + path.basename(root),
              executable: '/owned/fixture',
            }
          : {
              pid: 900003,
              observation:
                kind === 'denied'
                  ? 'identity unavailable'
                  : kind === 'reused'
                    ? 'identity changed'
                    : 'no longer a sandbox resident',
            }
      );
      expect(inspectionOrder.slice(0, 4)).toEqual(['cwd', 'cwd', 'signal', 'stat']);
      expect(facts.every((file) => file.startsWith('/proc/900003/'))).toBe(true);
      expect(JSON.stringify(diagnostic)).not.toContain('/foreign/root');
      expect(signals.mock.calls.every(([pid]) => pid === 900003)).toBe(true);
    } finally {
      Object.defineProperty(process, 'platform', platform);
      vi.restoreAllMocks();
      syncBuiltinESMExports();
    }
  }
);

it('diagnostic completion with unclosed pipes is distinct from an execution timeout', async () => {
  const f = diagnosticFixture(true, false);
  let root = '';
  try {
    const pending = withSandbox(
      async (sandbox) => {
        root = sandbox.root;
        roots.push(root);
        await runCli(sandbox, ['--version']);
      },
      { TMT_TEST_CLI: JSON.stringify({ executable: process.execPath, args: [] }) }
    );
    const captured = pending.catch((error: unknown) => error);
    await vi.advanceTimersByTimeAsync(0);
    f.complete();
    await vi.advanceTimersByTimeAsync(1010);
    expect(await captured).toMatchObject({ message: expect.stringContaining('retained fixture') });
    const report = JSON.parse(String(f.diagnostics.mock.calls[0][1]));
    const phases = report.events.map((event: { phase: string }) => event.phase);
    expect(phases).toContain('cli-completion');
    expect(phases).not.toContain('deadline');
    expect(report.cleanup).toMatchObject({ closeObserved: false, failed: true });
    expect(
      report.events.find((event: { phase: string }) => event.phase === 'cli-completion')
    ).toMatchObject({ status: 0, signal: null });
    expect(fs.existsSync(root)).toBe(true);
  } finally {
    f.restore();
  }
});

it('diagnostic reporting failure cannot replace timeout or interrupt disposal', async () => {
  const f = diagnosticFixture();
  let root = '';
  f.diagnostics.mockImplementation(() => {
    throw new Error('Synthetic reporting failure');
  });
  try {
    await withSandbox(
      async (sandbox) => {
        root = sandbox.root;
        const pending = runCli(sandbox, ['--version']).catch((error: unknown) => error);
        await vi.advanceTimersByTimeAsync(5010);
        expect(await pending).toMatchObject({
          message: 'CLI subprocess exceeded the 5000 millisecond test bound.',
        });
        expect(f.diagnostics).toHaveBeenCalledOnce();
      },
      { TMT_TEST_CLI: JSON.stringify({ executable: process.execPath, args: [] }) }
    );
    expect(fs.existsSync(root)).toBe(false);
  } finally {
    f.restore();
  }
});

it('diagnostic clones share invocation IDs without changing successful calls', async () => {
  const first = diagnosticFixture();
  let second: ReturnType<typeof diagnosticFixture> | undefined;
  try {
    await withSandbox(
      async (sandbox) => {
        const success = runCli(sandbox, ['--version']);
        await vi.advanceTimersByTimeAsync(0);
        first.complete();
        await vi.advanceTimersByTimeAsync(10);
        expect(await success).toEqual({ status: 0, signal: null, stdout: '', stderr: '' });
        first.restore();
        second = diagnosticFixture();
        const pending = runCli({ ...sandbox }, ['update', '--json']).catch(
          (error: unknown) => error
        );
        await vi.advanceTimersByTimeAsync(5010);
        expect(await pending).toMatchObject({
          message: 'CLI subprocess exceeded the 5000 millisecond test bound.',
        });
        const report = JSON.parse(String(second.diagnostics.mock.calls[0][1]));
        expect(report).toMatchObject({
          callId: 2,
          sandboxRoot: sandbox.root,
          argv: [{ literal: 'update' }, { literal: '--json' }],
        });
      },
      { TMT_TEST_CLI: JSON.stringify({ executable: process.execPath, args: [] }) }
    );
  } finally {
    second?.restore();
    first.restore();
  }
});

it('diagnostic long private fixture values remain hashed within the report budget', async () => {
  const f = diagnosticFixture();
  try {
    await withSandbox(
      async (sandbox) => {
        const privateValue = 'private fixture 雪'.repeat(1000);
        const pending = runCli(
          {
            ...sandbox,
            root: privateValue,
            cli: { executable: privateValue, args: Array(12).fill(privateValue) },
          },
          Array(12).fill(privateValue)
        ).catch((error: unknown) => error);
        await vi.advanceTimersByTimeAsync(5010);
        expect(await pending).toMatchObject({
          message: 'CLI subprocess exceeded the 5000 millisecond test bound.',
        });
        const text = String(f.diagnostics.mock.calls[0][1]);
        const report = JSON.parse(text);
        expect(Buffer.byteLength(text)).toBeLessThanOrEqual(16384);
        expect(text).not.toContain('private fixture');
        expect(report.omitted).toBeUndefined();
        expect(report).toMatchObject({
          fixtureArgc: 12,
          argc: 12,
          executableSha256: createHash('sha256').update(privateValue).digest('hex'),
          cleanup: { closeObserved: true, failed: false },
          deadlineMs: 5000,
        });
        expect(report.argv).toHaveLength(8);
        expect(report.fixtureArgv).toHaveLength(8);
      },
      { TMT_TEST_CLI: JSON.stringify({ executable: process.execPath, args: [] }) }
    );
  } finally {
    f.restore();
  }
});

it.each([
  'selected',
  'launcher',
  'discovery-denied',
  'discovery-denied-EPERM',
  'recheck-denied',
  'recheck-denied-EACCES',
  'endpoint-denied',
  'endpoint-denied-EPERM',
  'disappeared',
  'reused',
  'foreign',
  'stat-overflow',
  'wchan-overflow',
  'exe-overflow',
])('diagnostic pre-termination %s preserves the original deadline and cleanup', async (kind) => {
  const f = diagnosticFixture();
  const platform = Object.getOwnPropertyDescriptor(process, 'platform')!;
  const readdir = fs.readdirSync.bind(fs);
  const stat = fs.statSync.bind(fs);
  const readlink = fs.readlinkSync.bind(fs);
  const open = fs.openSync.bind(fs);
  const read = fs.readSync.bind(fs);
  const close = fs.closeSync.bind(fs);
  const endpoints: string[] = [];
  const files = new Map<number, string>();
  let root = '';
  let killed = false;
  let nextFd = 900100;
  let cliCwdReads = 0;
  let cliStatReads = 0;
  const residentPids = Array.from({ length: 12 }, (_, index) => 900010 + index);
  const signal = f.signals.getMockImplementation()!;
  f.signals.mockImplementation((pid, sig) => {
    if (sig === 'SIGKILL') killed = true;
    return signal(pid, sig);
  });
  Object.defineProperty(process, 'platform', { value: 'linux' });
  vi.spyOn(fs, 'readdirSync').mockImplementation((file, options) => {
    if (String(file) !== '/proc') return readdir(file, options);
    if (kind.startsWith('discovery-denied') && !killed)
      throw Object.assign(new Error('denied discovery'), {
        code: kind.endsWith('EPERM') ? 'EPERM' : 'EACCES',
      });
    return (killed ? [] : ['900002', ...residentPids.map(String)]) as unknown as ReturnType<
      typeof fs.readdirSync
    >;
  });
  vi.spyOn(fs, 'statSync').mockImplementation((file, options) =>
    /^\/proc\/\d+$/.test(String(file))
      ? ({ uid: process.getuid!() } as fs.Stats)
      : stat(file, options)
  );
  vi.spyOn(fs, 'readlinkSync').mockImplementation((file, options) => {
    const name = String(file);
    if (name.endsWith('/cwd') && name.startsWith('/proc/')) {
      if (name === '/proc/900002/cwd') {
        cliCwdReads++;
        if (kind.startsWith('recheck-denied') && cliCwdReads === 2)
          throw Object.assign(new Error('denied recheck'), {
            code: kind.endsWith('EACCES') ? 'EACCES' : 'EPERM',
          });
        if (kind === 'foreign') return '/foreign/root';
      }
      return root;
    }
    if (name.startsWith('/proc/') && name.endsWith('/exe')) {
      endpoints.push(name);
      if (kind === 'exe-overflow') return '/owned/' + 'x'.repeat(256);
      return kind === 'launcher' ? '/owned/runtime-caller-fixture' : '/owned/tmt';
    }
    return readlink(file, options);
  });
  vi.spyOn(fs, 'openSync').mockImplementation((file, flags, mode) => {
    const name = String(file);
    if (!name.startsWith('/proc/')) return open(file, flags, mode);
    expect(killed).toBe(false);
    endpoints.push(name);
    if (kind.startsWith('endpoint-denied'))
      throw Object.assign(new Error('denied endpoint'), {
        code: kind.endsWith('EPERM') ? 'EPERM' : 'EACCES',
      });
    if (kind === 'disappeared') throw Object.assign(new Error('gone'), { code: 'ENOENT' });
    const fd = nextFd++;
    files.set(fd, name);
    return fd;
  });
  vi.spyOn(fs, 'readSync').mockImplementation((fd, buffer, options) => {
    const name = files.get(fd);
    if (name === undefined) return read(fd, buffer, options);
    if (!Buffer.isBuffer(buffer)) throw new Error('Expected bounded byte buffer');
    const offset = options?.offset ?? 0;
    const length = options?.length ?? buffer.length - offset;
    const pid = name.split('/')[2];
    if (name.endsWith('/stat') && pid === '900002') cliStatReads++;
    const fields = Array(20).fill('0');
    Object.assign(fields, {
      0: 'S',
      1: '1',
      2: pid,
      3: pid,
      11: '17',
      12: '3',
      19: kind === 'reused' && pid === '900002' && cliStatReads > 1 ? '54321' : '12345',
    });
    const text = name.endsWith('/wchan')
      ? kind === 'wchan-overflow'
        ? 'x'.repeat(65)
        : 'do_wait\n'
      : kind === 'stat-overflow'
        ? 'x'.repeat(8193)
        : `${pid} (fixture) ${fields.join(' ')}`;
    const bytes = Buffer.from(text);
    const count = Math.min(bytes.length, length);
    buffer.set(bytes.subarray(0, count), offset);
    return count;
  });
  vi.spyOn(fs, 'closeSync').mockImplementation((fd) => {
    if (!files.delete(fd)) close(fd);
  });
  syncBuiltinESMExports();
  try {
    await withSandbox(
      async (sandbox) => {
        root = fs.realpathSync(sandbox.root);
        const result = runCli(sandbox, ['upgrade', '--json']).catch((error: unknown) => error);
        await vi.advanceTimersByTimeAsync(4999);
        expect(endpoints).toEqual([]);
        await vi.advanceTimersByTimeAsync(11);
        expect(await result).toMatchObject({
          message: 'CLI subprocess exceeded the 5000 millisecond test bound.',
        });
        const report = JSON.parse(String(f.diagnostics.mock.calls[0][1]));
        expect(report.deadlineMs).toBe(5000);
        expect(report.cleanup).toEqual({ closeObserved: true, remainingGroups: [], failed: false });
        const snapshot = report.preTermination;
        if (kind.startsWith('discovery-denied')) {
          expect(snapshot.observation).toBe('discovery unavailable');
          expect(endpoints).toEqual([]);
        } else {
          expect(snapshot).toMatchObject({ observation: 'snapshot', cliPid: 900002 });
          expect(snapshot.processes).toHaveLength(9);
          const cli = snapshot.processes[0];
          if (kind === 'selected' || kind === 'launcher') {
            expect(cli).toEqual({
              pid: 900002,
              observation: 'endpoint identity observed',
              ppid: '1',
              pgid: '900002',
              executable: kind === 'launcher' ? '/owned/runtime-caller-fixture' : '/owned/tmt',
              state: 'S',
              cpuTicks: { user: '17', system: '3' },
              wchan: 'do_wait',
            });
            expect(snapshot.processes[1].pgid).toBe('900010');
          } else {
            expect(cli).toEqual({
              pid: 900002,
              observation: kind.startsWith('recheck-denied')
                ? 'admission recheck unavailable'
                : kind === 'reused'
                  ? 'identity changed'
                  : kind === 'foreign'
                    ? 'no longer a sandbox resident'
                    : kind === 'exe-overflow'
                      ? 'endpoint facts unavailable'
                      : 'identity unavailable',
            });
          }
          expect(new Set(endpoints.map((name) => name.split('/')[2])).size).toBeLessThanOrEqual(9);
          expect(endpoints.some((name) => name.startsWith('/proc/900018/'))).toBe(false);
          if (kind.startsWith('recheck-denied') || kind === 'foreign')
            expect(endpoints.some((name) => name.startsWith('/proc/900002/'))).toBe(false);
        }
        expect(files.size).toBe(0);
        expect(JSON.stringify(report)).not.toContain('/foreign/root');
        expect(Buffer.byteLength(String(f.diagnostics.mock.calls[0][1]))).toBeLessThanOrEqual(
          16384
        );
        expect(f.signals.mock.calls.every(([pid]) => [-900001, -900002].includes(pid))).toBe(true);
        expect(f.spawn).toHaveBeenCalledOnce();
      },
      { TMT_TEST_CLI: JSON.stringify({ executable: process.execPath, args: [] }) }
    );
  } finally {
    Object.defineProperty(process, 'platform', platform);
    f.restore();
  }
});
