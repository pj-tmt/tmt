import { text } from '../src/strings.js';
import fs from 'node:fs';
import { expect, test } from '@playwright/test';
import { expectLive } from './harness/ask.js';
import { openColab, pairBrowser, startDoor } from './harness/browser.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

test.afterEach(disposeActiveWorlds);

/** These controls can reach only this stub, never an installed tmux executable. */
async function withStubTmux(
  run: (fixture: {
    executable: string;
    calls: () => Array<{ args: string[]; tmux?: string; pane?: string }>;
    socketReply: (value: string) => void;
  }) => Promise<void>,
) {
  const path = await import('node:path');
  const root = fs.mkdtempSync(path.join(fs.realpathSync('/tmp'), 'colab-tmux-stub-'));
  const log = path.join(root, 'calls.jsonl');
  const reply = path.join(root, 'socket-reply');
  const executable = path.join(root, 'tmux');
  fs.writeFileSync(
    executable,
    `#!${process.execPath}
const fs = require('node:fs');
const args = process.argv.slice(2);
fs.appendFileSync(${JSON.stringify(log)}, JSON.stringify({ args, tmux: process.env.TMUX, pane: process.env.TMUX_PANE }) + '\\n');
const query = args.at(-1);
if (query === '#{socket_path}') console.log(fs.existsSync(${JSON.stringify(reply)}) ? fs.readFileSync(${JSON.stringify(reply)}, 'utf8') : args[args.indexOf('-S') + 1]);
if (query === '#{pid}') console.log(process.pid);
if (query === '#{session_id}') console.log('$0');
if (query === '#{pane_id}') console.log('%0');
`,
    { mode: 0o700 },
  );
  try {
    await run({
      executable,
      socketReply: (value) => fs.writeFileSync(reply, value),
      calls: () =>
        fs.existsSync(log)
          ? fs
              .readFileSync(log, 'utf8')
              .trim()
              .split('\n')
              .map((line) => JSON.parse(line))
          : [],
    });
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
}

test('harness: private tmux commands and the native wrapper select only the asserted socket', async () => {
  const { execFileSync } = await import('node:child_process');
  const { AcceptanceWorld } = await import('./harness/world.js');
  await withStubTmux(async (stub) => {
    const binaries = { tmt: process.execPath, remote: process.execPath, colab: process.execPath };
    const world = new AcceptanceWorld(binaries, { tmuxExecutable: stub.executable });
    try {
      await world.start();
      const setupCalls = stub.calls().length;
      world.tmux(['list-sessions']);
      execFileSync('tmux', ['list-sessions'], { env: world.env('%0') });
      execFileSync('tmux', ['-u', '-S', world.socketPath, 'list-sessions'], {
        env: world.env('%0'),
      });
      const calls = stub.calls();
      const list = ['-f', '/dev/null', '-S', world.socketPath, 'list-sessions'];
      expect(calls.slice(setupCalls).map((call) => call.args)).toEqual([
        list,
        list,
        ['-u', ...list],
      ]);
      for (const call of calls) {
        const args = call.args[0] === '-u' ? call.args.slice(1) : call.args;
        expect(args.slice(0, 4)).toEqual(['-f', '/dev/null', '-S', world.socketPath]);
        expect(call.tmux).toBeUndefined();
        expect(call.pane).toBeUndefined();
      }
      expect(() => world.tmux(['-S', '/foreign', 'list-sessions'])).toThrow('not allowed');
      expect(() =>
        execFileSync('tmux', ['-S', '/foreign', 'list-sessions'], {
          env: world.env('%0'),
          stdio: 'pipe',
        }),
      ).toThrow();
      expect(stub.calls()).toEqual(calls);
    } finally {
      expect(await world.dispose()).toEqual([]);
    }
    expect(fs.existsSync(world.root)).toBe(false);
  });
});

test('harness: a changed private tmux socket is refused before the first server command', async () => {
  const path = await import('node:path');
  const { execFileSync } = await import('node:child_process');
  const { AcceptanceWorld } = await import('./harness/world.js');
  await withStubTmux(async (stub) => {
    const binaries = { tmt: process.execPath, remote: process.execPath, colab: process.execPath };
    const world = new AcceptanceWorld(binaries, { tmuxExecutable: stub.executable });
    fs.symlinkSync(path.join(world.root, 'foreign.sock'), world.socketPath);
    try {
      await expect(world.start()).rejects.toThrow('is a symlink');
      expect(() =>
        execFileSync('tmux', ['list-sessions'], {
          env: world.env(),
          stdio: 'pipe',
        }),
      ).toThrow();
      expect(stub.calls()).toEqual([]);
      expect(() => AcceptanceWorld.stopServer(world.root, world.socketPath)).toThrow(
        'is a symlink',
      );
      expect(() => AcceptanceWorld.stopServer(world.root, '/foreign.sock')).toThrow(
        'outside the test root',
      );
      expect(stub.calls()).toEqual([]);
    } finally {
      expect(await world.dispose()).toEqual([]);
    }
    expect(fs.existsSync(world.root)).toBe(false);
  });
});

test('harness: a refused reported tmux socket still stops the created private server', async () => {
  const { AcceptanceWorld } = await import('./harness/world.js');
  await withStubTmux(async (stub) => {
    const binaries = { tmt: process.execPath, remote: process.execPath, colab: process.execPath };
    // A refusal after starting must retain enough ownership to stop this world's server.
    const mismatch = new AcceptanceWorld(binaries, { tmuxExecutable: stub.executable });
    stub.socketReply('/foreign.sock');
    try {
      await expect(mismatch.start()).rejects.toThrow('different acceptance socket');
    } finally {
      const callsBeforeDispose = stub.calls().length;
      expect(await mismatch.dispose()).toEqual([]);
      expect(stub.calls()).toHaveLength(callsBeforeDispose + 1);
      expect(stub.calls().at(-1)?.args.slice(0, 4)).toEqual([
        '-f',
        '/dev/null',
        '-S',
        mismatch.socketPath,
      ]);
    }
    expect(fs.existsSync(mismatch.root)).toBe(false);
  });
});

// Proves the harness itself, independent of the Ask agent code: real binaries,
// real pairing of two browser devices, a recipient whose durable counter and
// real `tmt reply` carry the work, and a world that leaves nothing behind.
test('harness: two paired browsers, a counted recipient and a clean teardown', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world);
    const recipient = await world.startAgent('ask-recipient');
    const asker = await world.startAgent('ask-asker');
    const first = await pairBrowser(world, 'viewer-one');
    const second = await pairBrowser(world, 'viewer-two');
    expect(first.profile).not.toBe(second.profile);
    await openColab(door, first);
    await openColab(door, second);

    // Real core delivery to the recipient: one wake, one counted receipt, one real reply.
    const talk = await world.tmt(['talk', recipient.name, 'harness probe', '--json'], {
      pane: asker.pane,
    });
    expect(talk.code, talk.stdout + talk.stderr).toBe(0);
    const received = recipient.received();
    expect(received).toHaveLength(1);
    expect(received[0]).toMatchObject({ source: 'talk', message: 'harness probe' });
    const replied = recipient.rows().filter((row) => row.event === 'replied');
    expect(replied).toHaveLength(1);
    expect(recipient.rows().filter((row) => row.event === 'failure')).toEqual([]);
  });
});

