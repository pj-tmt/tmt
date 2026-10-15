import {
  decodeHeader,
  encodeBinary,
  Envelope,
  exactKeys,
  generatedId,
  requireValue,
} from '@tmt/colab-client';
import { Connection } from './connection.js';
import { type JsonValue, type OwnRoot, type OwnRecord, UPDATE_BYTES } from './fold-protocol.js';
import type { ObjectEntry } from './objects.js';
import { record as durableRecord } from './storage.js';
import { publicationSequence, type OwnPublicationReceipt } from './own-publication.js';
interface SavedWriter {
  pending: { id: string; entry: ObjectEntry } | null;
  completed: string[];
}
/** What combining this device's own stream did: `merged` updates now sit in one checkpoint at
 * `through`, nothing was there to combine, or the stream could not be combined (its state would
 * not fit one checkpoint, did not decode back identically, or the serve did not take it). */
export type OwnCompaction =
  | { outcome: 'combined'; merged: number; through: number }
  | { outcome: 'nothing' }
  | { outcome: 'not-combined' };
function ownCompaction(value: unknown): OwnCompaction {
  requireValue(typeof value === 'object' && value !== null && !Array.isArray(value));
  const raw = value as Record<string, unknown>;
  if (raw.outcome === 'combined') {
    exactKeys(raw, ['outcome', 'merged', 'through']);
    requireValue(
      Number.isSafeInteger(raw.merged) &&
        (raw.merged as number) > 0 &&
        Number.isSafeInteger(raw.through) &&
        (raw.through as number) >= (raw.merged as number),
    );
    return { outcome: 'combined', merged: raw.merged as number, through: raw.through as number };
  }
  exactKeys(raw, ['outcome']);
  requireValue(raw.outcome === 'nothing' || raw.outcome === 'not-combined');
  return { outcome: raw.outcome as 'nothing' | 'not-combined' };
}
const COMPACT_RELAY_MS = 30_000;

/** Where a writer keeps its state: the app's durable records, or, for a link session that stores
 * nothing in the owner app's records, memory that lasts as long as the tab. */
