import {
  binary,
  decodeHeader,
  digest,
  encodeBinary,
  Envelope,
  equal,
  exactKeys,
  requireValue,
  text,
} from '@tmt/colab-client';
import {
  AttachmentObjectChannel,
  objectOutcome,
  type AttachmentObjectRequest,
  type AttachmentObjectOutcome,
} from './attachment-channel.js';
import { attachmentHistory, type AttachmentSnapshot } from './attachment-history.js';
import type { Admission } from './admission.js';
import { Catchup } from './catchup.js';
import { Fold } from './fold.js';
import type { AdmittedUpdate, JsonValue, OwnRecord, Projection } from './fold-protocol.js';
import type { PageView } from './transport.js';
import { Frames } from './frames.js';
import { hex } from './export.js';
import { Objects, position, UPDATE_ENVELOPE_BYTES, type ObjectEntry } from './objects.js';
import { SAVE_SOURCE_BYTES, SaveTooLarge, saveResult, type SaveResult } from './save.js';
import { text as strings } from './strings.js';

function sameValue(a: JsonValue | undefined, b: JsonValue): boolean {
  if (a === b) return true;
  if (a === null || b === null || typeof a !== 'object' || typeof b !== 'object') return false;
  if (Array.isArray(a) || Array.isArray(b))
    return (
      Array.isArray(a) &&
      Array.isArray(b) &&
      a.length === b.length &&
      a.every((value, index) => sameValue(value, b[index]))
    );
  const keys = Object.keys(b);
  return (
    Object.keys(a).length === keys.length &&
    keys.every((key) => Object.hasOwn(a, key) && sameValue(a[key], b[key]))
  );
}

const CHUNK_BYTES = 32 * 1024;
/** The whole reply budget for one save: upload window, native preparation and commit. */
const SAVE_REPLY_MS = 30_000;
/** Bytes the socket may hold unsent before the next chunk waits; keeps a 2 MiB save paced. */
const SAVE_BUFFER_BYTES = 256 * 1024;

/** The refusal codes a sync error frame may carry; the terminal card words each one. */
export const SYNC_ERROR_CODES = [
  'DENIED',
  'EXPIRED',
  'STALE_EPOCH',
  'INVALID',
  'GAP',
  'CAPACITY',
  'CONFLICT',
  'RESYNC_REQUIRED',
] as const;

/** A connection owns one reader/Worker. Queued messages and all local Worker
 * requests share one executor; no authority or keys enter the decoder. */
