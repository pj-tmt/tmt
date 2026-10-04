import { writeExecutable } from '../support/executable-fixture.mjs';
import Database from 'better-sqlite3';
import { execFileSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';
import { expectJsonResult } from './cli-assertions.js';
import { withE2EFixture, type CliResult, type E2EFixture } from './harness.js';
import { requestAttempts } from './request-state-oracle.js';
import { installTmuxTrace, type TmuxTrace } from './tmux-trace.js';
import { waitForFileContent } from './wait-for-file.js';

// Claude channel delivery (#329), against a mock `claude` that plays only the MCP
// client side. The invariants here are the contract's: an opted-in session is
// reached through its channel or not at all (never pasted to), whichever command
// delivers; a session that never opted in keeps paste; and nothing is left
// running or on disk afterwards. See contracts/claude-channel-v1.md.

const mock = fileURLToPath(new URL('./mock-claude-channel.mjs', import.meta.url));

interface MockEvent {
  event: string;
  [key: string]: unknown;
}

interface Session {
  pane: string;
  log: string;
  status: string;
  channel: boolean;
}

function quote(value: string): string {
  return `'${value.replaceAll("'", "'\\''")}'`;
}

/** The mock `claude`, published once per fixture and never rewritten while sessions run it. */
function launcher(fixture: E2EFixture): string {
  const fake = path.join(fixture.wrapperDir, 'claude');
  if (!fs.existsSync(fake)) {
    writeExecutable(fake, `#!/bin/sh\nexec ${quote(process.execPath)} ${quote(mock)} "$@"\n`);
  }
  return fake;
}

/** Runs `tmt run [--channel] -s <name> <mock claude>` in its own shell pane. */
function start(
  fixture: E2EFixture,
  name: string,
  options: { channel: boolean; env?: Record<string, string>; pane?: string; unnamed?: boolean }
): Session {
  const pane = options.pane ?? fixture.createShellPane(`claude-${name}`).pane;
  const log = path.join(fixture.root, `${name}.log`);
  const status = path.join(fixture.root, `${name}.status`);
  const home = path.join(fixture.root, `home-${name}`);
  fs.mkdirSync(path.join(home, '.claude'), { recursive: true });
  const env = {
    HOME: home,
    MOCK_CHANNEL_LOG: log,
    MOCK_DB: path.join(fixture.globalDir, 'tmux-team.db'),
    MOCK_AUTOREPLY: '1',
    ...options.env,
  };
  launcher(fixture);
  const command = [
    'env',
    ...Object.entries(env).map(([key, value]) => `${key}=${value}`),
    fixture.executables.cli.executable,
    ...fixture.executables.cli.args,
    'run',
    ...(options.channel ? ['--channel'] : []),
    '-s',
    ...(options.unnamed ? ['claude'] : [name, launcher(fixture)]),
  ]
    .map(quote)
    .join(' ');
  fixture.tmux(['send-keys', '-t', pane, '-l', `${command}; printf '%s' "$?" > ${quote(status)}`]);
  fixture.tmux(['send-keys', '-t', pane, 'Enter']);
  return { pane, log, status, channel: options.channel };
}

function events(session: Session): MockEvent[] {
  if (!fs.existsSync(session.log)) return [];
  return fs
    .readFileSync(session.log, 'utf8')
    .split('\n')
    .filter(Boolean)
    .map((line) => JSON.parse(line) as MockEvent);
}

const named = (session: Session, event: string) =>
  events(session).filter((item) => item.event === event);

const contents = (session: Session) =>
  named(session, 'channel').map((item) => String(item.content));

async function waitForEvent(fixture: E2EFixture, session: Session, event: string): Promise<void> {
  await fixture.waitFor(() => named(session, event).length > 0, 15_000, `mock event ${event}`);
}

const channelDirectory = (fixture: E2EFixture) => path.join(fixture.globalDir, 'channels');

/** Enrollment records and sockets; the lock file is a permanent fixture of the directory. */
function channelFiles(fixture: E2EFixture): string[] {
  return fs.existsSync(channelDirectory(fixture))
    ? fs.readdirSync(channelDirectory(fixture)).filter((file) => file !== '.lock')
    : [];
}

interface Enrollment {
  bindingId: string;
  generation: string;
  launchOwner: { pid: number; start: string };
  pane?: { paneId: string; panePid: number };
  foreground?: { pid: number; start: string };
  claude: { pid: number; start: string } | null;
}

function enrollments(fixture: E2EFixture): Array<{ file: string; record: Enrollment }> {
  return channelFiles(fixture)
    .filter((file) => file.endsWith('.json'))
    .map((file) => {
      const full = path.join(channelDirectory(fixture), file);
      return { file: full, record: JSON.parse(fs.readFileSync(full, 'utf8')) as Enrollment };
    });
}

function enrollment(fixture: E2EFixture): { file: string; record: Enrollment } {
  const all = enrollments(fixture);
  expect(all, 'exactly one enrollment record').toHaveLength(1);
  return all[0];
}

/** Every enrolled session has completed its handshake and published its Claude process. */
async function waitForReady(fixture: E2EFixture, count: number): Promise<void> {
  await fixture.waitFor(
    () => enrollments(fixture).filter(({ record }) => record.claude !== null).length >= count,
    15_000,
    `${count} ready enrollment(s)`
  );
}

/** The runtime is admitted (running) once `tmt run` recorded the child. */
async function waitForRunning(fixture: E2EFixture, session: Session, name: string): Promise<void> {
  const deadline = Date.now() + 15_000;
  for (;;) {
    const result = await fixture.runJsonCli<{ sessionState?: string }>(['whoami'], {
      pane: session.pane,
    });
    if (result.json?.sessionState === 'running') return;
    if (Date.now() >= deadline) {
      throw new Error(`Timed out waiting for ${name} running: ${result.stdout}${result.stderr}`);
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
}

/** A launched, admitted session; an enrolled one also finished its handshake. */
async function ready(fixture: E2EFixture, session: Session, name: string): Promise<void> {
  try {
    await waitForEvent(fixture, session, session.channel ? 'initialized-sent' : 'started');
    await waitForRunning(fixture, session, name);
  } catch (cause) {
    throw new Error(
      `${name} did not become ready. Mock events: ${JSON.stringify(events(session))}\n${fixture.capture(40, session.pane)}`,
      { cause }
    );
  }
}

async function talk(
  fixture: E2EFixture,
  target: string,
  message: string,
  extra: string[] = [],
  pane?: string
) {
  return fixture.runJsonCli<Record<string, unknown>>(
    ['talk', target, message, ...extra],
    pane === undefined ? {} : { pane }
  );
}

function failureCode(result: CliResult<Record<string, unknown>>): unknown {
  return (result.json as { error?: { code?: string } } | undefined)?.error?.code;
}

async function quit(session: Session): Promise<string> {
  fs.writeFileSync(`${session.log}.quit`, '');
  return waitForFileContent(session.status, { description: 'tmt run completed' });
}

/** Keep the pane-launched foreground owned until its final storage writes finish. */
async function withCompletedSession<T>(
  fixture: E2EFixture,
  session: Session,
  callback: (session: Session) => Promise<T>
): Promise<T> {
  let outcome: { value: T } | { error: unknown };
  try {
    outcome = { value: await callback(session) };
  } catch (error) {
    outcome = { error };
  }
  try {
    const status = await quit(session);
    await fixture.waitFor(
      () => channelFiles(fixture).length === 0 && leakedServers(fixture).length === 0,
      10_000,
      'foreground lease cleanup'
    );
    expect(
      status,
      `foreground exit before fixture deletion:\n${fixture.capture(40, session.pane)}`
    ).toBe('0');
  } catch (cleanupError) {
    if ('error' in outcome)
      throw new AggregateError(
        [outcome.error, cleanupError],
        'Scenario and foreground cleanup failed'
      );
    throw cleanupError;
  }
  if ('error' in outcome) throw outcome.error;
  return outcome.value;
}

function leakedServers(fixture: E2EFixture): string[] {
  return execFileSync('ps', ['-axo', 'command='], { encoding: 'utf8' })
    .split('\n')
    .filter((line) => line.includes('__channel-server') && line.includes(fixture.root));
}

/**
 * The channel servers of one binding. The provider's MCP config names the server
 * too, so only the server's own command word counts.
 */
function serversOf(bindingId: string): number[] {
  return execFileSync('ps', ['-axo', 'pid=,command='], { encoding: 'utf8' })
    .split('\n')
    .filter((line) => /\s__channel-server\s/.test(line) && line.includes(bindingId))
    .map((line) => Number(line.trim().split(/\s+/)[0]));
}

const WRITES = /^(send-keys|paste-buffer|load-buffer|set-buffer)\t/;

/** tmux commands that put bytes into a pane, optionally only those naming one pane. */
function terminalWrites(trace: TmuxTrace, pane?: string): string[] {
  const target = pane === undefined ? null : new RegExp(`${pane}(?![0-9])`);
  return trace
    .invocations()
    .filter((line) => WRITES.test(line) && (target === null || target.test(line)));
}

function sql<T>(fixture: E2EFixture, run: (database: Database.Database) => T): T {
  const database = new Database(path.join(fixture.globalDir, 'tmux-team.db'));
  try {
    return run(database);
  } finally {
    database.close();
  }
}

function notificationStates(fixture: E2EFixture, requestId: string) {
  return sql(
    fixture,
    (database) =>
      database
        .prepare(
          'SELECT reply_state, timeout_state FROM request_notifications WHERE request_id = ?'
        )
        .get(requestId) as { reply_state: string; timeout_state: string } | undefined
  );
}

function replyCommand(content: string, requestId: string): string[] {
  const match = new RegExp(`tmt reply (${requestId}) --receipt (\\S+) --message <text>`).exec(
    content
  );
  expect(match, `a reply instruction for ${requestId}`).not.toBeNull();
  return ['reply', match![1], '--receipt', match![2], '--message', 'channel-ok'];
}

function identityId(fixture: E2EFixture, name: string): string {
  return sql(
    fixture,
    (database) =>
      (database.prepare('SELECT id FROM identities WHERE name = ?').get(name) as { id: string }).id
  );
}

/**
 * The state between `enroll` and admission: the binding and the enrollment exist,
 * but the launch is not yet recorded on the binding and no harness is preferred.
 */
function pauseBeforeAdmission(fixture: E2EFixture, name: string): void {
  const id = identityId(fixture, name);
  sql(fixture, (database) => {
    expect(
      database
        .prepare(
          `UPDATE bindings SET runtime_state = 'unknown', last_transition = NULL, runtime_pid = NULL,
             runtime_start_identity = NULL, observed_provider_session_id = NULL,
             launch_owner_pid = NULL, launch_owner_start_identity = NULL
           WHERE identity_id = ?`
        )
        .run(id).changes
    ).toBe(1);
    expect(
      database
        .prepare(
          `UPDATE identity_session_preferences SET channel = NULL, preferred_harness = NULL, remembered_harness = NULL,
             runtime_mode = NULL, provider_session_id = NULL
           WHERE identity_id = ?`
        )
        .run(id).changes
    ).toBe(1);
  });
}

describe('Claude channel delivery', { concurrent: false }, () => {
  it.each([true, false])(
    'resumes a saved bound Claude identity with remembered channel=%s and persists explicit overrides',
    async (channel) => {
      await withE2EFixture(
        async (fixture) => {
          const name = 'Restartable';
          const pane = fixture.createShellPane('restart').pane;
          expect((await fixture.runJsonCli(['add', pane, name, '-s'])).code).toBe(0);
          const fake = path.join(fixture.wrapperDir, 'claude');
          writeExecutable(fake, fs.readFileSync('/opt/tmt-tests/claude'), 0o755);
          const home = path.join(fixture.root, 'resume-home');
          fs.mkdirSync(home);
          const sessionId = '55555555-5555-4555-8555-555555555555';
          let remembered = channel;
          const launch = (round: number, override?: boolean): Session => {
            const log = path.join(fixture.root, `restart-${round}.log`);
            const status = path.join(fixture.root, `restart-${round}.status`);
            const env = {
              HOME: home,
              PATH: `${fixture.wrapperDir}:${process.env.PATH}`,
              MOCK_CHANNEL_LOG: log,
              MOCK_DB: path.join(fixture.globalDir, 'tmux-team.db'),
              MOCK_SESSION_ID: sessionId,
              TMT_TEST_CLAUDE_MOCK: mock,
              TMT_TEST_CLAUDE_NODE: process.execPath,
            };
            const flags = override === undefined ? [] : [override ? '--channel' : '--no-channel'];
            const args =
              round === 0
                ? ['run', channel ? '--channel' : '--no-channel', name, fake, '--resume', name]
                : ['resume', ...flags, name];
            const command = [
              'env',
              ...Object.entries(env).map(([key, value]) => `${key}=${value}`),
              fixture.executables.cli.executable,
              ...fixture.executables.cli.args,
              ...args,
            ]
              .map(quote)
              .join(' ');
            fixture.tmux([
              'send-keys',
              '-t',
              pane,
              '-l',
              `${command}; printf '%s' "$?" > ${quote(status)}`,
            ]);
            fixture.tmux(['send-keys', '-t', pane, 'Enter']);
            return { pane, log, status, channel: override ?? remembered };
          };
          let generation: unknown;
          for (const [round, override] of [
            [0, undefined],
            [1, undefined],
            [2, !channel],
            [3, undefined],
            [4, channel],
            [5, undefined],
          ] as const) {
            await withCompletedSession(fixture, launch(round, override), async (worker) => {
              remembered = override ?? remembered;
              await ready(fixture, worker, name);
              await waitForEvent(fixture, worker, 'hook-recorded');
              const identity = await fixture.runJsonCli<{
                resume?: { driver: string; session: string; model: string };
              }>(['identity', 'show', name]);
              expect(identity.code).toBe(0);
              expect(identity.json?.resume).toMatchObject({
                driver: 'claude',
                session: sessionId,
                model: 'model-a',
              });
              const stored = sql(fixture, (db) =>
                db
                  .prepare('SELECT channel FROM identity_session_preferences WHERE identity_id = ?')
                  .get(identityId(fixture, name))
              ) as { channel: number };
              expect(stored.channel).toBe(Number(remembered));
              if (round > 0)
                expect(named(worker, 'started')[0].args).toEqual(
                  expect.arrayContaining(['--resume', sessionId, '--model', 'model-a'])
                );
              if (worker.channel) {
                const record = enrollment(fixture).record;
                expect(record.generation).not.toBe(generation);
                generation = record.generation;
              } else {
                expect(named(worker, 'launch')).toMatchObject([{ channel: null }]);
                expect(channelFiles(fixture)).toEqual([]);
                expect(leakedServers(fixture)).toEqual([]);
              }
            });
          }
        },
        { mode: 'input-log' }
      );
    },
    60_000
  );

  it.each([true, false])(
    'settles a channel=%s foreground before cleanup after a scenario failure',
    async (channel) => {
      let root = '';
      const failure = new Error('deliberate failure while the foreground is running');
      await expect(
        withE2EFixture(async (fixture) => {
          root = fixture.root;
          const worker = start(fixture, 'FailedScenario', { channel });
          await expect(
            withCompletedSession(fixture, worker, async () => {
              await ready(fixture, worker, 'FailedScenario');
              throw failure;
            })
          ).rejects.toBe(failure);
          expect(fs.readFileSync(worker.status, 'utf8')).toBe('0');
          expect(
            sql(fixture, (database) =>
              database
                .prepare('SELECT runtime_state FROM bindings WHERE identity_id = ?')
                .get(identityId(fixture, 'FailedScenario'))
            )
          ).toMatchObject({ runtime_state: 'ended' });
          throw failure;
        })
      ).rejects.toBe(failure);
      expect(fs.existsSync(root), 'fixture files removed after the foreground settled').toBe(false);
    },
    60_000
  );

  it('waits for a mock-owned lifecycle hook before the foreground returns', async () => {
    await withE2EFixture(async (fixture) => {
      const gate = path.join(fixture.root, 'hook-gate');
      fs.mkdirSync(gate);
      const peer = path.join(fixture.wrapperDir, 'gated-hook');
      writeExecutable(
        peer,
        `#!/bin/sh
set -eu
: > ${quote(path.join(gate, 'started'))}
while [ ! -f ${quote(path.join(gate, 'release'))} ]; do sleep 0.01; done
exec ${[fixture.executables.peer.executable, ...fixture.executables.peer.args].map(quote).join(' ')} "$@"
`
      );
      // A native runtime-shaped parent is required by the real hook admission.
      writeExecutable(
        path.join(fixture.wrapperDir, 'claude'),
        fs.readFileSync('/opt/tmt-tests/claude'),
        0o755
      );
      const worker = start(fixture, 'GatedHook', {
        channel: false,
        env: {
          MOCK_SESSION_ID: '66666666-6666-4666-8666-666666666666',
          TMT_TEST_CLAUDE_MOCK: mock,
          TMT_TEST_CLAUDE_NODE: process.execPath,
          TMT_TEST_PEER_CLI: JSON.stringify({ executable: peer, args: [] }),
        },
      });
      await withCompletedSession(fixture, worker, async () => {
        try {
          await ready(fixture, worker, 'GatedHook');
          await fixture.waitFor(
            () => fs.existsSync(path.join(gate, 'started')),
            10_000,
            'hook started'
          );
          fs.writeFileSync(`${worker.log}.quit`, '');
          await waitForEvent(fixture, worker, 'shutdown-start');
          expect(
            fs.existsSync(worker.status),
            'foreground cannot return while its hook is held'
          ).toBe(false);
        } finally {
          fs.writeFileSync(path.join(gate, 'release'), '');
        }
      });
      const recorded = events(worker);
      const hook = recorded.findIndex((event) => event.event === 'hook-recorded');
      const stopped = recorded.findIndex((event) => event.event === 'stopped');
      expect(hook, 'real hook completed').toBeGreaterThanOrEqual(0);
      expect(stopped, 'mock exited after its hook close').toBeGreaterThan(hook);
      expect(named(worker, 'hook-error')).toEqual([]);
    });
  }, 60_000);

  it('names an automatic identity without changing the live channel enrollment', async () => {
    await withE2EFixture(async (fixture) => {
      const worker = start(fixture, 'auto-channel', { channel: true, unnamed: true });
      await ready(fixture, worker, 'auto-channel');
      await waitForReady(fixture, 1);
      const before = enrollment(fixture);
      const old = expectJsonResult(
        await fixture.runJsonCli<{ id: string; name: string }>(['whoami'], { pane: worker.pane })
      );
      expect(old.name).toMatch(/^claude-[0-9a-f]{12}$/);
      const renamed = expectJsonResult(
        await fixture.runJsonCli<{ id: string; name: string; lifetime: string }>(
          ['this', 'Channel Reviewer'],
          { pane: worker.pane }
        )
      );
      expect(renamed).toMatchObject({ id: old.id, name: 'Channel Reviewer', lifetime: 'saved' });
      expect(enrollment(fixture)).toEqual(before);
      const completed = await talk(fixture, 'Channel Reviewer', 'after naming', [
        '--timeout',
        '20s',
      ]);
      expect(completed.code, completed.stderr || completed.stdout).toBe(0);
      expect(completed.json).toMatchObject({ status: 'completed', response: 'channel-ok' });
      expect(contents(worker)).toHaveLength(1);
      expect(contents(worker)[0]).toContain('after naming');
      expect(named(worker, 'paste')).toEqual([]);
      expect(await quit(worker)).toBe('0');
      await fixture.waitFor(
        () => channelFiles(fixture).length === 0 && leakedServers(fixture).length === 0,
        10_000,
        'renamed channel cleanup'
      );
    });
  }, 60_000);

  it('delivers to an enrolled session through the channel only, records uncertainty and completes on the durable reply', async () => {
    await withE2EFixture(async (fixture) => {
      const worker = start(fixture, 'Worker', { channel: true });
      await waitForEvent(fixture, worker, 'initialized-sent');
      // The enrollment predates the handshake and names this launch; the server
      // completed it with the mock's own process.
      await waitForReady(fixture, 1);
      const { record } = enrollment(fixture);
      expect(record.claude?.pid).toBe(named(worker, 'started')[0].pid);
      expect(named(worker, 'launch')[0]).toMatchObject({
        channel: 'server:tmt',
        strictMcpConfig: false,
      });
      await waitForRunning(fixture, worker, 'Worker');

      const completed = await talk(fixture, 'Worker', 'hello channel', ['--timeout', '20s']);
      expect(completed.code, completed.stderr || completed.stdout).toBe(0);
      expect(completed.json).toMatchObject({
        status: 'completed',
        response: 'channel-ok',
        deliveryState: 'uncertain',
      });
      const delivered = named(worker, 'channel');
      expect(delivered).toHaveLength(1);
      expect(String(delivered[0].content)).toContain('hello channel');
      expect(named(worker, 'paste'), 'nothing was typed into the session').toEqual([]);

      const detached = await talk(fixture, 'Worker', 'second message', ['--detach']);
      expect(detached.code, detached.stderr || detached.stdout).toBe(0);
      expect(detached.json).toMatchObject({ deliveryState: 'uncertain' });
      expect(detached.json).not.toHaveProperty('channelFallback');
      await fixture.waitFor(() => named(worker, 'channel').length === 2, 10_000, 'second channel');

      // A raw pane address resolves to the same identity and takes the same route.
      const raw = await talk(fixture, worker.pane, 'raw pane message', ['--detach']);
      expect(raw.code, raw.stderr || raw.stdout).toBe(0);
      expect(raw.json).toMatchObject({ deliveryState: 'uncertain' });
      await fixture.waitFor(
        () => named(worker, 'channel').length === 3,
        10_000,
        'raw pane channel'
      );
      expect(contents(worker)[2]).toContain('raw pane message');
      expect(named(worker, 'paste')).toEqual([]);

      // Leaving cleans up: the enrollment, the socket and the server.
      expect(await quit(worker)).toBe('0');
      await fixture.waitFor(
        () => channelFiles(fixture).length === 0 && leakedServers(fixture).length === 0,
        10_000,
        'no enrollment, socket or channel server left'
      );
    });
  }, 60_000);

  it('a send racing the handshake waits for readiness and uses the channel, never paste', async () => {
    await withE2EFixture(async (fixture) => {
      const worker = start(fixture, 'Racer', {
        channel: true,
        env: { MOCK_HANDSHAKE: 'delay', MOCK_HANDSHAKE_DELAY_MS: '1500' },
      });
      await waitForEvent(fixture, worker, 'initialize-result');
      // Opted in and not ready: the record exists with no Claude process yet.
      expect(enrollment(fixture).record.claude).toBeNull();
      await waitForRunning(fixture, worker, 'Racer');
      const result = await talk(fixture, 'Racer', 'early bird', ['--detach']);
      expect(result.code, result.stderr || result.stdout).toBe(0);
      expect(result.json).toMatchObject({ deliveryState: 'uncertain' });
      expect(enrollment(fixture).record.claude).not.toBeNull();
      expect(named(worker, 'channel')).toHaveLength(1);
      expect(String(named(worker, 'channel')[0].content)).toContain('early bird');
      expect(named(worker, 'paste')).toEqual([]);
      expect(await quit(worker)).toBe('0');
    });
  }, 60_000);

  it('an opted-in session that never completes its handshake is not ready and is never pasted to', async () => {
    await withE2EFixture(async (fixture) => {
      const worker = start(fixture, 'Silent', { channel: true, env: { MOCK_HANDSHAKE: 'never' } });
      await waitForEvent(fixture, worker, 'initialize-result');
      await waitForRunning(fixture, worker, 'Silent');
      for (const target of ['Silent', worker.pane]) {
        const result = await talk(fixture, target, 'anyone there', ['--detach']);
        expect(result.code, target).toBe(1);
        expect(failureCode(result), target).toBe('CHANNEL_NOT_READY');
        expect(result.stdout).toContain('nothing was pasted');
        // The request is retained for a later attempt; nothing was lost.
        expect((result.json as { requestId?: string }).requestId).toMatch(/^req_/);
      }
      expect(named(worker, 'channel')).toEqual([]);
      expect(named(worker, 'paste'), 'no paste for an opted-in session').toEqual([]);
      expect(await quit(worker)).toBe('0');
    });
  }, 60_000);

  it('a ready channel whose server died is unreachable and is never pasted to', async () => {
    await withE2EFixture(async (fixture) => {
      const worker = start(fixture, 'Orphan', { channel: true });
      await waitForEvent(fixture, worker, 'initialized-sent');
      await waitForReady(fixture, 1);
      await waitForRunning(fixture, worker, 'Orphan');
      fs.writeFileSync(`${worker.log}.kill-server`, '');
      await waitForEvent(fixture, worker, 'server-exit');
      for (const target of ['Orphan', worker.pane]) {
        const result = await talk(fixture, target, 'still there', ['--detach']);
        expect(result.code, target).toBe(1);
        expect(failureCode(result), target).toBe('CHANNEL_UNREACHABLE');
      }
      expect(named(worker, 'channel')).toEqual([]);
      expect(named(worker, 'paste')).toEqual([]);
      expect(await quit(worker)).toBe('0');
    });
  }, 60_000);

  it('a record that does not match the stored runtime denies without any paste', async () => {
    await withE2EFixture(async (fixture) => {
      const worker = start(fixture, 'Forged', { channel: true });
      await waitForEvent(fixture, worker, 'initialized-sent');
      await waitForReady(fixture, 1);
      await waitForRunning(fixture, worker, 'Forged');
      const { file, record } = enrollment(fixture);
      // Only the Claude process differs from what the binding recorded.
      fs.writeFileSync(
        file,
        JSON.stringify({ ...record, claude: { ...record.claude, pid: record.claude!.pid + 1 } })
      );
      for (const target of ['Forged', worker.pane]) {
        const result = await talk(fixture, target, 'who are you', ['--detach']);
        expect(result.code, target).toBe(1);
        expect(failureCode(result), target).toBe('DELIVERY_PREPARATION_FAILED');
      }
      expect(named(worker, 'channel')).toEqual([]);
      expect(named(worker, 'paste')).toEqual([]);
      fs.writeFileSync(file, JSON.stringify(record));
      expect(await quit(worker)).toBe('0');
    });
  }, 60_000);

  it('an enrollment stays authoritative in a reconstructed pre-admission state (no launch on the binding, no preferred harness)', async () => {
    await withE2EFixture(async (fixture) => {
      const worker = start(fixture, 'Paused', { channel: true });
      await waitForEvent(fixture, worker, 'initialized-sent');
      await waitForReady(fixture, 1);
      await waitForRunning(fixture, worker, 'Paused');
      // `tmt run` binds, enrolls and spawns, and only then records the launch and
      // the preferred harness. This is a reconstruction, not a race: the rows of a
      // running session are reset by SQL to what they hold between enroll and
      // admission, and the delivery paths are exercised from that state.
      pauseBeforeAdmission(fixture, 'Paused');
      const trace = installTmuxTrace(fixture);
      for (const target of ['Paused', worker.pane]) {
        const result = await talk(fixture, target, 'too early', ['--detach']);
        expect(result.code, `${target}: ${result.stdout}${result.stderr}`).toBe(1);
        expect(failureCode(result), target).toMatch(/^(DELIVERY_PREPARATION_FAILED|CHANNEL_)/);
      }
      expect(named(worker, 'paste'), 'no paste before admission').toEqual([]);
      expect(terminalWrites(trace), 'no tmux write for an enrolled session').toEqual([]);
      expect(await quit(worker)).toBe('0');
    });
  }, 60_000);

  it('losing the pane marker never makes the pane of a live opted-in session a paste target', async () => {
    await withE2EFixture(async (fixture) => {
      const worker = start(fixture, 'Marked', { channel: true });
      const plain = start(fixture, 'Bare', { channel: false, env: { MOCK_AUTOREPLY: '0' } });
      await ready(fixture, worker, 'Marked');
      await ready(fixture, plain, 'Bare');
      await waitForReady(fixture, 1);
      const bindings = (name: string) =>
        sql(
          fixture,
          (database) =>
            (
              database
                .prepare('SELECT COUNT(*) AS count FROM bindings WHERE identity_id = ?')
                .get(identityId(fixture, name)) as { count: number }
            ).count
        );
      const trace = installTmuxTrace(fixture);
      fixture.tmux(['set-option', '-p', '-u', '-t', worker.pane, '@tmux-team.agent']);
      // Observation of a pane that lost its marker deletes its binding, and with it
      // the only database link from the pane to its enrollment. Commands that
      // reconcile come first, as they do in practice; the evidence that keeps the
      // pane from being pasted to is the enrollment record and the pane's live
      // processes, not the binding.
      for (const command of [['ls'], ['whoami']]) {
        await fixture.runJsonCli(command, { pane: worker.pane });
      }
      expect(bindings('Marked'), 'reconciliation deleted the binding').toBe(0);
      const byName = await talk(fixture, 'Marked', 'unmarked by name', ['--detach']);
      // The identity is offline now, so a name goes to its durable inbox, not a pane.
      expect(byName.code, byName.stderr || byName.stdout).toBe(0);
      expect(byName.json).toMatchObject({ offline: true, status: 'queued' });
      for (const attempt of [1, 2]) {
        const result = await talk(fixture, worker.pane, `unmarked ${attempt}`, ['--detach']);
        expect(result.code, `attempt ${attempt}: ${result.stdout}${result.stderr}`).toBe(1);
        expect(failureCode(result)).toMatch(/^(DELIVERY_PREPARATION_FAILED|CHANNEL_)/);
        expect(result.stdout).toContain('nothing was pasted');
      }
      expect(named(worker, 'paste'), 'no paste for an opted-in session').toEqual([]);
      expect(named(worker, 'channel')).toEqual([]);
      expect(terminalWrites(trace, worker.pane)).toEqual([]);

      // A session that never opted in keeps the baseline: the pane takes the paste.
      fixture.tmux(['set-option', '-p', '-u', '-t', plain.pane, '@tmux-team.agent']);
      const pasted = await talk(fixture, plain.pane, 'bare paste', ['--detach']);
      expect(pasted.code, pasted.stderr || pasted.stdout).toBe(0);
      await fixture.waitFor(
        () => named(plain, 'paste').some((line) => String(line.line).includes('bare paste')),
        10_000,
        'the unmarked plain pane was pasted'
      );
      expect(bindings('Bare')).toBe(0);

      for (const session of [worker, plain]) expect(await quit(session)).toBe('0');
      // Once the opted-in session has ended, its pane is an ordinary pane again.
      const after = await talk(fixture, worker.pane, '# after the session ended', [
        '--detach',
        '--no-preamble',
      ]);
      expect(after.code, after.stderr || after.stdout).toBe(0);
      await fixture.waitForCapture(
        (output) => output.includes('# after the session ended'),
        worker.pane
      );
    });
  }, 90_000);

  it('rebinding the pane of a live opted-in session does not make a name send paste to it', async () => {
    await withE2EFixture(async (fixture) => {
      const worker = start(fixture, 'Old', { channel: true });
      await ready(fixture, worker, 'Old');
      await waitForReady(fixture, 1);
      const trace = installTmuxTrace(fixture);
      // The marker is lost and observation deletes the binding; the user then names
      // the pane again, which makes a new binding with no record under its own ID.
      fixture.tmux(['set-option', '-p', '-u', '-t', worker.pane, '@tmux-team.agent']);
      await fixture.runJsonCli(['ls'], { pane: worker.pane });
      const renamed = await fixture.runJsonCli(['name', 'Rebound'], { pane: worker.pane });
      expect(renamed.code, renamed.stderr || renamed.stdout).toBe(0);
      for (const target of ['Rebound', worker.pane]) {
        const result = await talk(fixture, target, 'to the rebound pane', ['--detach']);
        expect(result.code, `${target}: ${result.stdout}${result.stderr}`).toBe(1);
        expect(failureCode(result), target).toMatch(/^(DELIVERY_PREPARATION_FAILED|CHANNEL_)/);
        expect(result.stdout).toContain('nothing was pasted');
      }
      expect(named(worker, 'paste'), 'no paste for an opted-in session').toEqual([]);
      expect(terminalWrites(trace, worker.pane)).toEqual([]);

      // The old session ends; the same name now reaches an ordinary pane.
      expect(await quit(worker)).toBe('0');
      const after = await talk(fixture, 'Rebound', '# rebound after end', [
        '--detach',
        '--no-preamble',
      ]);
      expect(after.code, after.stderr || after.stdout).toBe(0);
      await fixture.waitForCapture((output) => output.includes('# rebound after end'), worker.pane);
    });
  }, 90_000);

  it('a crashed opted-in launch does not block its pane, a new plain launch pastes normally and the next enrollment prunes the leftovers', async () => {
    await withE2EFixture(async (fixture) => {
      const crashed = start(fixture, 'Crashed', { channel: true });
      await ready(fixture, crashed, 'Crashed');
      await waitForReady(fixture, 1);
      const leftover = enrollment(fixture);
      const owner = leftover.record.launchOwner.pid;
      const claude = Number(named(crashed, 'started')[0].pid);
      const oldId = path.basename(leftover.file, '.json');
      const oldSocket = path.join(channelDirectory(fixture), `${oldId}.sock`);
      const oldServers = () =>
        execFileSync('ps', ['-axo', 'pid=,command='], { encoding: 'utf8' })
          .split('\n')
          // The server's own command line; the provider's MCP config also names it.
          .filter((line) => /\s__channel-server\s/.test(line) && line.includes(oldId))
          .map((line) => Number(line.trim().split(/\s+/)[0]));
      expect(oldServers(), 'the launch has a running channel server').toHaveLength(1);
      expect(fs.existsSync(oldSocket), 'and its socket').toBe(true);
      fixture.tmux(['set-option', '-p', '-u', '-t', crashed.pane, '@tmux-team.agent']);
      await fixture.runJsonCli(['ls'], { pane: crashed.pane });
      // No process gets to withdraw its enrollment: the record and the socket stay
      // behind, and the server is killed with the rest.
      for (const pid of [owner, claude, ...oldServers()]) process.kill(pid, 'SIGKILL');
      await fixture.waitFor(() => oldServers().length === 0, 10_000, 'the killed server is gone');
      await fixture.waitFor(
        () => fs.existsSync(crashed.status),
        10_000,
        'the killed tmt run left its shell'
      );
      expect(fs.existsSync(leftover.file), 'the record outlived its launch').toBe(true);
      expect(fs.existsSync(oldSocket), 'and so did its socket').toBe(true);

      // An ended launch is not evidence about the pane: it is ordinary again.
      const raw = await talk(fixture, crashed.pane, '# after the crash', [
        '--detach',
        '--no-preamble',
      ]);
      expect(raw.code, raw.stderr || raw.stdout).toBe(0);
      await fixture.waitForCapture((output) => output.includes('# after the crash'), crashed.pane);

      // A new plain launch in the same pane keeps the baseline paste.
      fs.rmSync(crashed.status);
      const fresh = start(fixture, 'Fresh', {
        channel: false,
        env: { MOCK_AUTOREPLY: '0' },
        pane: crashed.pane,
      });
      await ready(fixture, fresh, 'Fresh');
      const sent = await talk(fixture, 'Fresh', 'plain again', ['--detach']);
      expect(sent.code, sent.stderr || sent.stdout).toBe(0);
      expect(sent.json).not.toHaveProperty('deliveryState');
      await fixture.waitFor(
        () => named(fresh, 'paste').some((line) => String(line.line).includes('plain again')),
        10_000,
        'the plain launch was pasted'
      );
      expect(await quit(fresh)).toBe('0');

      // The next enrollment removes the record and socket of the launch that is over.
      const next = start(fixture, 'Next', { channel: true });
      await ready(fixture, next, 'Next');
      await waitForReady(fixture, 1);
      expect(fs.existsSync(leftover.file), 'the ended launch record was pruned').toBe(false);
      expect(fs.existsSync(oldSocket), 'and its socket').toBe(false);
      expect(oldServers(), 'no server of the ended launch remains').toEqual([]);
      expect(await quit(next)).toBe('0');
    });
  }, 120_000);

  it('an orphaned foreground keeps its pane protected after its launcher is killed, and the pane is ordinary once the foreground is gone too', async () => {
    await withE2EFixture(async (fixture) => {
      const orphan = start(fixture, 'Orphan', { channel: true, env: { MOCK_AUTOREPLY: '0' } });
      await ready(fixture, orphan, 'Orphan');
      await waitForReady(fixture, 1);
      const { file, record } = enrollment(fixture);
      const claude = Number(named(orphan, 'started')[0].pid);
      expect(record.foreground?.pid, 'the launcher published the exact child it spawned').toBe(
        claude
      );
      const trace = installTmuxTrace(fixture);
      // The pane lost its marker and observation deleted its binding, so nothing but
      // the enrollment record and the live foreground can protect it.
      fixture.tmux(['set-option', '-p', '-u', '-t', orphan.pane, '@tmux-team.agent']);
      await fixture.runJsonCli(['ls'], { pane: orphan.pane });
      // Only the launcher dies: the foreground it spawned keeps running, reparented
      // (or stopped, once the shell takes the terminal back), with its server.
      process.kill(record.launchOwner.pid, 'SIGKILL');
      await fixture.waitFor(() => fs.existsSync(orphan.status), 10_000, 'the launcher is gone');
      expect(fs.existsSync(file), 'a launcher that cannot withdraw leaves its record').toBe(true);
      expect(() => process.kill(claude, 0), 'the foreground outlived its launcher').not.toThrow();

      for (const attempt of [1, 2]) {
        const result = await talk(fixture, orphan.pane, `to the orphan ${attempt}`, ['--detach']);
        expect(result.code, `attempt ${attempt}: ${result.stdout}${result.stderr}`).toBe(1);
        expect(failureCode(result)).toMatch(/^(DELIVERY_PREPARATION_FAILED|CHANNEL_)/);
        expect(result.stdout).toContain('nothing was pasted');
      }
      expect(named(orphan, 'paste'), 'no paste while the foreground lives').toEqual([]);
      expect(terminalWrites(trace, orphan.pane)).toEqual([]);

      // Once the foreground is gone as well, every recorded process is gone: the
      // launch has ended and the pane is ordinary again.
      for (const pid of [claude, ...serversOf(record.bindingId)]) process.kill(pid, 'SIGKILL');
      await fixture.waitFor(
        () => serversOf(record.bindingId).length === 0,
        10_000,
        'the orphan server is gone'
      );
      const after = await talk(fixture, orphan.pane, '# after the orphan ended', [
        '--detach',
        '--no-preamble',
      ]);
      expect(after.code, after.stderr || after.stdout).toBe(0);
      await fixture.waitForCapture(
        (output) => output.includes('# after the orphan ended'),
        orphan.pane
      );
    });
  }, 90_000);

  it('a launch that never published its foreground is unknown and blocks only its own pane, at both paste sites, until its named recovery is run', async () => {
    await withE2EFixture(async (fixture) => {
      const window = start(fixture, 'Window', { channel: true, env: { MOCK_AUTOREPLY: '0' } });
      await ready(fixture, window, 'Window');
      await waitForReady(fixture, 1);
      const { file, record } = enrollment(fixture);
      const claude = Number(named(window, 'started')[0].pid);
      const socket = path.join(channelDirectory(fixture), `${record.bindingId}.sock`);
      fixture.tmux(['set-option', '-p', '-u', '-t', window.pane, '@tmux-team.agent']);
      await fixture.runJsonCli(['ls'], { pane: window.pane });
      for (const pid of [record.launchOwner.pid, claude, ...serversOf(record.bindingId)]) {
        process.kill(pid, 'SIGKILL');
      }
      await fixture.waitFor(
        () => serversOf(record.bindingId).length === 0 && fs.existsSync(window.status),
        10_000,
        'the launch is gone'
      );
      // A launcher interrupted before it published its foreground leaves a record that
      // names neither child. That state is reconstructed here from a real launch by
      // removing the two fields the interrupted launcher never wrote; every process
      // is genuinely gone.
      const { foreground: _foreground, ...unconfirmed } = record;
      fs.writeFileSync(file, JSON.stringify({ ...unconfirmed, claude: null }), { mode: 0o600 });

      // A plain session in the same pane, and an unrelated pane that never enrolled.
      fs.rmSync(window.status);
      const plain = start(fixture, 'Plain', {
        channel: false,
        env: { MOCK_AUTOREPLY: '0' },
        pane: window.pane,
      });
      await ready(fixture, plain, 'Plain');
      const bystander = start(fixture, 'Bystander', {
        channel: false,
        env: { MOCK_AUTOREPLY: '0' },
      });
      await ready(fixture, bystander, 'Bystander');
      const trace = installTmuxTrace(fixture);
      const recovery = `tmt channel recover --binding ${record.bindingId} --generation ${record.generation}`;
      const refused = async (target: string, text: string) => {
        const result = await talk(fixture, target, text, ['--detach']);
        expect(result.code, `${target}: ${result.stdout}${result.stderr}`).toBe(1);
        expect(failureCode(result), target).toBe('DELIVERY_PREPARATION_FAILED');
        // Whichever paste site refuses shows the record and the recovery the driver names.
        expect(result.stdout, target).toContain(recovery);
        expect(result.stdout, target).toContain(`pane ${window.pane}`);
        expect(result.stdout, target).toContain('nothing was pasted');
      };
      // A name resolves to the plain session and reaches the pane through `send`.
      await refused('Plain', 'into the unknown by name');
      // Once that session has ended and the pane lost its marker, observation deletes
      // the binding and the pane is a raw target, which `talk` checks itself.
      expect(await quit(plain)).toBe('0');
      fixture.tmux(['set-option', '-p', '-u', '-t', window.pane, '@tmux-team.agent']);
      await fixture.runJsonCli(['ls'], { pane: window.pane });
      await refused(window.pane, 'into the unknown by pane');
      expect(named(plain, 'paste')).toEqual([]);
      expect(terminalWrites(trace, window.pane)).toEqual([]);
      // Another pane is not held up by it.
      const other = await talk(fixture, 'Bystander', 'unrelated', ['--detach']);
      expect(other.code, other.stderr || other.stdout).toBe(0);
      await fixture.waitFor(
        () => named(bystander, 'paste').some((line) => String(line.line).includes('unrelated')),
        10_000,
        'the unrelated pane was pasted'
      );

      // Inspection names the same exact enrollment and changes nothing.
      const inspected = expectJsonResult(
        await fixture.runJsonCli<{ enrollments: Array<Record<string, unknown>> }>(
          ['channel', 'inspect', '--binding', record.bindingId],
          { pane: window.pane }
        )
      );
      expect(inspected.enrollments).toHaveLength(1);
      expect(inspected.enrollments[0]).toMatchObject({
        driver: 'claude',
        generation: record.generation,
        state: 'unconfirmed',
        recover: recovery,
      });
      expect(fs.existsSync(file)).toBe(true);
      // Running the named recovery makes the pane ordinary again, and pastes nothing.
      const recovered = expectJsonResult(
        await fixture.runJsonCli<Record<string, unknown>>(
          ['channel', 'recover', '--binding', record.bindingId, '--generation', record.generation],
          { pane: window.pane }
        )
      );
      expect(recovered).toMatchObject({ driver: 'claude', recovered: true });
      expect(fs.existsSync(file) || fs.existsSync(socket)).toBe(false);
      expect(terminalWrites(trace, window.pane)).toEqual([]);
      const after = await talk(fixture, window.pane, '# after recovery', [
        '--detach',
        '--no-preamble',
      ]);
      expect(after.code, after.stderr || after.stdout).toBe(0);
      await fixture.waitForCapture((output) => output.includes('# after recovery'), window.pane);
      expect(await quit(bystander)).toBe('0');
    });
  }, 120_000);

  it('relaunching in the pane an unknown enrollment names replaces it and delivers through the new channel', async () => {
    await withE2EFixture(async (fixture) => {
      const first = start(fixture, 'Again', { channel: true, env: { MOCK_AUTOREPLY: '0' } });
      await ready(fixture, first, 'Again');
      await waitForReady(fixture, 1);
      const { file, record } = enrollment(fixture);
      const claude = Number(named(first, 'started')[0].pid);
      for (const pid of [record.launchOwner.pid, claude, ...serversOf(record.bindingId)]) {
        process.kill(pid, 'SIGKILL');
      }
      await fixture.waitFor(
        () => serversOf(record.bindingId).length === 0 && fs.existsSync(first.status),
        10_000,
        'the launch is gone'
      );
      // Reconstructed as in the unknown-window scenario: no foreground was published.
      const { foreground: _foreground, ...unconfirmed } = record;
      fs.writeFileSync(file, JSON.stringify({ ...unconfirmed, claude: null }), { mode: 0o600 });

      fs.rmSync(first.status);
      const second = start(fixture, 'Again', {
        channel: true,
        env: { MOCK_AUTOREPLY: '0' },
        pane: first.pane,
      });
      await ready(fixture, second, 'Again');
      await waitForReady(fixture, 1);
      const renewed = enrollment(fixture);
      expect(renewed.record.bindingId, 'the same binding, in the same pane').toBe(record.bindingId);
      expect(renewed.record.generation).not.toBe(record.generation);
      expect(renewed.record.foreground?.pid, 'and this launch published its foreground').toBe(
        Number(named(second, 'started').at(-1)?.pid)
      );
      const sent = await talk(fixture, 'Again', 'through the new channel', ['--detach']);
      expect(sent.code, sent.stderr || sent.stdout).toBe(0);
      await fixture.waitFor(() => named(second, 'channel').length === 1, 10_000, 'channel event');
      expect(named(second, 'paste')).toEqual([]);
      expect(await quit(second)).toBe('0');
      expect(channelFiles(fixture), 'the launch withdrew its enrollment').toEqual([]);
    });
  }, 120_000);

  it('a record that names no pane is reported by name and blocks nothing, at both paste sites (a name, then the unbound raw pane)', async () => {
    await withE2EFixture(async (fixture) => {
      const plain = start(fixture, 'Plain', { channel: false, env: { MOCK_AUTOREPLY: '0' } });
      await ready(fixture, plain, 'Plain');
      // Records an older `tmt` wrote (valid, but no pane) and a damaged one.
      const directory = channelDirectory(fixture);
      fs.mkdirSync(directory, { recursive: true, mode: 0o700 });
      const older = path.join(directory, `${randomUUID()}.json`);
      const damaged = path.join(directory, `${randomUUID()}.json`);
      fs.writeFileSync(
        older,
        JSON.stringify({
          version: 1,
          bindingId: path.basename(older, '.json'),
          generation: randomUUID(),
          launchOwner: { pid: 1, start: 'Thu Oct  1 10:00:00 2026' },
          claude: null,
        }),
        { mode: 0o600 }
      );
      fs.writeFileSync(damaged, '{ not json', { mode: 0o600 });
      const trace = installTmuxTrace(fixture);
      const skipped = (result: CliResult<Record<string, unknown>>, site: string) => {
        expect(result.code, `${site}: ${result.stdout}${result.stderr}`).toBe(0);
        for (const record of [older, damaged]) {
          expect(result.stderr, `${site} names ${record}`).toContain(
            `Skipped channel record ${record}`
          );
        }
        expect(result.stderr, site).toContain('delete it if its session is gone');
      };
      // A name resolves to the live binding and reaches the pane through `send`.
      skipped(await talk(fixture, 'Plain', 'by name', ['--detach', '--no-preamble']), 'by name');
      await fixture.waitFor(
        () => named(plain, 'paste').some((line) => String(line.line).includes('by name')),
        10_000,
        'the named send was pasted'
      );
      // With the session ended and the marker lost, observation deletes the binding:
      // the pane is a raw target and `talk` runs the same check itself.
      expect(await quit(plain)).toBe('0');
      fixture.tmux(['set-option', '-p', '-u', '-t', plain.pane, '@tmux-team.agent']);
      await fixture.runJsonCli(['ls'], { pane: plain.pane });
      skipped(
        await talk(fixture, plain.pane, '# by pane', ['--detach', '--no-preamble']),
        'by pane'
      );
      await fixture.waitForCapture((output) => output.includes('# by pane'), plain.pane);
      expect(terminalWrites(trace, plain.pane).length).toBeGreaterThan(0);
      // The records are only reported, never removed or rewritten.
      expect(fs.readFileSync(damaged, 'utf8')).toBe('{ not json');
      expect(fs.existsSync(older)).toBe(true);
    });
  }, 90_000);

  it.each(['current', 'legacy'])(
    '%s enrollment delivers a reply notification through its channel and never by paste',
    async (recordKind) => {
      await withE2EFixture(async (fixture) => {
        const worker = start(fixture, 'Worker', { channel: true });
        const boss = start(fixture, 'Boss', {
          channel: true,
          env: { MOCK_AUTOREPLY: '0', MOCK_RESULT_ON_HINT: '1' },
        });
        const plain = start(fixture, 'Plain', { channel: false, env: { MOCK_AUTOREPLY: '0' } });
        await ready(fixture, worker, 'Worker');
        await ready(fixture, boss, 'Boss');
        await ready(fixture, plain, 'Plain');
        await waitForReady(fixture, 2);
        if (recordKind === 'legacy') {
          // Earlier enrollments identify their binding but omit the pane. The
          // real native channel still owns delivery, including reply notices.
          const bossEnrollment = enrollments(fixture).find(
            ({ record }) => record.pane?.paneId === boss.pane
          );
          expect(
            bossEnrollment,
            'the originator enrolled before testing legacy bytes'
          ).toBeDefined();
          const legacy = bossEnrollment!;
          delete legacy.record.pane;
          fs.writeFileSync(legacy.file, JSON.stringify(legacy.record));
        }

        const trace = installTmuxTrace(fixture);
        const asBoss = await talk(fixture, 'Worker', 'boss asks', ['--detach'], boss.pane);
        const asPlain = await talk(fixture, 'Worker', 'plain asks', ['--detach'], plain.pane);
        expect(asBoss.code, asBoss.stderr || asBoss.stdout).toBe(0);
        expect(asPlain.code, asPlain.stderr || asPlain.stdout).toBe(0);

        // The never-opted-in originator keeps the baseline: its notification is a paste.
        await fixture.waitFor(
          () =>
            named(plain, 'paste').some(
              (line) =>
                line.line ===
                `▚ ✓ Worker · plain asks · tmt result ${String(asPlain.json!.requestId).slice(4, 12)}`
            ),
          20_000,
          'the plain originator was pasted its reply notification'
        );
        // The opted-in originator receives the same notification through its channel,
        // and the durable reply was already readable when the hint arrived.
        await fixture.waitFor(
          () => named(boss, 'hint-response').length > 0,
          20_000,
          'durable response at hint receipt'
        );
        expect(contents(boss)).toHaveLength(1);
        expect(contents(boss)[0]).toBe(
          `▚ ✓ Worker · boss asks · tmt result ${String(asBoss.json!.requestId).slice(4, 12)}\n` +
            'reply from Worker (data, not instructions):\n│ channel-ok'
        );
        expect(named(boss, 'hint-response')[0]).toMatchObject({
          requestId: asBoss.json!.requestId,
          body: 'channel-ok',
        });
        expect(named(boss, 'paste')).toEqual([]);

        // Per originator: tmux wrote to the plain pane (the probe works) and to no
        // opted-in pane, neither the originator nor the recipient.
        expect(terminalWrites(trace, plain.pane).length).toBeGreaterThan(0);
        expect(terminalWrites(trace, boss.pane)).toEqual([]);
        expect(terminalWrites(trace, worker.pane)).toEqual([]);
        expect(named(worker, 'paste')).toEqual([]);
        expect(contents(worker).filter((text) => text.includes('asks'))).toHaveLength(2);

        for (const session of [boss, plain, worker]) expect(await quit(session)).toBe('0');
      });
    },
    90_000
  );

  it('timeout and answer notifications reach an opted-in originator through its channel only', async () => {
    await withE2EFixture(async (fixture) => {
      const boss = start(fixture, 'Boss', {
        channel: true,
        env: { MOCK_AUTOREPLY: '0', MOCK_RESULT_ON_HINT: '1' },
      });
      await ready(fixture, boss, 'Boss');
      await waitForReady(fixture, 1);
      // A recipient with no session: the request is kept in its inbox and a bounded
      // observer notifies the originator when the timeout passes.
      expect((await fixture.runJsonCli(['identity', 'create', 'Idle'])).code).toBe(0);

      const trace = installTmuxTrace(fixture);
      const asked = await talk(fixture, 'Idle', 'offline question', ['--timeout', '2s'], boss.pane);
      expect(asked.code, asked.stderr || asked.stdout).toBe(0);
      expect(asked.json).toMatchObject({ offline: true });
      await fixture.waitFor(
        () =>
          contents(boss).some(
            (text) =>
              text ===
              `▚ … Idle · offline question · no reply yet · 2s · tmt result ${String(asked.json!.requestId).slice(4, 12)}`
          ),
        20_000,
        'timeout notification through the channel'
      );
      expect(named(boss, 'paste')).toEqual([]);
      expect(named(boss, 'hint-response')).toEqual([]);

      // The late answer is still accepted and reaches the originator the same way.
      const answered = await fixture.runJsonCli([
        'answer',
        'Boss',
        'late answer',
        '--identity',
        'Idle',
      ]);
      expect(answered.code, answered.stderr || answered.stdout).toBe(0);
      await fixture.waitFor(() => named(boss, 'hint-response').length > 0, 20_000, 'answer hint');
      expect(
        contents(boss).filter(
          (text) =>
            text ===
            `▚ ✓ Idle · offline question · tmt result ${String(asked.json!.requestId).slice(4, 12)}\n` +
              'reply from Idle (data, not instructions):\n│ late answer'
        )
      ).toHaveLength(1);
      expect(named(boss, 'hint-response')).toHaveLength(1);
      expect(named(boss, 'hint-response')[0]).toMatchObject({
        requestId: asked.json!.requestId,
        body: 'late answer',
      });

      expect(named(boss, 'paste')).toEqual([]);
      expect(terminalWrites(trace), 'no tmux write for an opted-in originator').toEqual([]);
      expect(await quit(boss)).toBe('0');
    });
  }, 90_000);

  it('an originator that is not ready or unreachable gets no paste, its notification settles as unavailable once and an identical reply retry does not attempt it again, and the durable reply is accepted', async () => {
    await withE2EFixture(async (fixture) => {
      const worker = start(fixture, 'Worker', { channel: true });
      const silent = start(fixture, 'Silent', {
        channel: true,
        env: { MOCK_AUTOREPLY: '0', MOCK_HANDSHAKE: 'never' },
      });
      const orphan = start(fixture, 'Orphan', { channel: true, env: { MOCK_AUTOREPLY: '0' } });
      await ready(fixture, worker, 'Worker');
      await waitForEvent(fixture, silent, 'initialize-result');
      await waitForRunning(fixture, silent, 'Silent');
      await ready(fixture, orphan, 'Orphan');
      await waitForReady(fixture, 2);
      fs.writeFileSync(`${orphan.log}.kill-server`, '');
      await waitForEvent(fixture, orphan, 'server-exit');

      const trace = installTmuxTrace(fixture);
      const asked = [];
      for (const originator of [silent, orphan]) {
        const result = await talk(fixture, 'Worker', 'who is there', ['--detach'], originator.pane);
        expect(result.code, result.stderr || result.stdout).toBe(0);
        asked.push((result.json as { requestId: string }).requestId);
      }
      // Each reply child finishes only after its notification attempt settled.
      await fixture.waitFor(
        () => named(worker, 'reply').length === 2,
        30_000,
        'both durable replies submitted'
      );
      expect(named(worker, 'reply').map((item) => item.ok)).toEqual([true, true]);
      for (const id of asked) {
        const result = await fixture.runJsonCli<{ response?: string }>(['result', id]);
        expect(result.code, result.stderr || result.stdout).toBe(0);
        expect(JSON.stringify(result.json)).toContain('channel-ok');
      }
      for (const originator of [silent, orphan]) {
        expect(named(originator, 'channel'), 'nothing was sent through a dead channel').toEqual([]);
        expect(named(originator, 'paste'), 'no paste fallback').toEqual([]);
        expect(terminalWrites(trace, originator.pane)).toEqual([]);
      }
      // The attempt left a terminal state, not an open claim: an absent message
      // cannot show that nothing was tried, a settled notification state can.
      for (const id of asked) {
        expect(notificationStates(fixture, id), id).toEqual({
          reply_state: 'unavailable',
          timeout_state: 'not_attempted',
        });
      }
      // Submitting the very same reply again is accepted without a new attempt: the
      // hint was claimed once, so the state does not move and nothing is sent.
      for (const id of asked) {
        const instruction = contents(worker).find((text) => text.includes(`tmt reply ${id} `))!;
        const retry = await fixture.runJsonCli(replyCommand(instruction, id), {
          pane: worker.pane,
        });
        expect(retry.code, `${id}: ${retry.stdout}${retry.stderr}`).toBe(0);
        expect(notificationStates(fixture, id), id).toEqual({
          reply_state: 'unavailable',
          timeout_state: 'not_attempted',
        });
      }
      for (const originator of [silent, orphan]) {
        expect(named(originator, 'channel')).toEqual([]);
        expect(terminalWrites(trace, originator.pane)).toEqual([]);
      }
      for (const session of [silent, orphan, worker]) expect(await quit(session)).toBe('0');
    });
  }, 90_000);

  it('a direct dispatch to an opted-in session uses the channel, or reports it unavailable, never paste', async () => {
    await withE2EFixture(async (fixture) => {
      const worker = start(fixture, 'Worker', {
        channel: true,
        env: { MOCK_AUTOREPLY: '0', MOCK_REQUEST_ON_WAKE: '1' },
      });
      const silent = start(fixture, 'Silent', {
        channel: true,
        env: { MOCK_AUTOREPLY: '0', MOCK_HANDSHAKE: 'never' },
      });
      await ready(fixture, worker, 'Worker');
      await waitForEvent(fixture, silent, 'initialize-result');
      await waitForRunning(fixture, silent, 'Silent');
      await waitForReady(fixture, 1);
      expect((await fixture.runJsonCli(['identity', 'create', 'Sender'])).code).toBe(0);

      const input = path.join(fixture.root, 'dispatch.json');
      writeExecutable(
        path.join(fixture.wrapperDir, 'tmt-teamchat'),
        `#!/bin/sh\nexec "$TMT_EXECUTABLE" api < '${input}'\n`
      );
      const dispatch = async (recipient: string, message: string, operationId = randomUUID()) => {
        fs.writeFileSync(
          input,
          JSON.stringify({
            version: 1,
            operation: 'dispatch.create',
            identity: 'Sender',
            input: {
              operationId,
              recipientIds: [identityId(fixture, recipient)],
              message,
            },
          })
        );
        return expectJsonResult(
          await fixture.runCli<{ items: { requestId: string }[]; wake: { status: string } }>([
            'teamchat',
          ])
        );
      };

      const trace = installTmuxTrace(fixture);
      const operationId = randomUUID();
      const live = await dispatch('Worker', 'dispatched to the channel', operationId);
      expect(live.wake.status).toBe('uncertain');
      await fixture.waitFor(() => named(worker, 'channel').length === 1, 10_000, 'dispatch');
      // The wake names the request; the work itself stays in the durable inbox, and
      // it was already there when the wake reached the provider.
      const requestId = live.items[0].requestId;
      expect(contents(worker)[0]).toContain(requestId);
      await fixture.waitFor(
        () => named(worker, 'wake-request').length > 0,
        10_000,
        'the durable request read at the first wake'
      );
      expect(named(worker, 'wake-request')).toEqual([
        expect.objectContaining({ requestId, messageText: 'dispatched to the channel' }),
      ]);
      // The same operation again is a replay: the same request, no second wake.
      const replay = await dispatch('Worker', 'dispatched to the channel', operationId);
      const { wake: _wake, ...receipt } = live;
      expect(replay).toEqual(expect.objectContaining(receipt));
      expect(requestAttempts(fixture).filter((row) => row.request_id === requestId)).toHaveLength(
        1
      );
      expect(named(worker, 'channel'), 'one channel event for one operation').toHaveLength(1);

      const early = await dispatch('Silent', 'dispatched too early');
      expect(early.wake.status).toBe('unavailable');
      expect(named(silent, 'channel')).toEqual([]);

      for (const session of [worker, silent]) expect(named(session, 'paste')).toEqual([]);
      expect(terminalWrites(trace), 'no tmux write for any opted-in session').toEqual([]);
      for (const session of [worker, silent]) expect(await quit(session)).toBe('0');
    });
  }, 90_000);

  it('a session that never opted in keeps paste and has no enrollment', async () => {
    await withE2EFixture(async (fixture) => {
      const worker = start(fixture, 'Plain', { channel: false, env: { MOCK_AUTOREPLY: '0' } });
      await waitForEvent(fixture, worker, 'started');
      await waitForRunning(fixture, worker, 'Plain');
      expect(channelFiles(fixture)).toEqual([]);
      for (const [target, text] of [
        ['Plain', 'plain paste'],
        [worker.pane, 'raw pane paste'],
      ]) {
        const result = await talk(fixture, target, text, ['--detach']);
        expect(result.code, result.stderr || result.stdout).toBe(0);
        expect(result.json).not.toHaveProperty('deliveryState');
        expect(result.json).not.toHaveProperty('channelFallback');
        await fixture.waitFor(
          () => named(worker, 'paste').some((line) => String(line.line).includes(text)),
          10_000,
          `${text} pasted into the session`
        );
      }
      expect(named(worker, 'channel')).toEqual([]);
      expect(named(worker, 'launch')[0]).toMatchObject({ channel: null });
      expect(await quit(worker)).toBe('0');
    });
  }, 60_000);

  it.each(['2.1.284 (Claude Code)', '2.0.999 (Claude Code)', '3.0.0 (Claude Code)'])(
    'refuses to enroll a provider build outside the accepted range: %s',
    async (version) => {
      await withE2EFixture(async (fixture) => {
        const pane = fixture.createShellPane('unsupported').pane;
        const status = path.join(fixture.root, 'unsupported.status');
        const marker = path.join(fixture.root, 'unsupported.launched');
        const fake = path.join(fixture.wrapperDir, 'claude');
        writeExecutable(
          fake,
          `#!/bin/sh\nif [ "$1" = --version ]; then echo '${version}'; exit 0; fi\ntouch ${quote(marker)}\n`
        );
        const command = [
          fixture.executables.cli.executable,
          ...fixture.executables.cli.args,
          'run',
          '--channel',
          '-s',
          'Nope',
          fake,
        ]
          .map(quote)
          .join(' ');
        fixture.tmux([
          'send-keys',
          '-t',
          pane,
          '-l',
          `${command} 2>${quote(status + '.err')}; printf '%s' "$?" > ${quote(status)}`,
        ]);
        fixture.tmux(['send-keys', '-t', pane, 'Enter']);
        expect(await waitForFileContent(status, { description: 'refused launch' })).toBe('1');
        expect(fs.readFileSync(`${status}.err`, 'utf8')).toContain(
          'outside the supported channel range'
        );
        expect(fs.existsSync(marker), 'nothing was launched').toBe(false);
        expect(channelFiles(fixture)).toEqual([]);
      });
    },
    60_000
  );

  it('a newer provider build is accepted with an advisory, and its handshake decides', async () => {
    await withE2EFixture(async (fixture) => {
      // Newer within the major line: it launches, says so, and a completed
      // handshake delivers exactly like the tested build.
      const newer = start(fixture, 'Newer', {
        channel: true,
        env: { MOCK_VERSION: '2.1.999 (Claude Code)' },
      });
      await fixture.waitForCapture(
        (output) => output.includes('Claude Code 2.1.999 has not been tested'),
        newer.pane
      );
      await ready(fixture, newer, 'Newer');
      const completed = await talk(fixture, 'Newer', 'newer build', ['--timeout', '20s']);
      expect(completed.code, completed.stderr || completed.stdout).toBe(0);
      expect(completed.json).toMatchObject({ status: 'completed', deliveryState: 'uncertain' });
      expect(named(newer, 'paste')).toEqual([]);

      // A newer build whose handshake never completes stays not ready and is
      // never pasted to.
      const silent = start(fixture, 'Changed', {
        channel: true,
        env: { MOCK_VERSION: '2.7.0 (Claude Code)', MOCK_HANDSHAKE: 'never' },
      });
      await fixture.waitForCapture(
        (output) => output.includes('Claude Code 2.7.0 has not been tested'),
        silent.pane
      );
      await waitForEvent(fixture, silent, 'initialize-result');
      await waitForRunning(fixture, silent, 'Changed');
      const refused = await talk(fixture, 'Changed', 'anyone there', ['--detach']);
      expect(refused.code).toBe(1);
      expect(failureCode(refused)).toBe('CHANNEL_NOT_READY');
      expect(named(silent, 'channel')).toEqual([]);
      expect(named(silent, 'paste')).toEqual([]);
      for (const session of [newer, silent]) expect(await quit(session)).toBe('0');
    });
  }, 90_000);
});
