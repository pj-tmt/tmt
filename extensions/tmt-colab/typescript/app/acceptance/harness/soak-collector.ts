import net from 'node:net';
import path from 'node:path';
import type { BrowserContext, Page, TestInfo } from '@playwright/test';
import { assertPrivateSocket } from '../../../../../../typescript/test/support/private-tmux.js';
import { loadSoakDictionary, policy } from './soak-literals.mjs';
import { projectSoakFailure, readSoakTail } from './soak-diagnostics.js';

// The collector consumes only owned observations; it does not depend on scenario/browser helpers.
interface SoakWorld {
  root: string;
  dataRoot: string;
  enableSoakDiagnostics(): void;
  onFailure(observe: () => Promise<void>): void;
  soakLogs(): { records: Record<string, unknown>[]; truncated: boolean };
  soakChildren(): {
    role: string;
    alive: boolean;
    exitCode: number | null;
    signal: string | null;
    stderr: string;
  }[];
}
interface SoakDoor {
  origin: string;
  remote: { child: { exitCode: number | null } };
}

/** One bounded connection probe; all clients close, including on timeout. */
function accepts(options: net.NetConnectOpts): Promise<boolean> {
  return new Promise((resolve) => {
    const client = net.connect(options);
    const finish = (result: boolean) => {
      client.destroy();
      resolve(result);
    };
    client.setTimeout(200, () => finish(false));
    client.once('connect', () => finish(true));
    client.once('error', () => finish(false));
  });
}

/** Observes only the synthetic world. Public fields are reconstructed by the shared projector. */
export class SoakCollector {
  private readonly dictionary = loadSoakDictionary();
  private readonly originMs = Date.now();
  private readonly world: SoakWorld;
  private readonly info: TestInfo;
  private page?: Page;
  private door?: SoakDoor;
  private assertion = 'setup';
  private readonly logs: Record<string, unknown>[] = [];
  private truncated = false;

  constructor(world: SoakWorld, info: TestInfo) {
    this.world = world;
    this.info = info;
    world.enableSoakDiagnostics();
    world.onFailure(() => this.capture());
  }

  phase(assertion: string, page?: Page) {
    if (!policy.assertions.includes(assertion)) throw new Error('Unknown soak assertion');
    this.assertion = assertion;
    this.page = page;
  }

  setDoor(door: SoakDoor) {
    this.door = door;
  }

  observe(context: BrowserContext) {
    context.on('page', (page) => {
      page.on('pageerror', (error) => this.error('colab', { message: error.message }));
      page.on('websocket', (socket) =>
        socket.on('framereceived', ({ payload }) => {
          // Read only JSON protocol error envelopes, never retain network payloads.
          if (typeof payload !== 'string' || Buffer.byteLength(payload) > policy.tailBytes) return;
          try {
            const frame = JSON.parse(payload);
            if (frame && typeof frame === 'object') {
              if (frame.type === 'error') this.error('colab', frame);
              if (frame.error && typeof frame.error === 'object') this.error('remote', frame.error);
            }
          } catch {
            /* Binary and non-JSON frames carry no retained diagnostic. */
          }
        }),
      );
    });
  }

  private error(
    source: string,
    value: { code?: unknown; message?: unknown },
    atMs = Date.now() - this.originMs,
  ) {
    const code =
      typeof value.code === 'string' && this.dictionary.codes.has(value.code) ? value.code : null;
    const message =
      typeof value.message === 'string' && this.dictionary.messages.has(value.message)
        ? value.message
        : null;
    if (!code && !message) return;
    this.logs.push({ source, event: 'error', code, message, atMs });
    if (this.logs.length > 256) {
      this.logs.shift();
      this.truncated = true;
    }
  }

  private async capture() {
    let reference: string | null = null;
    if (this.page && !this.page.isClosed()) {
      reference = await this.page
        .locator('[data-failure-reference]')
        .first()
        .getAttribute('title', { timeout: 200 })
        .catch(() => null);
    }
    const observed = this.world.soakLogs();
    for (const value of observed.records)
      this.error(String(value.source), value, Number(value.atMs) - this.originMs);
    this.truncated ||= observed.truncated;
    const children = this.world.soakChildren();
    const native = readSoakTail(this.world.root, 'soak-native.stderr');
    this.truncated ||= native.truncated;
    const invocations: Record<string, unknown>[] = [];
    for (const line of native.lines) {
      try {
        const value = JSON.parse(line);
        const atMs = value.atMs - this.originMs;
        if (value.event === 'error') this.error(value.source, value, atMs);
        if (value.event === 'exit') this.logs.push({ source: value.source, event: 'exit', atMs });
        if (value.event === 'exit')
          invocations.push({
            role: value.source,
            alive: false,
            exitCode: value.exitCode,
            signal: value.signal,
          });
      } catch {
        /* The optional private log is not an error authority. */
      }
    }
    // These exact names come only from children this world created, not attachments or report input.
    for (const child of children) {
      const tail = readSoakTail(this.world.root, child.stderr);
      this.truncated ||= tail.truncated;
      for (const line of tail.lines) {
        try {
          const entry = JSON.parse(line);
          // Only explicit native timestamped records can enter a historical timeline.
          if (Number.isSafeInteger(entry.atMs))
            this.error(child.role, entry.error ?? entry, entry.atMs - this.originMs);
        } catch {
          /* Untimestamped human log text is not invented timeline evidence. */
        }
      }
    }
    const socket = path.join(this.world.dataRoot, 'colab', 'door.sock');
    assertPrivateSocket(socket, this.world.root);
    const endpoint = this.door && new URL(this.door.origin);
    const [socketAlive, doorAlive] = await Promise.all([
      accepts({ path: socket }),
      endpoint &&
      endpoint.protocol === 'http:' &&
      endpoint.hostname === '127.0.0.1' &&
      /^\d+$/.test(endpoint.port) &&
      this.door!.remote.child.exitCode === null
        ? accepts({ host: '127.0.0.1', port: Number(endpoint.port) })
        : Promise.resolve(false),
    ]);
    const lastError = [...this.logs].reverse().find((record) => record.event === 'error');
    const error =
      reference !== null || this.page
        ? { code: reference, message: reference }
        : { code: lastError?.code, message: lastError?.message };
    if (reference !== null) this.error('colab', error);
    const projected = projectSoakFailure(
      {
        version: 1,
        scenario: policy.id,
        observedMs: Date.now() - this.originMs,
        assertion: this.assertion,
        error,
        logs: this.logs,
        children: [...children, ...invocations.slice(-16)],
        spans: [],
        alive: { socket: socketAlive, door: doorAlive },
        truncated: { input: this.truncated },
      },
      this.dictionary,
    );
    if (projected)
      await this.info.attach(policy.attachment, {
        contentType: 'application/json',
        // originMs is private reporter input, discarded by the independent public reconstruction.
        body: Buffer.from(JSON.stringify({ ...projected, originMs: this.originMs })),
      });
  }
}
