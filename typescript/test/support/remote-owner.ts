import assert from 'node:assert/strict';
import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import fs from 'node:fs';
import http from 'node:http';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import Database from 'better-sqlite3';
import type { CliExecutable } from './cli-executable.mjs';
import { writeExecutable } from './executable-fixture.mjs';
import {
  object,
  text,
  fingerprintWords,
  RemoteDevice,
  requestEnvelope,
  verifyResponse,
  type PairedDevice,
  type RemoteDescriptor,
  type RemoteEnvelope,
} from './remote-device.js';

// The scenario owns E2EFixture. Shared support consumes its explicit resource
// coordinates instead of importing a suite or creating a second tmux lifetime.
export interface RemoteFixture {
  readonly root: string;
  readonly globalDir: string;
  readonly socketRoot: string;
  readonly socket: string;
  readonly socketPath: string;
  readonly serverPid: number;
  readonly pane: string;
  readonly wrapperDir: string;
  readonly workspace: string;
  readonly executables: { readonly cli: CliExecutable };
  paneSessionId(pane?: string): string;
}
export interface CoreCall {
  operation: string | null;
  operationId: string | null;
  argv: string[];
  wrapperPid: number;
  corePid: number | null;
}
interface Exit {
  code: number | null;
  signal: NodeJS.Signals | null;
}
function alive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    if (object(error).code === 'ESRCH') return false;
    throw error;
  }
}
async function until(predicate: () => boolean, description: string): Promise<void> {
  const deadline = Date.now() + 5000;
  while (!predicate()) {
    if (Date.now() >= deadline) throw new Error(`Timed out: ${description}`);
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
}
async function bounded<T>(promise: Promise<T>, description: string, timeoutMs = 5000): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      promise,
      new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(new Error(`Timed out: ${description}`)), timeoutMs);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}