// Sensitivity of the teardown guard: an untracked survivor naming the world's
// root must be reported, so a green cleanup cannot come from a blind check.
test('harness: the leak report detects a process the world did not stop', async () => {
  const { spawn } = await import('node:child_process');
  const { AcceptanceWorld } = await import('./harness/world.js');
  const world = new AcceptanceWorld();
  await world.start();
  const stray = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1e6)', world.root], {
    detached: true,
    stdio: 'ignore',
  });
  try {
    const leaks = await world.dispose();
    expect(leaks.filter((leak) => leak.startsWith('process remains'))).toHaveLength(1);
  } finally {
    process.kill(stray.pid ?? 0, 'SIGKILL');
  }
});

// A survivor is reported with who owns it, so the first failing run names the process to fix
// (#4296): its parent, process group and age, and the parent's command when it is still running.
test('harness: a reported survivor names its parent, process group and age', async () => {
  const { spawn } = await import('node:child_process');
  const { AcceptanceWorld } = await import('./harness/world.js');
  const world = new AcceptanceWorld();
  await world.start();
  const stray = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1e6)', world.root], {
    detached: true,
    stdio: 'ignore',
  });
  try {
    const leaks = await world.dispose();
    const [report] = leaks.filter((leak) => leak.startsWith('process remains'));
    expect(report).toContain(`process remains: ${stray.pid} `);
    // It was started detached by this process: its parent is this process and its group is its own.
    expect(report).toMatch(
      new RegExp(`\\(ppid ${process.pid}, pgid ${stray.pid}, age [\\d:.-]+, parent `),
    );
    expect(report).toContain(world.root);
  } finally {
    process.kill(stray.pid ?? 0, 'SIGKILL');
  }
});

