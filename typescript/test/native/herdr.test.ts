import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { spawn, spawnSync } from 'node:child_process';
import path from 'node:path';
import Database from 'better-sqlite3';
import { describe, expect, it } from 'vite-plus/test';
import { parseWholeStdout, runCli, withSandbox, type Sandbox } from '../support/cli-process.js';
import { installTmuxTripwire } from './tmux-tripwire.js';

// A real Herdr server, reached through the approved `tmt-driver-herdr`
// (#479, #1082). CI does not ship Herdr yet, so this runs only when
// TMT_TEST_HERDR names a pinned, digest-checked `herdr` binary. The server
// gets its own short socket and HOME; teardown fails the test if any of its
// processes remain.
const HERDR = process.env.TMT_TEST_HERDR;
// The build before #1082, whose Herdr host was built in: an absolute `tmt`
// built from that revision in its own worktree and target directory.
const PREVIOUS_TMT = process.env.TMT_TEST_PREVIOUS_TMT;
const HERDR_HINT = 'Herdr panes need the Herdr driver: tmt driver install herdr';

interface Herdr {
  readonly root: string;
  readonly env: NodeJS.ProcessEnv;
  json(args: readonly string[]): Record<string, any>;
  /** Run a Herdr command that prints no document. */
  command(args: readonly string[]): void;
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
  for (const key of ['TMUX', 'TMUX_PANE', 'TMT_HOME', 'HERDR_ENV', 'HERDR_PANE_ID']) {
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
    command(args) {
      const result = run(args);
      expect(result.status, `${args.join(' ')}: ${result.stderr}`).toBe(0);
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

/** Approves the Herdr driver built beside the tested `tmt`, by its path. */
async function approveHerdrDriver(sandbox: Sandbox): Promise<void> {
  const driver = path.join(path.dirname(sandbox.cli.executable), 'tmt-driver-herdr');
  const approved = await runCli(sandbox, ['driver', 'install', driver, '--yes']);
  expect(approved.status, approved.stderr).toBe(0);
}

/** The stored bindings and host servers, as the database holds them. */
function storedEndpoints(sandbox: Sandbox): { bindings: unknown[]; servers: unknown[] } {
  const database = new Database(sandbox.database, { readonly: true });
  try {
    return {
      bindings: database
        .prepare('SELECT id, transport, server_id, pane_id FROM bindings ORDER BY id')
        .all(),
      servers: database
        .prepare('SELECT server_id, host, socket_path FROM host_servers ORDER BY server_id')
        .all(),
    };
  } finally {
    database.close();
  }
}

describe.skipIf(!HERDR)('Herdr host (real server)', () => {
  it('binds, lists, renames, unbinds and removes identities, and loses them with the server', async () => {
    await withSandbox(async (sandbox) => {
      const tmuxLog = installTmuxTripwire(sandbox);
      const herdr = await startHerdr(sandbox);
      try {
        await approveHerdrDriver(sandbox);
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

describe.skipIf(!HERDR || !PREVIOUS_TMT)(
  'Herdr binding from the built-in host (real server)',
  () => {
    it('waits for the driver, then carries over with its server ID', async () => {
      await withSandbox(async (sandbox) => {
        const tmuxLog = installTmuxTripwire(sandbox);
        // Panes inherit the server's PATH: `claude` there is a stand-in agent
        // that records one prompt (Herdr pastes it bracketed) and exits.
        const agents = path.join(sandbox.root, 'agents');
        const received = path.join(sandbox.root, 'agent-received.log');
        mkdirSync(agents);
        writeExecutable(
          path.join(agents, 'claude'),
          [
            '#!/bin/sh',
            "printf '\\033[?2004h'",
            'stty -echo',
            'while IFS= read -r line; do',
            `  printf '%s\\n' "$line" >> '${received}'`,
            "  case $line in *'[201~'*) break ;; esac",
            'done',
            // Restore the terminal for the shell before handing it back.
            'stty echo',
            "printf '\\033[?2004l'",
            `echo exited >> '${received}'`,
            '',
          ].join('\n'),
          0o755
        );
        sandbox.env.PATH = `${agents}${path.delimiter}${sandbox.env.PATH}`;
        const herdr = await startHerdr(sandbox);
        const previous = { ...sandbox, cli: { executable: PREVIOUS_TMT!, args: [] } };
        try {
          herdr.json(['workspace', 'create', '--cwd', sandbox.cwd]);
          // The previous build binds through its built-in Herdr host.
          const named = await herdr.inPane('w1:p1', `${tmt(previous)} name --save worker`);
          expect(named.status, named.stderr).toBe(0);
          const before = storedEndpoints(sandbox);
          expect(before.bindings).toMatchObject([
            { transport: 'herdr', pane_id: expect.any(String) },
          ]);
          expect(before.servers).toHaveLength(1);
          const marker = herdr.json(['pane', 'get', 'w1:p1']).result.pane.tokens;
          expect(marker).toMatchObject({ tmt_name: 'worker' });

          // This build without an approved driver: the binding waits, nothing is
          // deleted, and a command in the pane says how to approve one.
          const waiting = await runCli(sandbox, ['ls', '--json']);
          expect(waiting.status, waiting.stderr).toBe(0);
          const { identities } = parseWholeStdout(waiting) as {
            identities: { presence: string }[];
          };
          const presence = identities[0].presence;
          expect(presence).not.toBe('active');
          expect(presence).not.toBe('offline');
          expect(storedEndpoints(sandbox)).toEqual(before);
          expect(herdr.json(['pane', 'get', 'w1:p1']).result.pane.tokens).toEqual(marker);
          const hinted = await herdr.inPane('w1:p1', `${tmt(sandbox)} whoami`);
          expect(hinted.stderr).toContain(HERDR_HINT);

          // Approved: the same binding is active on the same server ID.
          await approveHerdrDriver(sandbox);
          const listed = await runCli(sandbox, ['ls', '--json']);
          expect(parseWholeStdout(listed)).toMatchObject({
            identities: [{ name: 'worker', presence: 'active', target: 'w1:p1' }],
          });
          expect(storedEndpoints(sandbox)).toEqual(before);
          const whoami = await herdr.inPane('w1:p1', `${tmt(sandbox)} whoami`);
          expect(whoami.stdout).toBe('worker (saved) on pane w1:p1\n');
          expect(whoami.stderr).not.toContain(HERDR_HINT);

          // A message from another pane on the same server reaches the agent
          // in the binding's pane through the driver's prompt (a name routes
          // on the caller's own server).
          herdr.command(['pane', 'run', 'w1:p1', 'claude']);
          await until(
            () => herdr.json(['pane', 'get', 'w1:p1']).result.pane.agent === 'claude',
            'Herdr to see the agent'
          );
          herdr.json(['workspace', 'create', '--cwd', sandbox.cwd]);
          const talked = await herdr.inPane(
            'w2:p1',
            `${tmt(sandbox)} talk worker --detach carried-over-hello`
          );
          expect(talked.status, talked.stdout + talked.stderr).toBe(0);
          await until(
            () =>
              existsSync(received) && readFileSync(received, 'utf8').includes('carried-over-hello'),
            'the message at the agent'
          );
          await until(
            () => readFileSync(received, 'utf8').endsWith('exited\n'),
            'the agent to exit'
          ).catch((error) => {
            throw new Error(
              `${error.message}; received ${JSON.stringify(readFileSync(received, 'utf8'))}`
            );
          });

          // Unbind from the pane clears the marker.
          const unbound = await herdr.inPane('w1:p1', `${tmt(sandbox)} unbind`);
          expect(unbound.status, unbound.stderr).toBe(0);
          expect(herdr.json(['pane', 'get', 'w1:p1']).result.pane.tokens ?? {}).toEqual({});
          expect(existsSync(tmuxLog)).toBe(false);
        } finally {
          await stopHerdr(herdr);
        }
      });
    }, 60_000);
  }
);

// Runs without Herdr: `ls <text>` decides name-versus-target once, from
// storage, before any host is resolved (#498 decision 1). Herdr's targets
// are its driver's syntax, known once the driver is approved.
describe('ls with Herdr target-shaped text', () => {
  it('lists an existing identity that holds the name without running Herdr', async () => {
    await withSandbox(async (sandbox) => {
      await approveHerdrDriver(sandbox);
      const tmuxLog = installTmuxTripwire(sandbox);
      const herdrLog = path.join(sandbox.root, 'herdr-invocations.log');
      const tripwire = path.join(sandbox.root, 'herdr-tripwire');
      mkdirSync(tripwire);
      writeExecutable(
        path.join(tripwire, 'herdr'),
        `#!/bin/sh\nprintf "%s\\n" "$*" >> '${herdrLog}'\nexit 97\n`,
        0o755
      );
      // macOS delays a new executable's first run past a driver call's
      // deadline; run it once so the driver's call is the one recorded.
      spawnSync(path.join(tripwire, 'herdr'), ['warm-up']);
      rmSync(herdrLog, { force: true });
      sandbox.env.PATH = `${tripwire}${path.delimiter}${sandbox.env.PATH}`;
      delete sandbox.env.HERDR_PANE_ID;
      delete sandbox.env.HERDR_SOCKET_PATH;
      expect((await runCli(sandbox, ['identity', 'create', 'legacy'])).status).toBe(0);
      // Names like this were accepted before Herdr targets existed.
      const database = new Database(sandbox.database);
      try {
        database
          .prepare(
            "UPDATE identities SET name = 'w1:p2', canonical_name = 'w1:p2' WHERE canonical_name = 'legacy'"
          )
          .run();
      } finally {
        database.close();
      }

      const listed = await runCli(sandbox, ['ls', '--json', 'w1:p2']);
      expect(listed.status, listed.stderr).toBe(0);
      expect(parseWholeStdout(listed)).toMatchObject({
        identity: { name: 'w1:p2' },
        presence: 'offline',
      });
      expect((await runCli(sandbox, ['ls', 'w1:p2'])).status).toBe(0);
      expect(existsSync(herdrLog)).toBe(false);

      // Text no identity holds addresses a Herdr pane.
      const target = await runCli(sandbox, ['ls', 'w1:p3']);
      expect(target.status).not.toBe(0);
      expect(existsSync(herdrLog)).toBe(true);
      expect(existsSync(tmuxLog)).toBe(false);
    });
  });
});