class RemoteProcess {
  readonly child: ChildProcessWithoutNullStreams;
  readonly result: Promise<Exit>;
  readonly pid: number;
  readonly lines: string[] = [];
  private bytes = 0;
  private pending = '';
  private failure: Error | undefined;
  private observers = new Set<() => void>();
  constructor(executable: string, args: string[], cwd: string, env: NodeJS.ProcessEnv) {
    this.child = spawn(executable, args, {
      cwd,
      env,
      detached: true,
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    this.pid = this.child.pid ?? 0;
    this.child.stdin.on('error', (error) => {
      this.failure = error;
      this.changed();
    });
    this.child.stdout.setEncoding('utf8');
    this.child.stdout.on('data', (chunk: string) => {
      if (!this.charge(chunk)) return;
      this.pending += chunk;
      const lines = this.pending.split('\n');
      this.pending = lines.pop() ?? '';
      this.lines.push(...lines.filter(Boolean));
      this.changed();
    });
    this.child.stderr.setEncoding('utf8');
    this.child.stderr.on('data', (chunk: string) => {
      if (!this.charge(chunk)) return;
      // Keep diagnostics in the owned root, never in a successful response.
      fs.appendFileSync(path.join(cwd, `remote-process-${this.pid}.stderr`), chunk);
    });
    this.result = new Promise((resolve) => {
      this.child.once('error', (error) => {
        this.failure = error;
        this.changed();
        resolve({ code: 1, signal: null });
      });
      this.child.once('close', (code, signal) => {
        this.changed();
        resolve({ code, signal });
      });
    });
  }
  private charge(chunk: string): boolean {
    this.bytes += Buffer.byteLength(chunk);
    if (this.bytes <= 24 * 1024 * 1024) return true;
    this.failure = new Error('Remote test process exceeded its output bound');
    this.kill('SIGKILL');
    this.changed();
    return false;
  }
  private changed(): void {
    for (const observe of this.observers) observe();
  }
  event(predicate: (value: Record<string, unknown>) => boolean): Promise<Record<string, unknown>> {
    let observe: () => void;
    const found = new Promise<Record<string, unknown>>((resolve, reject) => {
      observe = () => {
        if (this.failure) {
          reject(this.failure);
          return;
        }
        try {
          for (const line of this.lines) {
            const value = object(JSON.parse(line));
            if (predicate(value)) {
              resolve(value);
              return;
            }
          }
          if (this.child.exitCode !== null || this.child.signalCode !== null)
            reject(new Error(`Remote process exited before expected event; PID ${this.pid}`));
        } catch (error) {
          reject(error);
        }
      };
      this.observers.add(observe);
      observe();
    });
    return bounded(found, `Remote event from PID ${this.pid}`).finally(() =>
      this.observers.delete(observe)
    );
  }
  kill(signal: NodeJS.Signals): void {
    if (!this.pid) return;
    try {
      process.kill(-this.pid, signal);
    } catch (error) {
      if (object(error).code !== 'ESRCH') throw error;
    }
  }
  async stop(): Promise<void> {
    this.kill('SIGTERM');
    try {
      await bounded(this.result, `Remote PID ${this.pid} close`);
    } catch {
      this.kill('SIGKILL');
      await bounded(this.result, `Remote PID ${this.pid} forced close`);
    }
    // A direct-child exit is insufficient: reject any surviving owned group.
    if (this.pid && alive(-this.pid)) {
      this.kill('SIGKILL');
      throw new Error(`Remote process group survived direct close: ${this.pid}`);
    }
    assert(!this.pid || !alive(this.pid), 'Remote direct child was reaped');
  }
}
export interface HttpReply {
  status: number;
  body: unknown;
  rawBody: string;
  headers: http.IncomingHttpHeaders;
}
/** One framed, bounded request; no cookie, ambient credential or SDK retry. */
export function post(url: string, input: unknown, origin?: string): Promise<HttpReply> {
  const body = Buffer.from(JSON.stringify(input));
  return new Promise((resolve, reject) => {
    const request = http.request(url, {
      method: 'POST',
      agent: false,
      headers: {
        'Content-Type': 'application/json',
        'Content-Length': body.length,
        Connection: 'close',
        ...(origin === undefined ? {} : { Origin: origin }),
      },
    });
    let bytes = 0;
    const chunks: Buffer[] = [];
    const timer = setTimeout(() => request.destroy(new Error('Remote HTTP deadline')), 20_000);
    request.on('error', (error) => {
      clearTimeout(timer);
      reject(error);
    });
    request.on('response', (response) => {
      response.on('data', (chunk: Buffer) => {
        bytes += chunk.length;
        if (bytes > 24 * 1024 * 1024) request.destroy(new Error('Remote HTTP output bound'));
        else chunks.push(chunk);
      });
      response.on('error', (error) => {
        clearTimeout(timer);
        reject(error);
      });
      response.on('aborted', () => {
        clearTimeout(timer);
        reject(new Error('Remote HTTP response aborted'));
      });
      response.on('end', () => {
        clearTimeout(timer);
        try {
          const rawBody = Buffer.concat(chunks).toString('utf8');
          resolve({
            status: response.statusCode ?? 0,
            body: rawBody === '' ? null : JSON.parse(rawBody),
            rawBody,
            headers: response.headers,
          });
        } catch (error) {
          reject(error);
        }
      });
    });
    request.end(body);
  });
}
export function dispatchIntent(operationId: string, recipientId: string, message: string) {
  return {
    version: 1,
    operation: 'dispatch.create',
    originator: 'anonymous',
    input: { operationId, recipientIds: [recipientId], message, kind: 'request' },
  };
}
export class RemoteSession {
  private next = 1n;
  private observed = 0n;
  constructor(
    readonly owner: RemoteOwner,
    readonly device: RemoteDevice,
    readonly paired: PairedDevice,
    readonly id: string,
    readonly windowId: string,
    openingSequence: string
  ) {
    this.observed = BigInt(openingSequence);
  }
  envelope(
    operation: string,
    input: unknown,
    options: {
      sequence?: string;
      id?: string;
      kind?: 'request' | 'control';
      payload?: Buffer;
    } = {}
  ): RemoteEnvelope {
    const sequence = options.sequence ?? String(this.next++);
    return requestEnvelope(
      this.device,
      this.paired,
      this.windowId,
      this.id,
      sequence,
      operation,
      input,
      options
    );
  }
  append(operation: string, input: unknown, id?: string): Promise<Record<string, unknown>> {
    return this.exchange('append', this.envelope(operation, input, id === undefined ? {} : { id }));
  }
  async exchange(
    route: 'append' | 'subscribe' | 'ack',
    request: RemoteEnvelope
  ): Promise<Record<string, unknown>> {
    const reply = await this.owner.post(route, request);
    assert([200, 202].includes(reply.status), `signed response HTTP status ${reply.status}`);
    const response = verifyResponse(reply.body, request, this.paired);
    assert(
      BigInt(response.envelope.sequence) > this.observed,
      'increasing machine response sequence'
    );
    this.observed = BigInt(response.envelope.sequence);
    return response.payload;
  }
}
export class RemoteOwner {
  readonly directory: string;
  readonly remoteDatabase: string;
  readonly coreTrace: string;
  private readonly env: NodeJS.ProcessEnv;
  private readonly processes = new Set<RemoteProcess>();
  private readonly requests = new Set<Promise<HttpReply>>();
  private serve: RemoteProcess | undefined;
  private crashedGroup: number | undefined;
  private barrierId: string | undefined;
  private address = '';
  private machineId = '';
  private windowId = '';
  private stoppedAddress = '';
  constructor(readonly fixture: RemoteFixture) {
    this.directory = path.join(fixture.root, 'remote-owner');
    this.remoteDatabase = path.join(fixture.globalDir, 'remote', 'remote.db');
    this.coreTrace = path.join(this.directory, 'core-calls.jsonl');
    fs.mkdirSync(this.directory, { recursive: true });
    const home = path.join(this.directory, 'home');
    fs.mkdirSync(home);
    this.env = {
      PATH: `${fixture.wrapperDir}:${path.dirname(process.execPath)}:/usr/local/bin:/usr/bin:/bin`,
      HOME: home,
      XDG_CONFIG_HOME: path.join(home, 'config'),
      XDG_DATA_HOME: path.join(home, 'data'),
      XDG_STATE_HOME: path.join(home, 'state'),
      XDG_CACHE_HOME: path.join(home, 'cache'),
      CODEX_HOME: path.join(home, 'codex'),
      TMPDIR: this.directory,
      LANG: 'C.UTF-8',
      TMT_HOME: fixture.globalDir,
      TMUX_TMPDIR: fixture.socketRoot,
      TMT_E2E_SOCKET: fixture.socket,
      TMUX: `${fixture.socketPath},${fixture.serverPid},${fixture.paneSessionId()}`,
      TMUX_PANE: fixture.pane,
      TMT_EXECUTABLE: this.coreWrapper(),
    };
  }
  private coreWrapper(): string {
    const script = path.join(this.directory, 'core-wrapper.mjs');
    // Exact input/output forwarding around the selected real executable. The
    // wrapper records actual launches; it never supplies an API result/state.
    const descriptor = this.fixture.executables.cli;
    writeExecutable(
      script,
      `import { spawn } from 'node:child_process';
import fs from 'node:fs';
const descriptor = ${JSON.stringify(descriptor)};
const directory = ${JSON.stringify(this.directory)};
const trace = ${JSON.stringify(this.coreTrace)};
const argv = [...descriptor.args, ...process.argv.slice(2)];
const input = fs.readFileSync(0);
let operation = null, operationId = null;
if (process.argv[2] === 'api') {
  try { const value = JSON.parse(input.toString('utf8')); operation = value.operation; operationId = value.input?.operationId ?? null; } catch {}
}
fs.appendFileSync(trace, JSON.stringify({ operation, operationId, argv, wrapperPid: process.pid, corePid: null }) + '\\n');
let barrier;
try { barrier = JSON.parse(fs.readFileSync(directory + '/barrier.json', 'utf8')); } catch {}
const gated = operation === 'dispatch.create' && operationId === barrier?.operationId;
async function gate(phase) {
  if (!gated || phase !== barrier.phase) return;
  fs.writeFileSync(directory + '/barrier-entered.json', JSON.stringify({ phase, operationId, pid: process.pid }));
  await new Promise((resolve, reject) => {
    const release = directory + '/barrier-release';
    const watcher = fs.watch(directory, check);
    const timer = setTimeout(() => { watcher.close(); reject(new Error('Crash barrier deadline')); }, 30000);
    function check() { if (fs.existsSync(release)) { clearTimeout(timer); watcher.close(); resolve(); } }
    check();
  });
}
await gate('before');
const child = spawn(descriptor.executable, argv, { stdio: ['pipe', gated ? 'pipe' : 'inherit', 'inherit'] });
fs.writeFileSync(directory + '/' + process.pid + '.core.json', JSON.stringify({ pid: child.pid }));
const chunks = [];
let bytes = 0;
if (gated) child.stdout.on('data', chunk => {
  bytes += chunk.length;
  if (bytes > 16 * 1024 * 1024) { child.kill('SIGKILL'); throw new Error('Actual core output exceeded bound'); }
  chunks.push(chunk);
});
child.stdin.on('error', () => {});
child.stdin.end(input);
const exit = await new Promise((resolve, reject) => {
  child.once('error', reject);
  child.once('close', (code, signal) => resolve({ code, signal }));
});
if (gated) {
  // These are the real selected core's bytes, retained before forwarding. The
  // scenario independently checks its durable receipt and actual agent effect.
  fs.writeFileSync(directory + '/barrier-core-output', Buffer.concat(chunks));
  await gate('after');
  process.stdout.write(Buffer.concat(chunks));
}
if (exit.signal) process.kill(process.pid, exit.signal);
else process.exitCode = exit.code ?? 1;
`,
      0o600
    );
    const executable = path.join(this.directory, 'core-wrapper');
    const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;
    writeExecutable(
      executable,
      `#!/bin/sh\nexec ${quote(process.execPath)} ${quote(script)} "$@"\n`
    );
    return executable;
  }
  coreCalls(): CoreCall[] {
    if (!fs.existsSync(this.coreTrace)) return [];
    return fs
      .readFileSync(this.coreTrace, 'utf8')
      .split('\n')
      .filter(Boolean)
      .map((line) => {
        const call = JSON.parse(line) as CoreCall;
        const child = path.join(this.directory, `${call.wrapperPid}.core.json`);
        if (fs.existsSync(child))
          call.corePid = Number(object(JSON.parse(fs.readFileSync(child, 'utf8'))).pid);
        return call;
      });
  }
  armCrash(operationId: string, phase: 'before' | 'after'): void {
    assert.equal(this.barrierId, undefined, 'one crash barrier');
    this.barrierId = operationId;
    fs.writeFileSync(
      path.join(this.directory, 'barrier.json'),
      JSON.stringify({ operationId, phase })
    );
  }
  async crashBarrier(): Promise<Record<string, unknown>> {
    const file = path.join(this.directory, 'barrier-entered.json');
    await until(
      () => fs.existsSync(file) && fs.readFileSync(file, 'utf8').length > 0,
      'real-core crash barrier'
    );
    const entered = object(JSON.parse(fs.readFileSync(file, 'utf8')));
    assert.equal(entered.operationId, this.barrierId);
    return entered;
  }
  barrierCoreOutput(): Record<string, unknown> {
    return object(
      JSON.parse(fs.readFileSync(path.join(this.directory, 'barrier-core-output'), 'utf8'))
    );
  }
  /** Kill only serve: the real owned invocation must retain its lease. */
  async crash(): Promise<void> {
    assert(this.serve, 'serve running for crash');
    const serve = this.serve;
    this.stoppedAddress = this.address;
    this.crashedGroup = serve.pid;
    process.kill(serve.pid, 'SIGKILL');
    const exit = await bounded(serve.result, 'crashed serve joined');
    assert.equal(exit.signal, 'SIGKILL');
    this.processes.delete(serve);
    this.serve = undefined;
    await this.assertListenerClosed();
  }
  async assertRestartBlocked(): Promise<void> {
    const attempt = this.launch(['serve', '--json']);
    try {
      const exit = await bounded(attempt.result, 'restart refused while invocation owns lease');
      assert.notEqual(exit.code, 0);
      const replies = attempt.lines.map((line) => object(JSON.parse(line)));
      assert(replies.some((reply) => object(reply.error).code === 'REMOTE_ALREADY_SERVING'));
    } finally {
      await attempt.stop();
      this.processes.delete(attempt);
    }
  }
  /** Reap the invocation before restart, leaving its original durable ID intact. */
  async finishCrash(): Promise<void> {
    assert(this.barrierId, 'armed crash barrier');
    for (const call of this.coreCalls().filter((call) => call.operationId === this.barrierId)) {
      if (alive(call.wrapperPid)) {
        try {
          process.kill(-call.wrapperPid, 'SIGKILL');
        } catch (error) {
          if (object(error).code !== 'ESRCH') throw error;
        }
      }
      await until(
        () => !alive(call.wrapperPid) && (!call.corePid || !alive(call.corePid)),
        'crash invocation reaped'
      );
    }
    this.barrierId = undefined;
    for (const file of ['barrier.json', 'barrier-entered.json', 'barrier-release'])
      fs.rmSync(path.join(this.directory, file), { force: true });
    if (this.crashedGroup) {
      assert(!alive(-this.crashedGroup), 'crashed serve group gone');
      this.crashedGroup = undefined;
    }
  }
  async approval(operationId: string): Promise<{
    held: Record<string, unknown>;
    finish: (answer: 'confirm' | 'refuse' | 'eof') => Promise<Record<string, unknown>>;
  }> {
    const child = this.launch(['approve', operationId, '--json']);
    const held = await child.event((event) => event.event === 'held');
    return {
      held,
      finish: async (answer) => {
        child.child.stdin.end(answer === 'eof' ? undefined : JSON.stringify({ op: answer }) + '\n');
        try {
          const ended = await child.event((event) => event.event === 'ended');
          assert.equal((await bounded(child.result, 'local approval exit')).code, 0);
          return ended;
        } finally {
          await child.stop();
          this.processes.delete(child);
        }
      },
    };
  }
  async cancel(operationId: string): Promise<Record<string, unknown>> {
    const child = this.launch(['cancel', operationId, '--json']);
    try {
      assert.equal((await bounded(child.result, 'local cancellation exit')).code, 0);
      assert.equal(child.lines.length, 1);
      return object(JSON.parse(child.lines[0]!));
    } finally {
      await child.stop();
      this.processes.delete(child);
    }
  }
  private launch(args: string[]): RemoteProcess {
    const executable = fileURLToPath(
      new URL('../../../rust/target/debug/tmt-remote', import.meta.url)
    );
    fs.accessSync(executable, fs.constants.X_OK);
    const child = new RemoteProcess(executable, args, this.directory, this.env);
    this.processes.add(child);
    return child;
  }
  async start(): Promise<void> {
    assert.equal(this.serve, undefined, 'one owned serve');
    this.serve = this.launch(['serve', '--json']);
    const descriptor = await this.serve.event((value) => typeof value.address === 'string');
    this.address = text(descriptor.address);
    this.machineId = text(descriptor.machineId);
    const url = new URL(this.address);
    assert.equal(url.hostname, '127.0.0.1');
    this.windowId = text(descriptor.windowId);
  }
  post(route: 'append' | 'subscribe' | 'ack' | 'pair', input: unknown): Promise<HttpReply> {
    assert(this.serve, 'serve must run before HTTP');
    const request = post(`${this.address}/${route}`, input);
    this.requests.add(request);
    void request.then(
      () => this.requests.delete(request),
      () => this.requests.delete(request)
    );
    return request;
  }
  async pair(device = new RemoteDevice()): Promise<{ device: RemoteDevice; paired: PairedDevice }> {
    const owner = this.launch(['pair', '--json']);
    try {
      const offer = await owner.event((event) => event.event === 'offer');
      const descriptor = object(offer.descriptor) as unknown as RemoteDescriptor;
      assert.equal(descriptor.machineId, this.machineId);
      assert.equal(descriptor.windowId, this.windowId);
      const enrollment = device.enrollment(descriptor, text(offer.code));
      // Subscribe to rejection immediately so an early HTTP failure is joined too.
      const submitted = this.post('pair', enrollment.body);
      const candidate = owner.event((event) => event.event === 'candidate');
      const response = await (async () => {
        const proposed = await Promise.race([
          candidate,
          submitted.then((reply) => {
            if (reply.status !== 200) throw new Error(`Pairing refused: ${reply.status}`);
            return candidate;
          }),
        ]);
        assert.equal(proposed.kind, 'cli');
        assert.equal(proposed.origin, device.origin);
        assert.equal(proposed.name, device.name);
        assert.deepEqual(proposed.words, fingerprintWords(device.publicBytes));
        owner.child.stdin.end('confirm\n');
        return submitted;
      })();
      const ended = await owner.event((event) => event.event === 'ended');
      assert.equal(ended.reason, 'paired');
      assert.equal(response.status, 200);
      const paired = enrollment.accept(response.body);
      assert.equal(ended.clientId, paired.clientId);
      assert.equal((await bounded(owner.result, 'pair owner exit')).code, 0);
      return { device, paired };
    } finally {
      await owner.stop();
      this.processes.delete(owner);
    }
  }
  opening(device: RemoteDevice, paired: PairedDevice): RemoteEnvelope {
    return requestEnvelope(
      device,
      paired,
      this.windowId,
      'new',
      '0',
      'session.open',
      { clientNonce: randomBytes(16).toString('hex') },
      { kind: 'control' }
    );
  }
  async session(device: RemoteDevice, paired: PairedDevice): Promise<RemoteSession> {
    const request = this.opening(device, paired);
    const reply = await this.post('append', request);
    assert.equal(reply.status, 200);
    const response = verifyResponse(reply.body, request, paired);
    const id = text(response.payload.sessionId);
    assert.equal(response.envelope.sessionId, id);
    return new RemoteSession(this, device, paired, id, this.windowId, response.envelope.sequence);
  }
  async stop(): Promise<void> {
    if (!this.serve) return;
    const serve = this.serve;
    this.stoppedAddress = this.address;
    await serve.stop();
    this.processes.delete(serve);
    this.serve = undefined;
    assert.equal(
      fs.existsSync(path.join(this.fixture.globalDir, 'remote', 'control.sock')),
      false,
      'Remote control socket removed after joined stop'
    );
    await this.assertListenerClosed();
  }
  async assertListenerClosed(): Promise<void> {
    if (!this.stoppedAddress) return;
    const url = new URL(this.stoppedAddress);
    await bounded(
      new Promise<void>((resolve, reject) => {
        const socket = net.connect({ host: '127.0.0.1', port: Number(url.port) });
        socket.once('connect', () => {
          socket.destroy();
          reject(new Error('Remote listener survived stop'));
        });
        socket.once('error', (error) => {
          if (object(error).code === 'ECONNREFUSED') resolve();
          else reject(error);
        });
        socket.setTimeout(1000, () => socket.destroy(new Error('Listener absence unconfirmed')));
      }),
      'Remote listener closed'
    );
  }
  /** Explicitly approved fixture seeding; never open Remote DB during serve. */
  seedGrant(
    clientId: string,
    policy: { mode?: 'hold'; agents?: string[]; expiresAtMs?: number }
  ): void {
    assert.equal(this.serve, undefined, 'seed only after joined serve stop');
    assert.equal(this.processes.size, 0, 'no owned Remote child during grant seeding');
    for (const call of this.coreCalls()) {
      assert(
        !alive(call.wrapperPid) && (!call.corePid || !alive(call.corePid)),
        'core invocation exited before seeding'
      );
    }
    const database = new Database(this.remoteDatabase, { fileMustExist: true });
    try {
      const row = object(
        database.prepare('SELECT * FROM grants WHERE client_id = ?').get(clientId)
      );
      const agents =
        policy.agents === undefined ? text(row.agents) : JSON.stringify([...policy.agents].sort());
      assert.equal(
        database
          .prepare(
            `UPDATE grants SET mode = ?, agents = ?, expires_at_ms = ?,
        revision = revision + 1 WHERE client_id = ?`
          )
          .run(policy.mode ?? row.mode, agents, policy.expiresAtMs ?? row.expires_at_ms, clientId)
          .changes,
        1
      );
    } finally {
      database.close();
    }
  }
  async dispose(): Promise<void> {
    const errors: unknown[] = [];
    try {
      await this.stop();
    } catch (error) {
      errors.push(error);
    }
    // Invoke children have their own process groups. Serve's death alone is
    // insufficient; stop every recorded actual invocation before releasing roots.
    for (const call of this.coreCalls()) {
      try {
        if (alive(call.wrapperPid) || (call.corePid && alive(call.corePid))) {
          try {
            process.kill(-call.wrapperPid, 'SIGKILL');
          } catch (error) {
            if (object(error).code !== 'ESRCH') throw error;
          }
          await until(
            () => !alive(call.wrapperPid) && (!call.corePid || !alive(call.corePid)),
            'owned invocation teardown'
          );
        }
      } catch (error) {
        errors.push(error);
      }
    }
    if (this.barrierId) {
      try {
        await this.finishCrash();
      } catch (error) {
        errors.push(error);
      }
    }
    for (const child of this.processes) {
      try {
        await child.stop();
      } catch (error) {
        errors.push(error);
      }
    }
    this.processes.clear();
    try {
      await bounded(Promise.allSettled(this.requests), 'Remote HTTP requests joined');
    } catch (error) {
      errors.push(error);
    }
    for (const call of this.coreCalls()) {
      if (alive(call.wrapperPid) || (call.corePid && alive(call.corePid)))
        errors.push(
          new Error(`Core invocation survived Remote teardown: ${call.wrapperPid}/${call.corePid}`)
        );
    }
    if (errors.length) {
      const evidence = fs.mkdtempSync(path.join(os.tmpdir(), 'tmt-1055-remote-cleanup-'));
      fs.cpSync(this.directory, path.join(evidence, 'owner'), { recursive: true });
      throw new AggregateError(errors, `Remote cleanup failed; retained evidence: ${evidence}`);
    }
  }
}