// #2520: a world is an agent run on a private PATH, so no command may reach the host browser.
// `colab open` and `page create` without --no-open/--json are the paths that once did.
test('harness: colab open and page create never invoke a browser opener in a world', async () => {
  await withWorld(async (world) => {
    await startDoor(world);
    world.linkExtensions();
    const created = await world.tmt(
      ['colab', 'page', 'create', '--title', 'Opener', '--layout', 'column', '--file', '-'],
      {
        stdin: '<p>No browser.</p>',
      },
    );
    expect(created.code, created.stdout + created.stderr).toBe(0);
    const opened = await world.tmt(['colab', 'open']);
    expect(opened.code, opened.stdout + opened.stderr).toBe(0);
    expect(opened.stdout).toContain('http');
    expect(fs.existsSync(world.openerLog), world.openerLog).toBe(false);
  });
});

// Sensitivity of that guard: a call to a stand-in opener is reported at teardown, so a quiet
// log cannot come from a stand-in nothing would ever write to.
test('harness: the leak report names a host browser opener call', async () => {
  const { execFileSync } = await import('node:child_process');
  const { AcceptanceWorld } = await import('./harness/world.js');
  const world = new AcceptanceWorld();
  await world.start();
  execFileSync('open', ['http://127.0.0.1:1/probe'], { env: world.env() });
  const leaks = await world.dispose();
  expect(leaks.filter((leak) => leak.startsWith('host browser opener called'))).toHaveLength(1);
  expect(leaks.join('\n')).toContain('open http://127.0.0.1:1/probe');
});

// An assertion failing mid-scenario must still stop every child, the tmux server
// and the browsers, and must surface the scenario's own error, not a cleanup one.
test('harness: an injected scenario failure still tears the world down', async () => {
  const { withWorld } = await import('./harness/with-world.js');
  const { openColab, pairBrowser, startDoor } = await import('./harness/browser.js');
  let root = '';
  await expect(
    withWorld(async (world) => {
      root = world.root;
      const door = await startDoor(world);
      await openColab(door, await pairBrowser(world, 'viewer-failing'));
      throw new Error('injected scenario failure');
    }),
  ).rejects.toThrow('injected scenario failure');
  expect(fs.existsSync(root)).toBe(false);
  const { execFileSync } = await import('node:child_process');
  expect(execFileSync('/bin/ps', ['-axo', 'command='], { encoding: 'utf8' })).not.toContain(root);
});

// A test that times out is abandoned, not cancelled (#2327): its afterEach disposes the world
// while the scenario may still be running. These cases drive that path without waiting for a
// real Playwright timeout.
const idle = ['-e', 'setInterval(() => {}, 1e6)'];
const processes = async () =>
  (await import('node:child_process')).execFileSync('/bin/ps', ['-axo', 'command='], {
    encoding: 'utf8',
  });