export class Connection {
  readonly objects: Objects;
  readonly fold = new Fold();
  readonly attachmentObjects = new AttachmentObjectChannel((request, deadline) =>
    this.objectRequest(request, deadline),
  );
  #objectReplies = new Map<
    string,
    {
      method: AttachmentObjectRequest['method'];
      resolve(value: AttachmentObjectOutcome): void;
      reject(error: Error): void;
      timer: ReturnType<typeof setTimeout>;
    }
  >();
  readonly ready: Promise<Projection>;
  #socket: WebSocket;
  #frames: Frames;
  #catchup: Catchup;
  #tasks = Promise.resolve();
  #queued = 0;
  #complete = false;
  #projection: PageView = { source: '', title: '' };
  #stopped = false;
  #ownPublications = new Set<{
    records: readonly OwnRecord[];
    resolve(): void;
    reject(error: Error): void;
    timer: ReturnType<typeof setTimeout>;
  }>();
  /** At most one save or status request is in flight per connection. */
  #reply: {
    operationId: string;
    resolve(value: SaveResult): void;
    reject(error: Error): void;
    timer: ReturnType<typeof setTimeout>;
  } | null = null;
  #receipts = new Map<
    string,
    {
      entry: ObjectEntry;
      resolve(): void;
      reject(error: Error): void;
      timer: ReturnType<typeof setTimeout>;
    }
  >();
  #resolve!: (value: Projection) => void;
  #reject!: (error: Error) => void;
  #timer: ReturnType<typeof setTimeout>;
  constructor(
    readonly admission: Admission,
    mount: URL,
    private readonly sharing: string | readonly string[],
    readonly publish: (value: PageView) => void,
    readonly failed: (error: Error) => void,
    /** Offered subprotocols; the server selects only `colab-sync-v1`. A reader appends its ticket. */
    protocols: readonly string[] = ['colab-sync-v1'],
  ) {
    this.objects = new Objects(admission);
    this.#catchup = new Catchup(admission, sharing, this.objects);
    this.#frames = new Frames(admission, (error) => this.close(error));
    this.ready = new Promise((resolve, reject) => {
      this.#resolve = resolve;
      this.#reject = reject;
    });
    this.#timer = setTimeout(() => this.close(new Error('Page catchup timed out')), 10_000);
    const url = new URL('sync', mount);
    url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
    const socket = (this.#socket = new WebSocket(admission.registration.syncUrl ?? url, [
      ...protocols,
    ]));
    socket.onopen = () => {
      try {
        if (socket.protocol !== 'colab-sync-v1') throw new Error('Invalid sync protocol');
        this.send('hello', {
          device: admission.reader?.principal ?? admission.registration.deviceId,
          membershipRevision: admission.head?.revision.toString() ?? '0',
          cursors: [],
        });
      } catch (error) {
        this.close(error instanceof Error ? error : new Error('Sync unavailable'));
      }
    };
    socket.onmessage = (event) => {
      if (this.#stopped) return;
      if (
        typeof event.data !== 'string' ||
        text(event.data).length > 64 * 1024 ||
        ++this.#queued > 8
      )
        return this.close(new Error('Sync message capacity'));
      const raw = event.data;
      void this.run(async () => {
        const frame = this.#frames.receive(raw);
        if (frame) await this.#receive(frame);
        // Partial chunks acknowledge only earlier admitted cursors. ACK grants
        // no application authority and never treats incomplete bytes as an object.
        this.send('ack', { cursors: this.objects.cursors() });
      })
        .catch((error) =>
          this.close(error instanceof Error ? error : new Error('Invalid sync message')),
        )
        .finally(() => this.#queued--);
    };
    socket.onerror = socket.onclose = () => this.close(new Error('Sync disconnected'));
  }
  get active() {
    return !this.#stopped;
  }
  run<T>(fn: () => Promise<T>): Promise<T> {
    const result = this.#tasks.then(async () => {
      requireValue(!this.#stopped);
      return fn();
    });
    this.#tasks = result.then(
      () => {},
      () => {},
    );
    return result;
  }
  /** Internal attachment owner: only this executor's authenticated Worker
   * projection and Objects cuts enter a capture. Historical bootstrap is supplied
   * by the object adapter, never substituted with the current projection. */
  async attachmentSnapshot(
    epoch = this.admission.epoch,
    deadline = performance.now() + 15_000,
  ): Promise<AttachmentSnapshot> {
    if (epoch !== this.admission.epoch) {
      // The detached source awaits outside run(): its responses arrive through
      // that same executor. It never mutates the live Worker or peer scope.
      return attachmentHistory(
        () => this.attachmentSnapshot(this.admission.epoch, deadline),
        this.attachmentObjects.historySource(),
        epoch,
        this.sharing,
        deadline,
      );
    }
    return this.run(async () => {
      requireValue(this.#complete && performance.now() < deadline);
      return {
        admission: this.admission,
        objects: this.objects,
        projection: structuredClone(this.#projection),
        revision: await this.objects.revision(),
      };
    });
  }
  /** Await outside the Worker executor: inbound replies use that executor too. */
  private async objectRequest(
    request: AttachmentObjectRequest,
    deadline: number,
  ): Promise<AttachmentObjectOutcome> {
    await this.ready;
    requireValue(
      !this.#stopped &&
        Number.isFinite(deadline) &&
        performance.now() < deadline &&
        this.#objectReplies.size < 8,
    );
    const requestId = crypto.randomUUID();
    return new Promise((resolve, reject) => {
      const timer = setTimeout(
        () => this.close(new Error('Storage unavailable')),
        Math.min(15_000, deadline - performance.now()),
      );
      this.#objectReplies.set(requestId, { method: request.method, resolve, reject, timer });
      try {
        this.send('object', { requestId, request });
      } catch (error) {
        clearTimeout(timer);
        this.#objectReplies.delete(requestId);
        reject(error instanceof Error ? error : new Error('Object channel unavailable'));
      }
    });
  }
  send(type: string, fields: Record<string, unknown>) {
    requireValue(!this.#stopped && this.#socket.readyState === WebSocket.OPEN);
    const a = this.admission,
      raw = JSON.stringify({
        version: 1,
        type,
        space: a.space,
        page: a.page,
        epoch: a.epoch,
        ...fields,
      });
    requireValue(text(raw).length <= 64 * 1024 && this.#socket.bufferedAmount <= 1024 * 1024);
    this.#socket.send(raw);
  }
  #emit(projection: PageView) {
    this.#projection = projection;
    this.publish({ ...projection, ownData: this.objects.ownData });
    for (const pending of this.#ownPublications) {
      if (!this.#containsOwn(pending.records)) continue;
      this.#ownPublications.delete(pending);
      clearTimeout(pending.timer);
      pending.resolve();
    }
  }
  #containsOwn(records: readonly OwnRecord[]) {
    const own = this.#projection.own?.[this.admission.registration.deviceId];
    return records.every(({ root, key, value }) => sameValue(own?.[root][key], value));
  }
  /** Relay success proves the leader appended, not that this reader has admitted
   * the broadcast. Dependent reads wait for this connection's verified fold. */
  waitForOwnRecords(records: readonly OwnRecord[]): Promise<void> {
    requireValue(!this.#stopped && records.length > 0 && records.length <= 32);
    if (this.#containsOwn(records)) return Promise.resolve();
    requireValue(this.#ownPublications.size < 8);
    return new Promise<void>((resolve, reject) => {
      const pending = {
        records: structuredClone(records),
        resolve,
        reject,
        timer: setTimeout(() => this.close(new Error('Own publication catchup timed out')), 10_000),
      };
      this.#ownPublications.add(pending);
    });
  }
  #batch(values: AdmittedUpdate[]) {
    return {
      updates: values.filter((v) => v.namespace === 'content').map((v) => v.update),
      own: values
        .filter((v) => v.namespace === 'own')
        .map(({ writer, update }) => ({ writer, update })),
    };
  }
  async #apply(stream: string, entry: ObjectEntry) {
    const update = await this.objects.admit(stream, entry);
    if (!update) {
      if (!this.#stopped) this.#emit(this.#projection);
      return;
    }
    try {
      const projection = await this.fold.run({ type: 'apply', ...this.#batch([update]) });
      if (!this.#stopped) this.#emit(projection);
    } finally {
      update.update.fill(0);
    }
  }
  async #receive(frame: Record<string, unknown>) {
    if (frame.type === 'error') {
      exactKeys(frame, ['version', 'type', 'space', 'page', 'epoch', 'code']);
      requireValue(
        typeof frame.code === 'string' &&
          (SYNC_ERROR_CODES as readonly string[]).includes(frame.code),
      );
      throw new Error(frame.code);
    }
    if (!this.#complete) {
      requireValue(frame.type === 'catchup');
      if (!(await this.#catchup.admitValue(frame))) return;
      if (!this.admission.root) throw new Error(strings.noWraps);
      const updates = this.#catchup.updates;
      try {
        const baseline = this.#catchup.baseline;
        if (baseline) {
          const result = await this.fold.run({
            type: 'baseline',
            update: baseline.update,
            title: baseline.title,
            sourceDigest: baseline.sourceDigest,
            commitment: baseline.commitment,
          });
          requireValue(result.source === baseline.source && result.title === baseline.title);
        }
        for (const update of this.#catchup.checkpoints)
          await this.fold.run({
            type: 'checkpoint',
            update: update.update,
            writer: update.namespace === 'own' ? update.writer : undefined,
          });
        const projection = await this.fold.run({ type: 'apply', ...this.#batch(updates) });
        requireValue(!this.#stopped);
        this.#complete = true;
        clearTimeout(this.#timer);
        this.#emit(projection);
        this.#resolve(projection);
      } finally {
        this.#catchup.baseline?.update.fill(0);
        this.#catchup.baseline = null;
        this.#catchup.checkpoints.forEach((v) => v.update.fill(0));
        this.#catchup.checkpoints = [];
        updates.forEach((v) => v.update.fill(0));
        this.#catchup.updates = [];
      }
      return;
    }
    if (frame.type === 'broadcast') {
      exactKeys(frame, [
        'version',
        'type',
        'space',
        'page',
        'epoch',
        'streamId',
        'seq',
        'envelopeHash',
        'envelope',
        ...(Object.hasOwn(frame, 'chains') ? ['chains'] : []),
      ]);
      if (Object.hasOwn(frame, 'chains')) await this.admission.chains(frame.chains);
      const pos = { streamId: frame.streamId, seq: frame.seq, envelopeHash: frame.envelopeHash };
      position(pos);
      requireValue(typeof frame.streamId === 'string' && typeof frame.envelope === 'string');
      await this.#apply(frame.streamId, {
        seq: pos.seq,
        envelopeHash: pos.envelopeHash,
        envelope: frame.envelope,
      });
    } else if (frame.type === 'receipt') {
      exactKeys(frame, [
        'version',
        'type',
        'space',
        'page',
        'epoch',
        'streamId',
        'seq',
        'envelopeHash',
      ]);
      position({ streamId: frame.streamId, seq: frame.seq, envelopeHash: frame.envelopeHash });
      requireValue(
        frame.streamId === this.admission.registration.deviceId && typeof frame.seq === 'string',
      );
      const pending = this.#receipts.get(frame.seq);
      requireValue(pending !== undefined && pending.entry.envelopeHash === frame.envelopeHash);
      await this.#apply(frame.streamId, pending.entry);
      this.#receipts.delete(frame.seq);
      clearTimeout(pending.timer);
      pending.resolve();
    } else if (frame.type === 'object-result') {
      exactKeys(frame, ['version', 'type', 'space', 'page', 'epoch', 'requestId', 'result']);
      requireValue(typeof frame.requestId === 'string');
      const pending = this.#objectReplies.get(frame.requestId);
      requireValue(pending !== undefined);
      const result = objectOutcome(frame.result, pending.method);
      this.#objectReplies.delete(frame.requestId);
      clearTimeout(pending.timer);
      pending.resolve(result);
    } else if (frame.type === 'saveresult') {
      const pending = this.#reply;
      requireValue(pending !== null);
      const result = saveResult(frame, pending.operationId);
      this.#reply = null;
      clearTimeout(pending.timer);
      pending.resolve(result);
    } else throw new Error('Unexpected sync message');
  }
  /** Publish the whole source through native preparation. The reply says what happened; a lost
   * reply closes this connection and is settled by one `saveStatus` on the next. */
  async save(operationId: string, base: string, source: string): Promise<SaveResult> {
    await this.ready;
    const bytes = text(source);
    if (bytes.length > SAVE_SOURCE_BYTES) throw new SaveTooLarge(bytes.length, SAVE_SOURCE_BYTES);
    const [baseHash, sourceHash] = await Promise.all([digest(text(base)), digest(bytes)]);
    const fields = {
      operationId,
      baseSha256: encodeBinary(baseHash),
      sourceSha256: encodeBinary(sourceHash),
    };
    return this.#request(operationId, async () => {
      if (bytes.length <= CHUNK_BYTES)
        return this.send('save', { ...fields, source: encodeBinary(bytes) });
      const objectId = hex(sourceHash),
        count = Math.ceil(bytes.length / CHUNK_BYTES);
      this.send('save', { ...fields, source: { objectId } });
      for (let index = 0; index < count; index++) {
        while (!this.#stopped && this.#socket.bufferedAmount > SAVE_BUFFER_BYTES)
          await new Promise((resolve) => setTimeout(resolve, 10));
        this.send('chunk', {
          objectId,
          envelopeHash: fields.sourceSha256,
          index,
          count,
          bytes: encodeBinary(bytes.slice(index * CHUNK_BYTES, (index + 1) * CHUNK_BYTES)),
        });
      }
    });
  }
  /** What the page recorded for an operation ID; the one request that settles a lost reply. */
  async saveStatus(operationId: string): Promise<SaveResult> {
    await this.ready;
    return this.#request(operationId, async () => this.send('savestatus', { operationId }));
  }
  #request(operationId: string, send: () => Promise<void>): Promise<SaveResult> {
    requireValue(!this.#stopped && this.#reply === null);
    return new Promise<SaveResult>((resolve, reject) => {
      const timer = setTimeout(() => this.close(new Error('Save reply timed out')), SAVE_REPLY_MS);
      this.#reply = { operationId, resolve, reject, timer };
      send().catch((error) =>
        this.close(error instanceof Error ? error : new Error('Save unavailable')),
      );
    });
  }
  async append(entry: ObjectEntry): Promise<void> {
    await this.ready;
    exactKeys(entry, ['seq', 'envelopeHash', 'envelope']);
    const a = this.admission,
      bytes = binary(entry.envelope, UPDATE_ENVELOPE_BYTES),
      env = Envelope.fromJson(bytes),
      h = decodeHeader(env.header());
    requireValue(
      h.context.space === a.space &&
        h.context.page === a.page &&
        h.context.epoch === a.epoch &&
        h.context.kind === 'update' &&
        (h.context.namespace === 'content' || h.context.namespace === 'own') &&
        h.context.authorDevice === a.registration.deviceId &&
        h.context.streamSeq === entry.seq &&
        equal(await env.hash(), binary(entry.envelopeHash, 32, 32)),
    );
    const plaintext = await env.open(
      h.context,
      a.root!,
      a.author(a.registration.deviceId, h.context.membershipRevision),
    );
    plaintext.fill(0);
    requireValue(!this.#receipts.has(entry.seq));
    await new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => this.close(new Error('Append receipt timed out')), 10_000);
      this.#receipts.set(entry.seq, { entry, resolve, reject, timer });
      try {
        this.send('append', {
          streamId: a.registration.deviceId,
          seq: entry.seq,
          envelopeHash: entry.envelopeHash,
          envelope: bytes.length > 32 * 1024 ? { objectId: h.objectId } : entry.envelope,
        });
        if (bytes.length > 32 * 1024) {
          const count = Math.ceil(bytes.length / (32 * 1024));
          for (let index = 0; index < count; index++)
            this.send('chunk', {
              objectId: h.objectId,
              envelopeHash: entry.envelopeHash,
              index,
              count,
              bytes: encodeBinary(bytes.slice(index * 32768, (index + 1) * 32768)),
            });
        }
      } catch (error) {
        this.close(error instanceof Error ? error : new Error('Append unavailable'));
      }
    });
  }
  close(error = new Error('Page closed')) {
    if (this.#stopped) return;
    this.#stopped = true;
    clearTimeout(this.#timer);
    this.#frames.close();
    this.#catchup.close();
    this.fold.close();
    this.#socket.onopen =
      this.#socket.onmessage =
      this.#socket.onerror =
      this.#socket.onclose =
        null;
    this.#socket.close();
    this.admission.closeKeys();
    this.#reject(error);
    for (const pending of this.#receipts.values()) {
      clearTimeout(pending.timer);
      pending.reject(error);
    }
    this.#receipts.clear();
    for (const pending of this.#objectReplies.values()) {
      clearTimeout(pending.timer);
      pending.reject(error);
    }
    this.#objectReplies.clear();
    if (this.#reply) {
      clearTimeout(this.#reply.timer);
      this.#reply.reject(error);
      this.#reply = null;
    }
    for (const pending of this.#ownPublications) {
      clearTimeout(pending.timer);
      pending.reject(error);
    }
    this.#ownPublications.clear();
    this.failed(error);
  }
}
