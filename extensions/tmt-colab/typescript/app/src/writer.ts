import {
  decodeHeader,
  encodeBinary,
  Envelope,
  exactKeys,
  generatedId,
  requireValue,
} from '@tmt/colab-client';
import { Connection } from './connection.js';
import { type JsonValue, UPDATE_BYTES } from './fold-protocol.js';
import type { ObjectEntry } from './objects.js';
import { recordKey, validateRecord, type AskRoot } from './ask-records.js';
import { record } from './storage.js';
interface SavedWriter {
  pending: { id: string; entry: ObjectEntry } | null;
  completed: string[];
}

/** A lifetime Web Lock owns this device stream. Other tabs relay plaintext
 * update bytes; the leader validates them before signing, and persists exact
 * ciphertext before send. Interrupted appends retry stored bytes, never reseal. */
export class Writer {
  #channel: BroadcastChannel;
  #controller = new AbortController();
  #release!: () => void;
  #leader = false;
  #closed = false;
  #tasks = Promise.resolve();
  #working = new Map<string, Promise<void>>();
  #pending = new Map<
    string,
    { resolve(): void; reject(error: Error): void; timer: ReturnType<typeof setTimeout> }
  >();
  constructor(
    readonly key: string,
    readonly connection: () => Promise<Connection>,
  ) {
    this.#channel = new BroadcastChannel(key);
    this.#channel.onmessage = (event) => {
      try {
        const value = event.data;
        if (value?.type === 'submit' && this.#leader) {
          exactKeys(value, ['type', 'id', 'update', 'namespace']);
          void this.#accept(value.id, value.update, value.namespace);
        } else if (value?.type === 'result') {
          exactKeys(value, ['type', 'id', 'ok']);
          requireValue(typeof value.id === 'string');
          generatedId(value.id);
          requireValue(typeof value.ok === 'boolean');
          const pending = this.#pending.get(value.id);
          if (!pending) return;
          this.#pending.delete(value.id);
          clearTimeout(pending.timer);
          if (value.ok) pending.resolve();
          else pending.reject(new Error('Edit was not accepted; reconnect before retrying'));
        }
      } catch {
        /* Untrusted relay messages never become signed updates. */
      }
    };
    const lifetime = new Promise<void>((resolve) => {
      this.#release = resolve;
    });
    void navigator.locks
      .request(key, { signal: this.#controller.signal }, async () => {
        if (this.#closed) return;
        this.#leader = true;
        await lifetime;
        await this.#tasks; // No second leader until outstanding durable work has settled.
        this.#leader = false;
      })
      .catch(() => {
        if (!this.#closed) this.close();
      });
  }
  #accept(id: unknown, bytes: unknown, namespace: unknown): Promise<void> {
    try {
      requireValue(typeof id === 'string');
      generatedId(id);
      requireValue(namespace === 'content' || namespace === 'own');
      requireValue(
        bytes instanceof Uint8Array && bytes.length <= UPDATE_BYTES && this.#working.size < 8,
      );
    } catch {
      return Promise.resolve();
    }
    const name = id as string,
      update = (bytes as Uint8Array).slice();
    const existing = this.#working.get(name);
    if (existing) {
      update.fill(0);
      return existing;
    }
    const task = this.#tasks.then(() => this.#write(name, update, namespace as 'content' | 'own'));
    this.#tasks = task.then(
      () => {},
      () => {},
    );
    const completed = task
      .then(
        () => this.#result(name, true),
        () => this.#result(name, false),
      )
      .finally(() => {
        update.fill(0);
        this.#working.delete(name);
      });
    this.#working.set(name, completed);
    return completed;
  }
  #result(id: string, ok: boolean) {
    if (this.#closed) return;
    this.#channel.postMessage({ type: 'result', id, ok });
    const pending = this.#pending.get(id);
    if (pending) {
      this.#pending.delete(id);
      clearTimeout(pending.timer);
      if (ok) pending.resolve();
      else pending.reject(new Error('Edit was not accepted; reconnect before retrying'));
    }
  }
  async #write(id: string, update: Uint8Array, namespace: 'content' | 'own') {
    requireValue(!this.#closed && this.#leader);
    const c = await this.connection();
    await c.ready;
    const state = (await record<SavedWriter>(this.key)) ?? { pending: null, completed: [] };
    exactKeys(state, ['pending', 'completed']);
    requireValue(
      Array.isArray(state.completed) &&
        state.completed.length <= 64 &&
        (state.pending === null ||
          (typeof state.pending === 'object' && !Array.isArray(state.pending))),
    );
    state.completed.forEach(generatedId);
    if (state.pending) {
      exactKeys(state.pending, ['id', 'entry']);
      generatedId(state.pending.id);
      await c.append(state.pending.entry); // Includes signature/scope/authority checks on restored data.
      state.completed.push(state.pending.id);
      state.completed = state.completed.slice(-64);
      state.pending = null;
      await record(this.key, state);
    }
    if (state.completed.includes(id)) return;
    const entry = await c.run(async () => {
      await c.fold.run(
        namespace === 'content'
          ? { type: 'check', updates: [update] }
          : {
              type: 'check',
              updates: [],
              own: [{ writer: c.admission.registration.deviceId, update }],
            },
      );
      const a = c.admission,
        head = c.objects.head(a.registration.deviceId);
      requireValue(a.root !== null && a.head !== null);
      a.author(a.registration.deviceId, a.head.revision.toString());
      const envelope = await Envelope.seal(
        {
          space: a.space,
          page: a.page,
          epoch: a.epoch,
          kind: 'update',
          namespace,
          authorDevice: a.registration.deviceId,
          membershipRevision: a.head.revision.toString(),
          streamSeq: String(head.seq + 1n),
          prevHash: head.hash,
        },
        a.root,
        a.registration.keys.sign,
        update,
      );
      requireValue(c.active && !this.#closed);
      return {
        seq: decodeHeader(envelope.header()).context.streamSeq,
        envelopeHash: encodeBinary(await envelope.hash()),
        envelope: encodeBinary(envelope.toJson()),
      };
    });
    state.pending = { id, entry };
    await record(this.key, state);
    requireValue(!this.#closed);
    await c.append(entry);
    state.pending = null;
    state.completed.push(id);
    state.completed = state.completed.slice(-64);
    await record(this.key, state);
  }
  submit(update: Uint8Array, namespace: 'content' | 'own' = 'content'): Promise<void> {
    requireValue(!this.#closed && update.length <= UPDATE_BYTES && this.#pending.size < 8);
    const id = crypto.randomUUID(),
      bytes = update.slice(),
      deadline = Date.now() + 10_000;
    return new Promise<void>((resolve, reject) => {
      const send = () => {
        if (this.#closed || Date.now() >= deadline) {
          this.#pending.delete(id);
          bytes.fill(0);
          reject(new Error('Writer relay timed out'));
          return;
        }
        if (this.#leader) void this.#accept(id, bytes, namespace);
        else this.#channel.postMessage({ type: 'submit', id, update: bytes, namespace });
        const pending = this.#pending.get(id);
        if (pending) pending.timer = setTimeout(send, 500);
      };
      this.#pending.set(id, {
        resolve: () => {
          bytes.fill(0);
          resolve();
        },
        reject: (error) => {
          bytes.fill(0);
          reject(error);
        },
        timer: setTimeout(send, 0),
      });
    });
  }
  async submitOwn(root: AskRoot, key: string, value: JsonValue) {
    validateRecord(value);
    const expected = recordKey(value);
    requireValue(root === expected.root && key === expected.key);
    const c = await this.connection();
    await c.ready;
    const prepared = await c.run(() =>
      c.fold.run({
        type: 'prepare-own',
        writer: c.admission.registration.deviceId,
        root,
        key,
        value: structuredClone(value) as unknown as JsonValue,
      }),
    );
    try {
      await this.submit(prepared.update, 'own');
    } finally {
      prepared.update.fill(0);
    }
  }
  close() {
    if (this.#closed) return;
    this.#closed = true;
    this.#controller.abort();
    this.#release();
    this.#channel.close();
    for (const pending of this.#pending.values()) {
      clearTimeout(pending.timer);
      pending.reject(new Error('Page writer closed'));
    }
    this.#pending.clear();
  }
}