test('harness: the timeout cleanup reports what a timed-out world leaked', async () => {
  const { spawn } = await import('node:child_process');
  let release = () => {};
  const parked = new Promise<void>((resolve) => (release = resolve));
  let root = '';
  let stray: ReturnType<typeof spawn> | undefined;
  const scenario = withWorld(async (world) => {
    root = world.root;
    stray = spawn(process.execPath, [...idle, world.root], { detached: true, stdio: 'ignore' });
    await parked;
  });
  await expect.poll(() => root).not.toBe('');
  try {
    // What the afterEach of a timed-out test does: dispose and surface the leak report.
    await expect(disposeActiveWorlds()).rejects.toThrow(/leaked[\s\S]*process remains/);
  } finally {
    process.kill(stray?.pid ?? 0, 'SIGKILL');
    release();
    await scenario.catch(() => {});
  }
  expect(fs.existsSync(root)).toBe(false);
});

test('harness: a scenario that outlives its dispose cannot add a child, a tmux call or a closer', async () => {
  const { AcceptanceWorld } = await import('./harness/world.js');
  const world = new AcceptanceWorld();
  await world.start();
  const root = world.root;
  expect(await world.dispose()).toEqual([]);
  expect(() => world.spawn('late', process.execPath, idle)).toThrow(/disposed/);
  expect(() => world.tmux(['list-sessions'])).toThrow(/disposed/);
  expect(() => world.onDispose(async () => {})).toThrow(/disposed/);
  await expect(world.tmt(['--version'])).rejects.toThrow(/disposed/);
  expect(fs.existsSync(root)).toBe(false);
  expect(await processes()).not.toContain(root);
});

test('harness: a timeout in the middle of spawning leaves no child behind', async () => {
  let root = '';
  let spawned = () => {};
  const first = new Promise<void>((resolve) => (spawned = resolve));
  const scenario = withWorld(async (world) => {
    root = world.root;
    world.spawn('first', process.execPath, idle);
    spawned();
    // The abandoned scenario is still working when the cleanup runs.
    await new Promise((resolve) => setTimeout(resolve, 300));
    world.spawn('second', process.execPath, idle);
  });
  await first;
  await disposeActiveWorlds();
  await expect(scenario).rejects.toThrow(/disposed/);
  expect(fs.existsSync(root)).toBe(false);
  expect(await processes()).not.toContain(root);
});

test('harness: a closer that never finishes is bounded and reported, and the rest is still torn down', async () => {
  const { AcceptanceWorld } = await import('./harness/world.js');
  const world = new AcceptanceWorld(undefined, { closerBoundMs: 300 });
  await world.start();
  world.spawn('kept', process.execPath, idle);
  world.onDispose((stage) => {
    stage('closing the browser context');
    return new Promise<void>(() => {});
  });
  const started = Date.now();
  const leaks = await world.dispose();
  expect(Date.now() - started).toBeLessThan(15_000);
  expect(leaks).toEqual([
    expect.stringContaining('did not finish within 300 ms (in closing the browser context)'),
  ]);
  expect(fs.existsSync(world.root)).toBe(false);
  const listing = await processes();
  expect(listing).not.toContain(world.root);
  expect(listing).not.toContain(` -S ${world.socketPath}`);
});

