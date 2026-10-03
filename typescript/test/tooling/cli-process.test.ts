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
