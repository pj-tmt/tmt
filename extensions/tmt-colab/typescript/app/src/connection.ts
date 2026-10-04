import {
  binary,
  decodeHeader,
  encodeBinary,
  Envelope,
  equal,
  exactKeys,
  requireValue,
  text,
} from '@tmt/colab-client';
import type { Admission } from './admission.js';
import { Catchup } from './catchup.js';
import { Fold } from './fold.js';
import type { AdmittedUpdate, Projection } from './fold-protocol.js';
import type { PageView } from './transport.js';
import { Frames } from './frames.js';
import { Objects, position, UPDATE_ENVELOPE_BYTES, type ObjectEntry } from './objects.js';
import { text as strings } from './strings.js';

/** A connection owns one reader/Worker. Queued messages and all local Worker
 * requests share one executor; no authority or keys enter the decoder. */
export class Connection {
  readonly objects: Objects;
  readonly fold = new Fold();
  readonly ready: Promise<Projection>;
  #socket: WebSocket;
  #frames: Frames;
  #catchup: Catchup;
  #tasks = Promise.resolve();
  #queued = 0;
  #complete = false;
  #projection: PageView = { source: '', title: '' };
  #stopped = false;
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
    sharing: string | readonly string[],
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
    const socket = (this.#socket = new WebSocket(url, [...protocols]));
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
          [
            'DENIED',
            'EXPIRED',
            'STALE_EPOCH',
            'INVALID',
            'GAP',
            'CAPACITY',
            'CONFLICT',
            'RESYNC_REQUIRED',
          ].includes(frame.code),
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
    } else throw new Error('Unexpected sync message');
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
    this.admission.root = null;
    this.#reject(error);
    for (const pending of this.#receipts.values()) {
      clearTimeout(pending.timer);
      pending.reject(error);
    }
    this.#receipts.clear();
    this.failed(error);
  }
}