// A provider wrapper must finish and be reaped by tmux before the server goes away.
test('harness: teardown waits for the recipient and its tmt run pane before stopping tmux', async () => {
  const { alive, until } = await import('./harness/process.js');
  await withWorld(async (world) => {
    // Bash retains a shell around redirected commands unless launch uses exec.
    world.tmux(['set-option', '-g', 'default-shell', '/bin/bash']);
    const recipient = await world.startAgent('claude-teardown', {
      driver: 'claude',
      stopGated: true,
    });
    const wrapperPid = Number(
      world.tmux(['display-message', '-p', '-t', recipient.pane, '#{pane_pid}']).trim(),
    );
    const providerPid = Number(recipient.rows().find((row) => row.event === 'ready')?.pid);
    expect(wrapperPid).toBeGreaterThan(0);
    expect(providerPid).toBeGreaterThan(0);
    expect(wrapperPid).not.toBe(providerPid);
    let finished = false;
    const disposal = world.dispose().then((leaks) => {
      finished = true;
      return leaks;
    });
    try {
      await until(
        () => recipient.rows().some((row) => row.event === 'stopping'),
        'provider stop boundary',
      );
      expect(finished).toBe(false);
      expect(alive(wrapperPid)).toBe(true);
      expect(alive(providerPid)).toBe(true);
      // Releasing the provider lets the real wrapper reap it and tmux reap the wrapper.
      fs.writeFileSync(`${recipient.gate}/stop.release`, '');
      expect(await disposal).toEqual([]);
      expect(alive(providerPid)).toBe(false);
      expect(alive(wrapperPid)).toBe(false);
      expect(fs.existsSync(world.root)).toBe(false);
    } finally {
      if (fs.existsSync(recipient.gate)) fs.writeFileSync(`${recipient.gate}/stop.release`, '');
      await disposal;
    }
  });
});

// A slow pane remains a failure even though the rest of cleanup still runs.
test('harness: an unfinished recipient pane is reported without relaxing the leak guard', async () => {
  await expect(
    withWorld(async (world) => {
      world.tmux(['set-option', '-g', 'default-shell', '/bin/bash']);
      const recipient = await world.startAgent('claude-unfinished', {
        driver: 'claude',
        stopGated: true,
      });
      const leaks = await world.dispose();
      expect(leaks).toContain(
        `agent pane ${recipient.pane}: Timed out: agent pane ${recipient.pane} completion`,
      );
      expect(fs.existsSync(world.root)).toBe(false);
    }),
  ).rejects.toThrow(/agent pane %\d+: Timed out: agent pane %\d+ completion/);
});

// Re-entrant disposal must join the first owner, including its leak report.
test('harness: concurrent disposal joins cleanup and preserves its report', async () => {
  const { AcceptanceWorld } = await import('./harness/world.js');
  const world = new AcceptanceWorld();
  let release = () => {};
  const gate = new Promise<void>((resolve) => (release = resolve));
  world.onDispose(async () => {
    await gate;
    throw new Error('injected closer failure');
  });
  const first = world.dispose();
  const second = world.dispose();
  expect(second).toBe(first);
  let finished = false;
  void second.then(() => (finished = true));
  await Promise.resolve();
  expect(finished).toBe(false);
  expect(fs.existsSync(world.root)).toBe(true);
  release();
  const report = await first;
  expect(await second).toBe(report);
  expect(report).toEqual(['closer: injected closer failure']);
  expect(fs.existsSync(world.root)).toBe(false);
});

test('harness: scenario and cleanup failures are both retained', async () => {
  let failure: unknown;
  try {
    await withWorld(async (world) => {
      world.onDispose(async () => {
        throw new Error('injected cleanup failure');
      });
      throw new Error('injected scenario failure');
    });
  } catch (error) {
    failure = error;
  }
  expect(failure).toBeInstanceOf(AggregateError);
  const errors = (failure as AggregateError).errors as Error[];
  expect(errors.map((error) => error.message).join('\n')).toContain('injected scenario failure');
  expect(errors.map((error) => error.message).join('\n')).toContain('injected cleanup failure');
});

interface WorldLifecycle {
  root: string;
  case: string;
  stage: string;
  leaks: number;
  pids: number[];
  servePids: number[];
  tmuxPid: number;
  tmuxSocket: string;
}