export interface WriterRecords {
  <T>(key: string): Promise<T | undefined>;
  <T>(key: string, value: T): Promise<void>;
}
export function memoryRecords(): WriterRecords {
  const kept = new Map<string, unknown>();
  return (async (key: string, ...values: unknown[]) => {
    if (values.length === 0) return kept.get(key);
    kept.set(key, structuredClone(values[0]));
  }) as WriterRecords;
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
  /** A tab that asked is answered from here when its answer was lost and it asks again. */
  #combined = new Map<string, OwnCompaction>();
  #compactions = new Map<
    string,
    {
      resolve(result: OwnCompaction): void;
      reject(error: Error): void;
      timer: ReturnType<typeof setTimeout>;
    }
  >();
  #pending = new Map<
    string,
    {
      seq: string | null;
      resolve(receipt: OwnPublicationReceipt): void;
      reject(error: Error): void;
      timer: ReturnType<typeof setTimeout>;
    }
  >();
  constructor(
    readonly key: string,
    readonly connection: () => Promise<Connection>,
    private readonly record: WriterRecords = durableRecord,
  ) {
    this.#channel = new BroadcastChannel(key);
    this.#channel.onmessage = (event) => {
      try {
        const value = event.data;
        if (value?.type === 'submit' && this.#leader) {
          exactKeys(value, ['type', 'id', 'update']);
          void this.#accept(value.id, value.update);
        } else if (value?.type === 'compact' && this.#leader) {
          exactKeys(value, ['type', 'id']);
          requireValue(typeof value.id === 'string');
          generatedId(value.id);
          void this.#acceptCompact(value.id);
        } else if (value?.type === 'compacted') {
          exactKeys(value, ['type', 'id', 'result']);
          requireValue(typeof value.id === 'string');
          generatedId(value.id);
          const pending = this.#compactions.get(value.id);
          if (!pending) return;
          const result = ownCompaction(value.result);
          this.#compactions.delete(value.id);
          clearTimeout(pending.timer);
          pending.resolve(result);
        } else if (value?.type === 'publication-receipt') {
          // Diagnostic metadata never acknowledges a write or changes admission. The ordinary
          // result stays unchanged, including for a tab still running the previous app assets.
          exactKeys(value, ['type', 'id', 'seq']);
          requireValue(typeof value.id === 'string' && publicationSequence(value.seq));
          const pending = this.#pending.get(value.id);
          if (pending) pending.seq = value.seq;
        } else if (value?.type === 'result') {
          exactKeys(value, ['type', 'id', 'ok']);
          requireValue(typeof value.id === 'string');
          generatedId(value.id);
          requireValue(typeof value.ok === 'boolean');
          const pending = this.#pending.get(value.id);
          if (!pending) return;
          this.#pending.delete(value.id);
          clearTimeout(pending.timer);
          if (value.ok) {
            pending.resolve({ seq: pending.seq });
          } else pending.reject(new Error('Edit was not accepted; reconnect before retrying'));
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
  #accept(id: unknown, bytes: unknown): Promise<void> {
    try {
      requireValue(typeof id === 'string');
      generatedId(id);
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
    const task = this.#tasks.then(() => this.#write(name, update));
    this.#tasks = task.then(
      () => {},
      () => {},
    );
    const completed = task
      .then(
        (seq) => this.#result(name, true, seq),
        () => this.#result(name, false),
      )
      .finally(() => {
        update.fill(0);
        this.#working.delete(name);
      });
    this.#working.set(name, completed);
    return completed;
  }
  #result(id: string, ok: boolean, seq: string | null = null) {
    if (this.#closed) return;
    if (ok && seq !== null) this.#channel.postMessage({ type: 'publication-receipt', id, seq });
    this.#channel.postMessage({ type: 'result', id, ok });
    const pending = this.#pending.get(id);
    if (pending) {
      this.#pending.delete(id);
      clearTimeout(pending.timer);
      if (ok) pending.resolve({ seq });
      else pending.reject(new Error('Edit was not accepted; reconnect before retrying'));
    }
  }
  /** The saved writer state with any interrupted append finished first: exact stored bytes are
   * retried, never resealed. */
  async #settled(c: Connection): Promise<SavedWriter> {
    const state = (await this.record<SavedWriter>(this.key)) ?? { pending: null, completed: [] };
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
      await this.record(this.key, state);
    }
    return state;
  }
  async #write(id: string, update: Uint8Array) {
    requireValue(!this.#closed && this.#leader);
    const c = await this.connection();
    await c.ready;
    const state = await this.#settled(c);
    if (state.completed.includes(id)) return null;
    const entry = await c.run(async () => {
      await c.fold.run({
        type: 'check',
        updates: [],
        own: [{ writer: c.admission.registration.deviceId, update }],
      });
      const a = c.admission,
        head = c.objects.head(a.registration.deviceId);
      requireValue(a.root !== null && a.head !== null);
      await a.ownAuthor(a.head.revision.toString());
      const envelope = await Envelope.seal(
        {
          space: a.space,
          page: a.page,
          epoch: a.epoch,
          kind: 'update',
          namespace: 'own',
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
    await this.record(this.key, state);
    requireValue(!this.#closed);
    await c.append(entry);
    state.pending = null;
    state.completed.push(id);
    state.completed = state.completed.slice(-64);
    await this.record(this.key, state);
    return entry.seq;
  }
  /** Combine this device's own stream into a checkpoint at its head. Only the leader tab runs it,
   * in the same queue as this device's writes, so no update of its own can land in between. */
  async #compactOwn(): Promise<OwnCompaction> {
    requireValue(!this.#closed && this.#leader);
    const c = await this.connection();
    await c.ready;
    const state = await this.#settled(c);
    const device = c.admission.registration.deviceId;
    const merged = c.objects.tail(device);
    if (merged === 0) return { outcome: 'nothing' };
    const entry = await c.run(async () => {
      const encoded = await c.fold.run({ type: 'encode-own', writer: device });
      try {
        // An empty encoding means the worker would not vouch for it: publish nothing.
        if (encoded.update.length === 0) return null;
        const a = c.admission,
          head = c.objects.head(device);
        requireValue(a.root !== null && a.head !== null && c.objects.tail(device) === merged);
        await a.ownAuthor(a.head.revision.toString());
        const envelope = await Envelope.seal(
          {
            space: a.space,
            page: a.page,
            epoch: a.epoch,
            kind: 'checkpoint',
            namespace: 'own',
            authorDevice: device,
            membershipRevision: a.head.revision.toString(),
            streamSeq: head.seq.toString(),
            prevHash: head.hash,
          },
          a.root,
          a.registration.keys.sign,
          encoded.update,
        );
        requireValue(c.active && !this.#closed);
        return {
          seq: head.seq.toString(),
          envelopeHash: encodeBinary(await envelope.hash()),
          envelope: encodeBinary(envelope.toJson()),
        };
      } finally {
        encoded.update.fill(0);
      }
    });
    if (!entry) return { outcome: 'not-combined' };
    const id = crypto.randomUUID();
    state.pending = { id, entry };
    await this.record(this.key, state);
    requireValue(!this.#closed);
    await c.append(entry);
    state.pending = null;
    state.completed.push(id);
    state.completed = state.completed.slice(-64);
    await this.record(this.key, state);
    return { outcome: 'combined', merged, through: Number(entry.seq) };
  }
  #acceptCompact(id: string): Promise<void> {
    const done = this.#combined.get(id);
    if (done) {
      this.#compactResult(id, done);
      return Promise.resolve();
    }
    const existing = this.#working.get(id);
    if (existing) return existing;
    if (this.#working.size >= 8) return Promise.resolve();
    const task = this.#tasks.then(() => this.#compactOwn());
    this.#tasks = task.then(
      () => {},
      () => {},
    );
    const completed = task
      .then(
        (result) => {
          this.#combined.set(id, result);
          if (this.#combined.size > 8) this.#combined.delete(this.#combined.keys().next().value!);
          this.#compactResult(id, result);
        },
        () => this.#compactResult(id, { outcome: 'not-combined' }),
      )
      .finally(() => this.#working.delete(id));
    this.#working.set(id, completed);
    return completed;
  }
  #compactResult(id: string, result: OwnCompaction) {
    if (this.#closed) return;
    this.#channel.postMessage({ type: 'compacted', id, result });
    const pending = this.#compactions.get(id);
    if (pending) {
      this.#compactions.delete(id);
      clearTimeout(pending.timer);
      pending.resolve(result);
    }
  }
  /** Combine this device's own stream, from any tab: the leader tab does the work. A relay that
   * finds no leader in time rejects, and the caller says the stream could not be combined. */
  compact(): Promise<OwnCompaction> {
    requireValue(!this.#closed);
    const id = crypto.randomUUID(),
      deadline = Date.now() + COMPACT_RELAY_MS;
    return new Promise<OwnCompaction>((resolve, reject) => {
      const send = () => {
        if (this.#closed || Date.now() >= deadline) {
          this.#compactions.delete(id);
          reject(new Error('Writer relay timed out'));
          return;
        }
        if (this.#leader) void this.#acceptCompact(id);
        else this.#channel.postMessage({ type: 'compact', id });
        const pending = this.#compactions.get(id);
        if (pending) pending.timer = setTimeout(send, 500);
      };
      this.#compactions.set(id, { resolve, reject, timer: setTimeout(send, 0) });
    });
  }
  submit(update: Uint8Array): Promise<OwnPublicationReceipt> {
    requireValue(!this.#closed && update.length <= UPDATE_BYTES && this.#pending.size < 8);
    const id = crypto.randomUUID(),
      bytes = update.slice(),
      deadline = Date.now() + 10_000;
    return new Promise<OwnPublicationReceipt>((resolve, reject) => {
      const send = () => {
        if (this.#closed || Date.now() >= deadline) {
          this.#pending.delete(id);
          bytes.fill(0);
          reject(new Error('Writer relay timed out'));
          return;
        }
        if (this.#leader) void this.#accept(id, bytes);
        else this.#channel.postMessage({ type: 'submit', id, update: bytes });
        const pending = this.#pending.get(id);
        if (pending) pending.timer = setTimeout(send, 500);
      };
      this.#pending.set(id, {
        seq: null,
        resolve: (receipt) => {
          bytes.fill(0);
          resolve(receipt);
        },
        reject: (error) => {
          bytes.fill(0);
          reject(error);
        },
        timer: setTimeout(send, 0),
      });
    });
  }
  async submitOwn(root: OwnRoot, key: string, value: JsonValue) {
    return this.submitOwnRecords([{ root, key, value }]);
  }
  /** A thread and its opening comment share one admitted envelope. `fits` may refuse the prepared
   * update by throwing, before it is submitted. */
  async submitOwnRecords(records: OwnRecord[], fits?: (update: number) => void) {
    records = structuredClone(records);
    requireValue(records.length > 0 && records.length <= 32);
    requireValue(
      records.every((record) =>
        ['threads', 'intents', 'messages', 'replies'].includes(record.root),
      ),
    );
    const c = await this.connection();
    await c.ready;
    const prepared = await c.run(() =>
      c.fold.run({
        type: 'prepare-own',
        writer: c.admission.registration.deviceId,
        records: structuredClone(records),
      }),
    );
    try {
      // A refusal here is before anything is sealed, stored or sent.
      fits?.(prepared.update.length);
      const receipt = await this.submit(prepared.update);
      await c.waitForOwnRecords(records, receipt);
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
    for (const pending of this.#compactions.values()) {
      clearTimeout(pending.timer);
      pending.reject(new Error('Page writer closed'));
    }
    this.#compactions.clear();
  }
}
