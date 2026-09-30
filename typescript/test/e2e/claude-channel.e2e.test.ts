import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { withE2EFixture, type CliResult, type E2EFixture } from './harness.js';
import { waitForFileContent } from './wait-for-file.js';

// Claude channel delivery (#329), against a mock `claude` that plays only the MCP
// client side. The invariants here are the contract's: an opted-in session is
// reached through its channel or not at all (never pasted to), a session that
// never opted in keeps paste, and nothing is left running or on disk afterwards.
// See contracts/claude-channel-v1.md.

const mock = fileURLToPath(new URL('./mock-claude-channel.mjs', import.meta.url));

interface MockEvent {
  event: string;
  [key: string]: unknown;
}

interface Session {
  pane: string;
  log: string;
  status: string;
}

function quote(value: string): string {
  return `'${value.replaceAll("'", "'\\''")}'`;
}

function launcher(fixture: E2EFixture): string {
  const fake = path.join(fixture.wrapperDir, 'claude');
  fs.writeFileSync(fake, `#!/bin/sh\nexec ${quote(process.execPath)} ${quote(mock)} "$@"\n`, {
    mode: 0o700,
  });
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
  const env = { HOME: home, MOCK_CHANNEL_LOG: log, MOCK_AUTOREPLY: '1', ...options.env };
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
  return { pane, log, status };
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

async function waitForEvent(fixture: E2EFixture, session: Session, event: string): Promise<void> {
  await fixture.waitFor(() => named(session, event).length > 0, 15_000, `mock event ${event}`);
}

const channelDirectory = (fixture: E2EFixture) => path.join(fixture.globalDir, 'channels');

function enrollmentFile(fixture: E2EFixture): string {
  const files = fs.readdirSync(channelDirectory(fixture)).filter((file) => file.endsWith('.json'));
  expect(files, 'exactly one enrollment record').toHaveLength(1);
  return path.join(channelDirectory(fixture), files[0]);
}

interface Enrollment {
  generation: string;
  launchOwner: { pid: number; start: string };
  claude: { pid: number; start: string } | null;
}

const enrollment = (fixture: E2EFixture) =>
  JSON.parse(fs.readFileSync(enrollmentFile(fixture), 'utf8')) as Enrollment;

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

async function talk(fixture: E2EFixture, target: string, message: string, extra: string[] = []) {
  return fixture.runJsonCli<Record<string, unknown>>(['talk', target, message, ...extra]);
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

describe.sequential('Claude channel delivery', () => {
  it('delivers to an enrolled session through the channel only, records uncertainty and completes on the durable reply', async () => {
    await withE2EFixture(async (fixture) => {
      const worker = start(fixture, 'Worker', { channel: true });
      await waitForEvent(fixture, worker, 'initialized-sent');
      // The enrollment predates the handshake and names this launch; the server
      // completed it with the mock's own process.
      await fixture.waitFor(() => enrollment(fixture).claude !== null, 15_000, 'ready enrollment');
      const record = enrollment(fixture);
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
      expect(named(worker, 'paste')).toEqual([]);

      // Leaving cleans up: the enrollment, the socket and the server.
      expect(await quit(worker)).toBe('0');
      await fixture.waitFor(
        () =>
          fs.readdirSync(channelDirectory(fixture)).length === 0 &&
          leakedServers(fixture).length === 0,
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
      expect(enrollment(fixture).claude).toBeNull();
      await waitForRunning(fixture, worker, 'Racer');
      const result = await talk(fixture, 'Racer', 'early bird', ['--detach']);
      expect(result.code, result.stderr || result.stdout).toBe(0);
      expect(result.json).toMatchObject({ deliveryState: 'uncertain' });
      expect(enrollment(fixture).claude).not.toBeNull();
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
      const result = await talk(fixture, 'Silent', 'anyone there', ['--detach']);
      expect(result.code).toBe(1);
      expect(failureCode(result)).toBe('CHANNEL_NOT_READY');
      expect(result.stdout).toContain('nothing was pasted');
      expect(named(worker, 'channel')).toEqual([]);
      expect(named(worker, 'paste'), 'no paste for an opted-in session').toEqual([]);
      // The request is retained for a later attempt; nothing was lost.
      expect((result.json as { requestId?: string }).requestId).toMatch(/^req_/);
      expect(await quit(worker)).toBe('0');
    });
  }, 60_000);

  it('a ready channel whose server died is unreachable and is never pasted to', async () => {
    await withE2EFixture(async (fixture) => {
      const worker = start(fixture, 'Orphan', { channel: true });
      await waitForEvent(fixture, worker, 'initialized-sent');
      await fixture.waitFor(() => enrollment(fixture).claude !== null, 15_000, 'ready enrollment');
      await waitForRunning(fixture, worker, 'Orphan');
      fs.writeFileSync(`${worker.log}.kill-server`, '');
      await waitForEvent(fixture, worker, 'server-exit');
      const result = await talk(fixture, 'Orphan', 'still there', ['--detach']);
      expect(result.code).toBe(1);
      expect(failureCode(result)).toBe('CHANNEL_UNREACHABLE');
      expect(named(worker, 'channel')).toEqual([]);
      expect(named(worker, 'paste')).toEqual([]);
      expect(await quit(worker)).toBe('0');
    });
  }, 60_000);

  it('a record that does not match the stored runtime denies without any paste', async () => {
    await withE2EFixture(async (fixture) => {
      const worker = start(fixture, 'Forged', { channel: true });
      await waitForEvent(fixture, worker, 'initialized-sent');
      await fixture.waitFor(() => enrollment(fixture).claude !== null, 15_000, 'ready enrollment');
      await waitForRunning(fixture, worker, 'Forged');
      const file = enrollmentFile(fixture);
      const record = enrollment(fixture);
      // Only the Claude process differs from what the binding recorded.
      fs.writeFileSync(
        file,
        JSON.stringify({ ...record, claude: { ...record.claude, pid: record.claude!.pid + 1 } })
      );
      const result = await talk(fixture, 'Forged', 'who are you', ['--detach']);
      expect(result.code).toBe(1);
      expect(failureCode(result)).toBe('DELIVERY_PREPARATION_FAILED');
      expect(named(worker, 'channel')).toEqual([]);
      expect(named(worker, 'paste')).toEqual([]);
      fs.writeFileSync(file, JSON.stringify(record));
      expect(await quit(worker)).toBe('0');
    });
  }, 60_000);

  it('a session that never opted in keeps paste and has no enrollment', async () => {
    await withE2EFixture(async (fixture) => {
      const worker = start(fixture, 'Plain', { channel: false, env: { MOCK_AUTOREPLY: '0' } });
      await waitForEvent(fixture, worker, 'started');
      await waitForRunning(fixture, worker, 'Plain');
      expect(
        fs.existsSync(channelDirectory(fixture)) ? fs.readdirSync(channelDirectory(fixture)) : []
      ).toEqual([]);
      const result = await talk(fixture, 'Plain', 'plain paste', ['--detach']);
      expect(result.code, result.stderr || result.stdout).toBe(0);
      expect(result.json).not.toHaveProperty('deliveryState');
      expect(result.json).not.toHaveProperty('channelFallback');
      await fixture.waitFor(
        () => named(worker, 'paste').some((line) => String(line.line).includes('plain paste')),
        10_000,
        'the message pasted into the session'
      );
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
      fs.writeFileSync(
        fake,
        `#!/bin/sh\nif [ "$1" = --version ]; then echo '2.1.999 (Claude Code)'; exit 0; fi\ntouch ${quote(marker)}\n`,
        { mode: 0o700 }
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
      expect(fs.existsSync(channelDirectory(fixture))).toBe(false);
    });
  }, 60_000);
});