/** Child fixture records are emitted only by the harness's spawn/start owners. */
async function cleanupChildWorlds(records: WorldLifecycle[]): Promise<void> {
  const { AcceptanceWorld } = await import('./harness/world.js');
  const { alive, until } = await import('./harness/process.js');
  const { signalServeLockOwner } = await import('./harness/serve-lock.js');
  for (const root of new Set(records.map((record) => record.root))) {
    // A teardown snapshot or a look-alike executable is never authority to kill.
    const groups = new Set(
      records
        .filter((record) => record.root === root && record.stage === 'spawned')
        .flatMap((record) => record.pids),
    );
    for (const group of groups) {
      if (alive(-group)) process.kill(-group, 'SIGKILL');
      await until(() => !alive(-group), 'recorded child group exit', 3_000);
    }
    const serves = new Set(
      records
        .filter((record) => record.root === root && record.stage === 'serve-owned')
        .flatMap((record) => record.servePids),
    );
    for (const pid of serves) {
      if (signalServeLockOwner(`${root}/state/colab/serve.lock`, pid, 'SIGKILL'))
        await until(() => !alive(pid), 'recorded private serve exit', 3_000);
    }
    const started = records.find((record) => record.root === root && record.stage === 'started');
    if (started && alive(started.tmuxPid)) {
      AcceptanceWorld.stopServer(root, started.tmuxSocket);
      await until(() => !alive(started.tmuxPid), 'recorded private tmux server exit', 3_000);
    }
    fs.rmSync(root, { recursive: true, force: true });
  }
}

test('harness: emergency cleanup leaves an unrecorded look-alike process alive', async () => {
  const { spawn } = await import('node:child_process');
  const { AcceptanceWorld } = await import('./harness/world.js');
  const { alive } = await import('./harness/process.js');
  const world = new AcceptanceWorld();
  const own = world.spawn('owned-idle', process.execPath, idle);
  const unrelated = spawn(process.execPath, idle, { detached: true, stdio: 'ignore' });
  const identity = {
    root: world.root,
    case: 'ownership control',
    servePids: [],
    leaks: 0,
    tmuxPid: 0,
    tmuxSocket: '',
  };
  try {
    await cleanupChildWorlds([
      { ...identity, stage: 'spawned', pids: [own.pid] },
      { ...identity, stage: 'begin', pids: [own.pid, unrelated.pid!] },
    ]);
    expect(alive(own.pid)).toBe(false);
    expect(alive(unrelated.pid!)).toBe(true);
  } finally {
    // The control itself owns this handle; it is not part of world emergency cleanup.
    if (unrelated.pid && alive(-unrelated.pid)) process.kill(-unrelated.pid, 'SIGKILL');
    await world.dispose();
  }
});

