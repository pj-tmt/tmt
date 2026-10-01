import Database from 'better-sqlite3';
import { execFileSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { expectJsonResult } from './cli-assertions.js';
import { withE2EFixture, type CliResult, type E2EFixture } from './harness.js';
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

/**
 * A short-lived `sh` writes the executable, so this process never holds a write
 * descriptor that a concurrent fork could carry into the exec of the file (the
 * ETXTBSY race; see DEVELOPMENT.md, "ETXTBSY").
 */
function writeExecutable(file: string, script: string): void {
  execFileSync('/bin/sh', ['-c', 'cat > "$1" && chmod 755 "$1"', 'sh', file], { input: script });
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
  options: { channel: boolean; env?: Record<string, string> }
): Session {
  const pane = fixture.createShellPane(`claude-${name}`).pane;
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
  const command = [
    'env',
    ...Object.entries(env).map(([key, value]) => `${key}=${value}`),
    fixture.executables.cli.executable,
    ...fixture.executables.cli.args,
    'run',
    ...(options.channel ? ['--channel'] : []),
    '-s',
    name,
    launcher(fixture),
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
  generation: string;
  launchOwner: { pid: number; start: string };
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
  await waitForEvent(fixture, session, session.channel ? 'initialized-sent' : 'started');
  await waitForRunning(fixture, session, name);
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

function leakedServers(fixture: E2EFixture): string[] {
  return execFileSync('ps', ['-axo', 'command='], { encoding: 'utf8' })
    .split('\n')
    .filter((line) => line.includes('__channel-server') && line.includes(fixture.root));
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
          `UPDATE identity_session_preferences SET preferred_harness = NULL, remembered_harness = NULL,
             runtime_mode = NULL, provider_session_id = NULL
           WHERE identity_id = ?`
        )
        .run(id).changes
    ).toBe(1);
  });
}

describe.sequential('Claude channel delivery', () => {
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

  it('an enrollment is authoritative before the launch is admitted or any harness is preferred', async () => {
    await withE2EFixture(async (fixture) => {
      const worker = start(fixture, 'Paused', { channel: true });
      await waitForEvent(fixture, worker, 'initialized-sent');
      await waitForReady(fixture, 1);
      await waitForRunning(fixture, worker, 'Paused');
      // `tmt run` binds, enrolls and spawns, and only then records the launch and
      // the preferred harness. Recreate the window between enroll and admission.
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

  it('a reply notification reaches an opted-in originator through its channel and never by paste', async () => {
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

      const trace = installTmuxTrace(fixture);
      const asBoss = await talk(fixture, 'Worker', 'boss asks', ['--detach'], boss.pane);
      const asPlain = await talk(fixture, 'Worker', 'plain asks', ['--detach'], plain.pane);
      expect(asBoss.code, asBoss.stderr || asBoss.stdout).toBe(0);
      expect(asPlain.code, asPlain.stderr || asPlain.stdout).toBe(0);

      // The never-opted-in originator keeps the baseline: its notification is a paste.
      await fixture.waitFor(
        () => named(plain, 'paste').some((line) => String(line.line).includes('reply from Worker')),
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
      expect(contents(boss)[0]).toContain('reply from Worker');
      expect(String(named(boss, 'hint-response')[0].body)).toContain('channel-ok');
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
  }, 90_000);

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
        () => contents(boss).some((text) => text.includes('no reply yet from Idle')),
        20_000,
        'timeout notification through the channel'
      );
      expect(named(boss, 'paste')).toEqual([]);

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
      expect(contents(boss).filter((text) => text.includes('reply from Idle'))).toHaveLength(1);
      expect(String(named(boss, 'hint-response')[0].body)).toContain('late answer');

      expect(named(boss, 'paste')).toEqual([]);
      expect(terminalWrites(trace), 'no tmux write for an opted-in originator').toEqual([]);
      expect(await quit(boss)).toBe('0');
    });
  }, 90_000);

  it('an originator that is not ready or unreachable gets no paste and no resend, and the durable reply is accepted', async () => {
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
      for (const session of [silent, orphan, worker]) expect(await quit(session)).toBe('0');
    });
  }, 90_000);

  it('a direct dispatch to an opted-in session uses the channel, or reports it unavailable, never paste', async () => {
    await withE2EFixture(async (fixture) => {
      const worker = start(fixture, 'Worker', { channel: true, env: { MOCK_AUTOREPLY: '0' } });
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
      const dispatch = async (recipient: string, message: string) => {
        fs.writeFileSync(
          input,
          JSON.stringify({
            version: 1,
            operation: 'dispatch.create',
            identity: 'Sender',
            input: {
              operationId: randomUUID(),
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
      const live = await dispatch('Worker', 'dispatched to the channel');
      expect(live.wake.status).toBe('uncertain');
      await fixture.waitFor(() => named(worker, 'channel').length === 1, 10_000, 'dispatch');
      // The wake names the request; the work itself stays in the durable inbox.
      expect(contents(worker)[0]).toContain(live.items[0].requestId);

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

  it('refuses to enroll a command whose provider version is not the recorded one', async () => {
    await withE2EFixture(async (fixture) => {
      const pane = fixture.createShellPane('unsupported').pane;
      const status = path.join(fixture.root, 'unsupported.status');
      const marker = path.join(fixture.root, 'unsupported.launched');
      const fake = path.join(fixture.wrapperDir, 'claude');
      writeExecutable(
        fake,
        `#!/bin/sh\nif [ "$1" = --version ]; then echo '2.1.999 (Claude Code)'; exit 0; fi\ntouch ${quote(marker)}\n`
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
  }, 60_000);
});
