import { spawn, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { parseWholeStdout, runCli, withSandbox, type Sandbox } from '../support/cli-process.js';
import { installTmuxTripwire } from './tmux-tripwire.js';

// A real Herdr server (#479 H3). CI does not ship Herdr yet (H4 adds it to
// the E2E image), so this runs only when TMT_TEST_HERDR names a pinned,
// digest-checked `herdr` binary. The server gets its own short socket and
// HOME; teardown fails the test if any of its processes remain.
const HERDR = process.env.TMT_TEST_HERDR;

interface Herdr {
  readonly root: string;
  readonly env: NodeJS.ProcessEnv;
  json(args: readonly string[]): Record<string, any>;
  /** Run a shell command in a pane; returns its stdout, stderr and status. */
  inPane(
    pane: string,
    command: string
  ): Promise<{ stdout: string; stderr: string; status: number }>;
  restart(): Promise<void>;
  stop(): void;
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function until(check: () => boolean, what: string, ms = 10_000): Promise<void> {
  const deadline = Date.now() + ms;
  while (!check()) {
    if (Date.now() > deadline) throw new Error(`Timed out waiting for ${what}`);
    await sleep(50);
  }
}

async function startHerdr(sandbox: Sandbox): Promise<Herdr> {
  // macOS limits socket paths to 104 bytes, and the sandbox root is too deep
  // for one; only the socket lives in this short directory.
  const root = mkdtempSync('/tmp/tmt-herdr-');
  const socket = path.join(root, 's.sock');
  // Herdr's configuration lives in the sandbox, whichever base it reads.
  for (const base of [path.join(sandbox.home, '.config'), sandbox.xdgConfigHome]) {
    mkdirSync(path.join(base, 'herdr'), { recursive: true });
    writeFileSync(
      path.join(base, 'herdr', 'config.toml'),
      '[update]\nversion_check = false\nmanifest_check = false\n'
    );
  }
  // Panes inherit the server's environment, so commands in a pane and
  // outside use the same sandbox and resolve the same server by its socket.
  for (const key of ['TMUX', 'TMUX_PANE', 'TMUX_TEAM_HOME', 'HERDR_ENV', 'HERDR_PANE_ID']) {
    delete sandbox.env[key];
  }
  sandbox.env.PATH = `${path.dirname(HERDR!)}${path.delimiter}${sandbox.env.PATH}`;
  sandbox.env.HERDR_SOCKET_PATH = socket;
  sandbox.env.SHELL = '/bin/sh';
  const herdrEnv = { ...sandbox.env };
  const run = (args: readonly string[]) =>
    spawnSync(HERDR!, args, { env: herdrEnv, encoding: 'utf8', timeout: 10_000 });
  const running = () => run(['status', 'server']).stdout?.includes('status: running') ?? false;
  const start = async () => {
    const server = spawn(HERDR!, ['server'], { env: herdrEnv, detached: true, stdio: 'ignore' });
    server.unref();
    await until(running, 'the Herdr server');
  };
  await start();
  let sequence = 0;
  return {
    root,
    env: herdrEnv,
    json(args) {
      const result = run(args);
      expect(result.status, `${args.join(' ')}: ${result.stderr}`).toBe(0);
      return JSON.parse(result.stdout);
    },
    async inPane(pane, command) {
      const tag = path.join(root, `run-${(sequence += 1)}`);
      const result = run([
        'pane',
        'run',
        pane,
        `${command} > '${tag}.out' 2> '${tag}.err'; echo $? > '${tag}.status'`,
      ]);
      expect(result.status, result.stderr).toBe(0);
      await until(
        () => existsSync(`${tag}.status`) && readFileSync(`${tag}.status`, 'utf8').endsWith('\n'),
        command
      );
      return {
        stdout: readFileSync(`${tag}.out`, 'utf8'),
        stderr: readFileSync(`${tag}.err`, 'utf8'),
        status: Number(readFileSync(`${tag}.status`, 'utf8').trim()),
      };
    },
    async restart() {
      run(['server', 'stop']);
      await until(() => !running(), 'the Herdr server to stop');
      await start();
    },
    stop() {
      run(['server', 'stop']);
    },
  };
}

async function stopHerdr(herdr: Herdr): Promise<void> {
  herdr.stop();
  const leftovers = () =>
    (spawnSync('ps', ['-A', '-o', 'args='], { encoding: 'utf8' }).stdout ?? '')
      .split('\n')
      .filter((line) => line.includes(`${HERDR} server`));
  try {
    await until(() => leftovers().length === 0, 'Herdr processes to exit');
  } finally {
    rmSync(herdr.root, { recursive: true, force: true });
  }
}

function tmt(sandbox: Sandbox): string {
  return [sandbox.cli.executable, ...sandbox.cli.args].map((word) => `'${word}'`).join(' ');
}

describe.skipIf(!HERDR)('Herdr host (real server)', () => {
  it('binds, lists, renames, unbinds and removes identities, and loses them with the server', async () => {
    await withSandbox(async (sandbox) => {
      const tmuxLog = installTmuxTripwire(sandbox);
      const herdr = await startHerdr(sandbox);
      try {
        herdr.json(['workspace', 'create', '--cwd', sandbox.cwd]);
        herdr.json(['workspace', 'create', '--cwd', sandbox.cwd]);
        const panes = herdr.json(['pane', 'list']).result.panes as { pane_id: string }[];
        expect(panes.map((pane) => pane.pane_id)).toEqual(['w1:p1', 'w2:p1']);

        // Caller-scoped commands inside a Herdr pane.
        const named = await herdr.inPane('w1:p1', `${tmt(sandbox)} name worker`);
        expect(named).toMatchObject({ status: 0 });
        expect(named.stdout).toContain("Bound temporary identity 'worker' on pane w1:p1");
        const whoami = await herdr.inPane('w1:p1', `${tmt(sandbox)} whoami`);
        expect(whoami.stdout).toBe('worker (temporary) on pane w1:p1\n');

        // Outside any pane the caller's host is tmux; the Herdr binding is
        // still probed on Herdr, and tmux is never run.
        const listed = await runCli(sandbox, ['ls', '--json']);
        expect(listed.status).toBe(0);
        expect(parseWholeStdout(listed)).toMatchObject({
          identities: [
            {
              name: 'worker',
              presence: 'active',
              target: 'w1:p1',
              address: 'herdr:w1:p1',
              driver: 'herdr',
            },
          ],
        });
        const tokens = herdr.json(['pane', 'get', 'w1:p1']).result.pane.tokens;
        expect(tokens).toMatchObject({ tmt_name: 'worker', tmt_cname: 'worker' });

        // Explicit targets, rename (the marker follows), unbind and rm.
        const added = await runCli(sandbox, ['add', '--save', 'w2:p1', 'reviewer']);
        expect(added.status).toBe(0);
        expect(added.stdout).toContain("Bound saved identity 'reviewer' on pane w2:p1");
        const missing = await runCli(sandbox, ['add', 'w9:p9', 'nobody']);
        expect(missing.status).toBe(3);
        expect(missing.stderr).toContain("Pane target 'w9:p9' was not found");
        // A name longer than one 80-character token spans continuation keys;
        // Herdr merges reports per key, so a shorter name must clear them.
        const long = 'w'.repeat(200);
        expect((await runCli(sandbox, ['rename', 'worker', long])).status).toBe(0);
        expect(herdr.json(['pane', 'get', 'w1:p1']).result.pane.tokens).toMatchObject({
          tmt_name_3: 'w'.repeat(40),
        });
        expect((await runCli(sandbox, ['rename', long, 'lead'])).status).toBe(0);
        const renamed = herdr.json(['pane', 'get', 'w1:p1']).result.pane.tokens;
        expect(renamed).toMatchObject({ tmt_name: 'lead', tmt_cname: 'lead' });
        expect(Object.keys(renamed).filter((key) => /_[234]$/.test(key))).toEqual([]);
        expect(parseWholeStdout(await runCli(sandbox, ['ls', '--json']))).toMatchObject({
          identities: [
            { name: 'lead', presence: 'active' },
            { name: 'reviewer', presence: 'active' },
          ],
        });
        const unbound = await herdr.inPane('w1:p1', `${tmt(sandbox)} unbind`);
        expect(unbound.stdout).toContain("Unbound 'lead' from pane w1:p1; identity retired");
        expect(herdr.json(['pane', 'get', 'w1:p1']).result.pane.tokens ?? {}).toEqual({});

        // A server restart reuses pane IDs but not terminals: the saved
        // identity goes offline and is never bound to the new terminal.
        await herdr.restart();
        const after = await runCli(sandbox, ['ls', '--json']);
        expect(parseWholeStdout(after)).toMatchObject({
          identities: [{ name: 'reviewer', presence: 'offline', pane: null }],
        });
        expect((await runCli(sandbox, ['rm', '--force', 'reviewer'])).status).toBe(0);
        expect(existsSync(tmuxLog)).toBe(false);
      } finally {
        await stopHerdr(herdr);
      }
    });
  }, 60_000);
});