// Actual Playwright cancellation, including timeout while withWorld is already disposing.
test('harness: child timeouts join teardown and leave no worlds', async () => {
  const path = await import('node:path');
  const { fileURLToPath } = await import('node:url');
  const { execFile } = await import('node:child_process');
  const { promisify } = await import('node:util');
  const { alive } = await import('./harness/process.js');
  await withWorld(async (outsideWorld) => {
    const directory = fs.mkdtempSync(path.join(fs.realpathSync('/tmp'), 'colab-timeout-proof-'));
    const outsider = await startDoor(outsideWorld);
    const harness = fileURLToPath(new URL('./harness/', import.meta.url));
    const config = path.join(directory, 'playwright.config.cjs');
    fs.writeFileSync(
      config,
      `module.exports = ${JSON.stringify({
        testDir: harness,
        testMatch: 'timeout.fixture.ts',
        workers: 1,
        retries: 0,
        timeout: 30_000,
        outputDir: path.join(directory, 'results'),
        reporter: [['line'], ['json', { outputFile: path.join(directory, 'report.json') }]],
      })}`,
    );
    // Reporter environment overrides the config's outputFile. Keep the child's
    // report private and preserve only the shared browser installation location.
    const parentReport = path.join(directory, 'parent-report.json');
    fs.writeFileSync(parentReport, 'parent reporter sentinel');
    const inherited = {
      ...process.env,
      PLAYWRIGHT_JSON_OUTPUT_FILE: parentReport,
      PLAYWRIGHT_JSON_OUTPUT_DIR: directory,
      PLAYWRIGHT_JSON_OUTPUT_NAME: 'parent-report.json',
    };
    const childEnvironment = Object.fromEntries(
      Object.entries(inherited).filter(
        ([name]) => !name.startsWith('PLAYWRIGHT_') || name === 'PLAYWRIGHT_BROWSERS_PATH',
      ),
    );
    let records: WorldLifecycle[] = [];
    try {
      let output = '';
      let code: unknown;
      try {
        await promisify(execFile)(
          process.execPath,
          [
            fileURLToPath(new URL('../node_modules/@playwright/test/cli.js', import.meta.url)),
            'test',
            '--config',
            config,
          ],
          {
            env: {
              ...childEnvironment,
              PLAYWRIGHT_JSON_OUTPUT_FILE: path.join(directory, 'report.json'),
              TMT_AGENT: '1',
              BROWSER: '/usr/bin/true',
              TMT_ACCEPTANCE_KEEP: '0',
            },
            timeout: 60_000,
            maxBuffer: 2_000_000,
          },
        );
      } catch (error) {
        const result = error as Error & { code: unknown; stdout: string; stderr: string };
        code = result.code;
        output = result.stdout + result.stderr;
      }
      records = output.split('\n').flatMap((line) => {
        const match = line.match(/colab-world (\{.*\})/);
        return match ? [JSON.parse(match[1]) as WorldLifecycle] : [];
      });
      expect(code, output).toBe(1);
      expect(output).toContain('2 failed');
      expect(fs.readFileSync(parentReport, 'utf8')).toBe('parent reporter sentinel');
      const report = JSON.parse(fs.readFileSync(path.join(directory, 'report.json'), 'utf8')) as {
        suites: Array<{
          specs: Array<{
            tests: Array<{
              results: Array<{
                status: string;
                attachments: Array<{ name: string; contentType: string; body?: string }>;
              }>;
            }>;
          }>;
        }>;
      };
      expect(report.suites[0].specs.map((spec) => spec.tests[0].results[0].status)).toEqual([
        'timedOut',
        'timedOut',
      ]);
      for (const spec of report.suites[0].specs) {
        const diagnostics = spec.tests[0].results[0].attachments.filter(
          (attachment) => attachment.name === 'colab-own-publication',
        );
        expect(diagnostics).toHaveLength(1);
        expect(diagnostics[0].contentType).toBe('application/json');
        expect(JSON.parse(Buffer.from(diagnostics[0].body!, 'base64').toString('utf8'))).toEqual({
          root: 'messages',
          kind: 'ask-state',
          streamId: '30000000-0000-4000-8000-000000000001',
          submittedSeq: '7',
          admittedSeq: '4',
          leaderReceipt: true,
        });
      }
      await test.info().attach('child-timeout-lifecycle', {
        body: JSON.stringify(records),
        contentType: 'application/json',
      });
      const completed = records.filter((record) => record.stage === 'complete');
      expect(completed, output).toHaveLength(2);
      for (const record of completed) {
        expect(record.case).toMatch(/^child timeout:/);
        expect(record.leaks).toBe(0);
        expect(fs.existsSync(record.root)).toBe(false);
        expect(record.pids.every((pid) => !alive(pid) && !alive(-pid))).toBe(true);
        expect(alive(record.tmuxPid)).toBe(false);
        expect(record.servePids.every((pid) => !alive(pid))).toBe(true);
      }
      expect(alive(outsider.colab.pid)).toBe(true);
      // Lifecycle output uses only identities, never the paired URL, seed or arguments.
      expect(JSON.stringify(records)).not.toMatch(/https?:|#|seed|capability|argv/);
      console.log(`Child timeout qualification: ${completed.length} worlds cleaned`);
    } finally {
      // Failure/sensitivity cleanup is restricted to the child-recorded owned identities.
      // Assertions above still fail: emergency cleanup never qualifies a leaking run.
      await cleanupChildWorlds(records);
      expect(alive(outsider.colab.pid)).toBe(true);
      fs.rmSync(directory, { recursive: true, force: true });
    }
  });
});

// The soak tests' Live check names the state a page stayed in. The attachment is a closed set,
// read from the parent's status label only; the CI report keeps nothing else (#3083).
test('harness: a failed Live check attaches the closed header state and keeps its failure', async ({
  page,
}) => {
  const attached = () =>
    test
      .info()
      .attachments.filter((attachment) => attachment.name === 'colab-live-state')
      .map((attachment) => JSON.parse(attachment.body!.toString('utf8')));
  const header = (label: string) =>
    `<span class="status page-connection"><span class="status-label">${label}</span></span>`;
  await page.setContent(`${header('Live')}<iframe></iframe>`);
  await expectLive(page, 300);
  expect(attached()).toEqual([]);
  await page.setContent(`${header('Opening page…')}<iframe></iframe>`);
  await expect(expectLive(page, 300)).rejects.toThrow(/Opening page…/);
  await page.setContent(`${header('Preview stopped')}`);
  await expect(expectLive(page, 300)).rejects.toThrow();
  await page.setContent('<main>Not a page</main>');
  await expect(expectLive(page, 300)).rejects.toThrow();
  await page.setContent(`${header('Secret label')}`);
  await expect(expectLive(page, 300)).rejects.toThrow();
  expect(attached()).toEqual([
    { state: 'opening', frame: true, visibility: 'visible', ended: 'none' },
    { state: 'stopped', frame: false, visibility: 'visible', ended: 'none' },
    { state: 'none', frame: false, visibility: 'visible', ended: 'none' },
    { state: 'other', frame: false, visibility: 'visible', ended: 'none' },
  ]);
});

// #3083: an ended connection stops the preview without a renderer notice; the diagnostic names its
// cause from the closed reference label, and nothing else the page shows.
test('harness: a stopped preview names the closed cause its terminal notice shows', async ({
  page,
}) => {
  const attached = () =>
    test
      .info()
      .attachments.filter((attachment) => attachment.name === 'colab-live-state')
      .map((attachment) => JSON.parse(attachment.body!.toString('utf8')).ended);
  const ended = (reference: string) =>
    `<span class="status page-connection"><span class="status-label">${text.blocked}</span></span>` +
    `<p>Authored sentence the diagnostic must not copy.</p>` +
    `<small><code data-failure-reference>${reference}</code></small>`;
  const causes = Object.entries(text.failureReference);
  expect(causes.length).toBeGreaterThan(0);
  for (const [, label] of causes) {
    await page.setContent(ended(label));
    await expect(expectLive(page, 300)).rejects.toThrow();
  }
  await page.setContent(ended('Authored label outside the set'));
  await expect(expectLive(page, 300)).rejects.toThrow();
  await page.setContent(
    `<span class="status page-connection"><span class="status-label">${text.blocked}</span></span>`,
  );
  await expect(expectLive(page, 300)).rejects.toThrow();
  expect(attached()).toEqual([...causes.map(([cause]) => cause), 'other', 'none']);
  expect(JSON.stringify(attached())).not.toContain('Authored');
});

// A stopped preview names its renderer notice without inferring an unobserved cause.
test('harness: a stopped preview names its renderer notice without inferring the cause', async ({
  page,
}) => {
  const stopped = (notice: string) =>
    `<span class="status page-connection"><span class="status-label">Preview stopped</span></span><p>${notice}</p>`;
  await page.setContent(stopped(text.failed));
  await expect(expectLive(page, 300)).rejects.toThrow(/ · renderer stop failed$/);
  await page.setContent(stopped(text.navigation));
  await expect(expectLive(page, 300)).rejects.toThrow(/ · renderer stop navigation$/);
  await page.setContent(stopped(''));
  await expect(expectLive(page, 300)).rejects.toThrow(/ · renderer stop none$/);
});
