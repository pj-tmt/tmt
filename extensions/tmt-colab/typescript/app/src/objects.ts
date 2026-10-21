import {
  binary,
  checkAnchor,
  digest,
  encodeBinary,
  frame,
  streamCut,
  text,
  decimal,
  decodeHeader,
  Envelope,
  equal,
  exactKeys,
  FOLD,
  generatedId,
  MAX_ENVELOPE_JSON,
  requireValue,
  signatureInput,
  strictVerify,
} from '@tmt/colab-client';
import { Admission } from './admission.js';
import {
  READ_TAIL_UPDATES,
  STATE_BYTES,
  UPDATE_BYTES,
  type AdmittedUpdate,
} from './fold-protocol.js';
export interface Position {
  seq: string;
  envelopeHash: string;
}
export interface ObjectEntry extends Position {
  envelope: string;
}
export function position(value: unknown): asserts value is Position {
  exactKeys(value, ['streamId', 'seq', 'envelopeHash']);
  requireValue(typeof value.streamId === 'string' && typeof value.seq === 'string');
  generatedId(value.streamId);
  decimal(value.seq);
  binary(value.envelopeHash, 32, 32);
}
/** The most bytes a sealed envelope holds for an update of `update` bytes: base64 of the update
 * and its framing, plus the header and signature. */
export const sealedBound = (update: number) => Math.floor(((update + 2048) * 4) / 3) + 2048;
export const UPDATE_ENVELOPE_BYTES = sealedBound(UPDATE_BYTES);
type Namespace = 'content' | 'own';
interface Head {
  seq: bigint;
  hash: Uint8Array;
}
/** Shared author chain; checkpoint hashes are namespace cursors, not update heads.
 * Authenticated plaintext carries its namespace and writer into the isolated fold. */
export class Objects {
  #heads = new Map<string, Head>();
  #ownSigningKeys = new Map<string, Uint8Array>();
  #ownerWriters = new Map<string, boolean>();
  #seen = new Map<string, Uint8Array>();
  #positions = new Map<
    string,
    { streamId: string; namespace: Namespace; seq: string; envelopeHash: string }
  >();
  #prefixes = new Map<string, { head: Head; checkpoints: Map<Namespace, Uint8Array> }>();
  /** The covered stream's update envelope at n that a fold is checked against (colab-v1, "Planned
   * owner fold"): admitted as an anchor, applied to nothing, and used only to verify a fold. */
  #anchors = new Map<string, { seq: bigint; envelope: Envelope }>();
  ownData = false;
  constructor(
    readonly admission: Admission,
    readonly epoch = admission.epoch,
  ) {}
  head(stream: string) {
    const h = this.#heads.get(stream);
    return { seq: h?.seq ?? 0n, hash: h?.hash.slice() ?? new Uint8Array(32) };
  }
  /** Display verification only: this key came from a cut-admitted, authenticated
   * own envelope. It does not grant current publication or Remote authority. */
  ownSigningKey(writer: string): Uint8Array | undefined {
    return this.#ownSigningKeys.get(writer)?.slice();
  }
  /** Whether every own envelope admitted from this writer came from an owner-member device,
   * the browser's form of native `status_writers`. A bridge's records are visible, but only
   * an owner device can resolve a thread. */
  statusWriter(writer: string): boolean {
    return this.#ownerWriters.get(writer) === true;
  }
  /** The updates a stream holds after its checkpoints (all of them when it has none): the ones
   * only the device that wrote the stream can combine into a checkpoint. */
  tail(stream: string): number {
    return Number(this.head(stream).seq - (this.#prefixes.get(stream)?.head.seq ?? 0n));
  }
  /** This device sealed a checkpoint over its own stream's `own` namespace at the stream's head
   * and the serve receipted it: the cut moves exactly as admitting it at catch-up would move it,
   * so the revision this device computes stays the serve's. */
  recordCheckpoint(stream: string, entry: ObjectEntry, head: Uint8Array) {
    const seq = decimal(entry.seq),
      current = this.head(stream),
      hash = binary(entry.envelopeHash, 32, 32),
      prior = this.#prefixes.get(stream);
    // The same checkpoint again (a retry after a restart) changes nothing.
    if (
      prior &&
      prior.head.seq === seq &&
      equal(prior.checkpoints.get('own') ?? new Uint8Array(), hash)
    )
      return;
    requireValue(
      seq > 0n &&
        seq === current.seq &&
        equal(head, current.hash) &&
        seq > (this.#prefixes.get(stream)?.head.seq ?? 0n),
    );
    this.#prefixes.set(stream, {
      head: { seq, hash: head.slice() },
      checkpoints: new Map([['own', hash]]),
    });
    this.#positions.set(`${stream}:own`, {
      streamId: stream,
      namespace: 'own',
      seq: entry.seq,
      envelopeHash: entry.envelopeHash,
    });
  }
  /** Checkpoints the serve says it holds that this view has not recorded: the latest notice per
   * stream and namespace. A notice grants nothing; it only says the revision computed here may
   * not be the serve's until a catch-up admits that checkpoint. */
  #unseen = new Map<
    string,
    { stream: string; namespace: Namespace; seq: bigint; hash: Uint8Array }
  >();
  /** The serve announced a stored checkpoint of `stream`. It is recorded as unseen unless this
   * view already holds that cut or a later one; nothing else changes. */
  noteCut(stream: string, namespace: Namespace, entry: Position) {
    generatedId(stream);
    const seq = decimal(entry.seq),
      hash = binary(entry.envelopeHash, 32, 32),
      key = `${stream}:${namespace}`,
      held = this.#unseen.get(key);
    requireValue(seq > 0n);
    if (this.#holds(stream, namespace, seq, hash) || (held && held.seq >= seq)) return;
    requireValue(held !== undefined || this.#unseen.size < 256);
    this.#unseen.set(key, { stream, namespace, seq, hash });
  }
  /** Whether the serve holds a checkpoint this view has not recorded. Notices that a catch-up
   * has since admitted, or a later cut has passed, no longer count. */
  hasUnseenCut(): boolean {
    for (const [key, cut] of this.#unseen)
      if (this.#holds(cut.stream, cut.namespace, cut.seq, cut.hash)) this.#unseen.delete(key);
    return this.#unseen.size > 0;
  }
  #holds(stream: string, namespace: Namespace, seq: bigint, hash: Uint8Array) {
    const cut = this.#prefixes.get(stream);
    return (
      cut !== undefined &&
      (cut.head.seq > seq ||
        (cut.head.seq === seq && equal(cut.checkpoints.get(namespace) ?? new Uint8Array(), hash)))
    );
  }
  cursors() {
    return [...this.#positions]
      .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
      .map(([, v]) => ({ ...v }));
  }
  /** Native page::token parity: namespace order, stream order and exact cuts,
   * including the empty namespace of a shared per-device author chain. */
  async revision(): Promise<string> {
    const a = this.admission,
      head = a.head;
    requireValue(head !== null);
    const cuts = (['content', 'own'] as const).flatMap((namespace) =>
      [...this.#heads.keys()].sort().map((streamId) => {
        const prefix = this.#prefixes.get(streamId),
          checkpointHash = prefix?.checkpoints.get(namespace) ?? null,
          checkpointSeq = checkpointHash ? prefix!.head.seq.toString() : '0',
          position = this.#positions.get(`${streamId}:${namespace}`),
          atCheckpoint =
            position !== undefined && checkpointHash !== null && position.seq === checkpointSeq;
        return {
          pageId: a.page,
          epoch: this.epoch,
          namespace,
          cut: encodeBinary(
            streamCut.input({
              streamId,
              namespace,
              checkpointHash,
              checkpointSeq,
              tailHeadSeq: position?.seq ?? '0',
              tailHeadHash: atCheckpoint
                ? prefix!.head.hash
                : position
                  ? binary(position.envelopeHash, 32, 32)
                  : new Uint8Array(32),
            }),
          ),
        };
      }),
    );
    const bytes = await digest(
      frame(
        text('tmt-colab-page-revision-v1'),
        text(a.space),
        text(a.page),
        text(head.revision.toString()),
        head.hash.slice(),
        text(this.epoch),
        text(JSON.stringify(cuts)),
      ),
    );
    return `v1:${[...bytes].map((b) => b.toString(16).padStart(2, '0')).join('')}`;
  }
  finish() {
    for (const { cut } of this.admission.cuts(undefined, this.epoch)) {
      const seq = decimal(cut.tailHeadSeq, true);
      if (seq === 0n) continue;
      const exact = this.#seen.get(`${cut.streamId}:${seq}`);
      requireValue(exact !== undefined && equal(exact, cut.tailHeadHash));
    }
  }
  async admit(
    stream: string,
    entry: ObjectEntry,
    kind: 'update' | 'checkpoint' = 'update',
    namespace?: Namespace,
  ): Promise<AdmittedUpdate | null> {
    exactKeys(entry, ['seq', 'envelopeHash', 'envelope']);
    generatedId(stream);
    const seq = decimal(entry.seq),
      hash = binary(entry.envelopeHash, 32, 32);
    const env = Envelope.fromJson(
      binary(entry.envelope, kind === 'checkpoint' ? MAX_ENVELOPE_JSON : UPDATE_ENVELOPE_BYTES),
    );
    const { context: c, coveredDevice } = decodeHeader(env.header()),
      a = this.admission;
    // A checkpoint slot holds a checkpoint of the stream's own device, or an owner fold of it,
    // which another owner device signs and which names the stream as its covered device.
    const fold = kind === 'checkpoint' && c.kind === FOLD;
    requireValue(
      c.space === a.space &&
        c.page === a.page &&
        c.epoch === this.epoch &&
        (fold ? coveredDevice === stream : c.authorDevice === stream && c.kind === kind) &&
        c.streamSeq === entry.seq &&
        (namespace === undefined || c.namespace === namespace) &&
        equal(await env.hash(), hash),
    );
    let key: Uint8Array,
      coveredKey: Uint8Array | undefined,
      ownerDevice = false;
    if (fold) {
      // The anchor is the covered device's own update at n: a fold names a head that device
      // signed, never one its signer asserts. Without it the fold is refused and applies nothing.
      const anchor = this.#anchors.get(stream);
      requireValue(anchor !== undefined && anchor.seq === seq);
      const keys = a.readFold(c, stream, hash);
      await checkAnchor(env, anchor.envelope, keys.covered);
      key = keys.signer;
      coveredKey = keys.covered;
      ownerDevice = true;
    } else {
      const author = a.readAuthor(c, hash);
      key = author.key;
      ownerDevice = author.ownerDevice;
    }
    requireValue(
      await strictVerify(
        key,
        env.signature(),
        await signatureInput(env.header(), env.ciphertext()),
      ),
    );
    const ns = c.namespace;
    const name = `${stream}:${seq}`,
      head = this.head(stream);
    let replay = false;
    if (kind === 'checkpoint') {
      const prefix = this.#prefixes.get(stream);
      if (prefix) {
        requireValue(
          seq === prefix.head.seq && equal(c.prevHash, prefix.head.hash) && head.seq === seq,
        );
        const prior = prefix.checkpoints.get(ns);
        if (prior) {
          requireValue(equal(prior, hash));
          replay = true;
        }
      } else
        requireValue(
          head.seq === 0n && this.#heads.size < 256 && this.#seen.size < READ_TAIL_UPDATES + 256,
        );
    } else {
      const prior = this.#seen.get(name);
      if (prior) {
        requireValue(equal(prior, hash));
        replay = true;
      } else
        requireValue(
          seq === head.seq + 1n &&
            equal(c.prevHash, head.hash) &&
            this.#seen.size < READ_TAIL_UPDATES + 256 &&
            (this.#heads.has(stream) || this.#heads.size < 256),
        );
    }
    if (replay) {
      if (ns === 'own') this.ownData = true;
      return null;
    }
    requireValue(this.#positions.has(`${stream}:${ns}`) || this.#positions.size < 256);
    const plaintext = await env.open(c, a.readRoot(this.epoch), key, fold ? stream : undefined);
    if (plaintext.length > (kind === 'checkpoint' ? STATE_BYTES : UPDATE_BYTES)) {
      plaintext.fill(0);
      throw new Error('Checkpoint decoder capacity');
    }
    if (kind === 'checkpoint') {
      let prefix = this.#prefixes.get(stream);
      if (!prefix) {
        prefix = { head: { seq, hash: c.prevHash.slice() }, checkpoints: new Map() };
        this.#prefixes.set(stream, prefix);
        this.#heads.set(stream, prefix.head);
        this.#seen.set(name, prefix.head.hash);
      }
      prefix.checkpoints.set(ns, hash);
    } else {
      this.#seen.set(name, hash);
      this.#heads.set(stream, { seq, hash });
    }
    this.#positions.set(`${stream}:${ns}`, {
      streamId: stream,
      namespace: ns,
      seq: entry.seq,
      envelopeHash: entry.envelopeHash,
    });
    if (ns === 'own') {
      this.ownData = true;
      // The writer of an own record is the covered device, so its key is the one that vouches.
      this.#ownSigningKeys.set(stream, (coveredKey ?? key).slice());
      this.#ownerWriters.set(stream, (this.#ownerWriters.get(stream) ?? true) && ownerDevice);
    }
    return { namespace: ns, writer: stream, update: plaintext };
  }
  /** An anchor frame: the covered stream's update envelope at n, which a fold that follows is
   * checked against. It is signed by an owner-member device within what its revoke cut allows,
   * and applies to nothing: it moves no head, cut or cursor. */
  async #admitAnchor(stream: string, entry: ObjectEntry, namespace: Namespace) {
    exactKeys(entry, ['seq', 'envelopeHash', 'envelope']);
    const seq = decimal(entry.seq),
      hash = binary(entry.envelopeHash, 32, 32),
      env = Envelope.fromJson(binary(entry.envelope, UPDATE_ENVELOPE_BYTES));
    const { context: c, coveredDevice } = decodeHeader(env.header()),
      a = this.admission;
    requireValue(
      seq > 0n &&
        c.space === a.space &&
        c.page === a.page &&
        c.epoch === this.epoch &&
        c.kind === 'update' &&
        coveredDevice === undefined &&
        c.authorDevice === stream &&
        c.streamSeq === entry.seq &&
        c.namespace === namespace &&
        equal(await env.hash(), hash) &&
        (this.#anchors.has(stream) || this.#anchors.size < 256),
    );
    const author = a.readAuthor(c, hash);
    requireValue(
      author.ownerDevice &&
        (await strictVerify(
          author.key,
          env.signature(),
          await signatureInput(env.header(), env.ciphertext()),
        )),
    );
    this.#anchors.set(stream, { seq, envelope: env });
  }
  async streams(
    value: unknown,
  ): Promise<{ checkpoints: AdmittedUpdate[]; updates: AdmittedUpdate[] }> {
    requireValue(Array.isArray(value) && value.length <= 256);
    const updates: AdmittedUpdate[] = [],
      checkpoints: AdmittedUpdate[] = [];
    let count = 0;
    for (const stream of value) {
      // An anchor frame carries one more key, so a reader that does not know folds refuses it.
      const anchored =
        typeof stream === 'object' && stream !== null && Object.hasOwn(stream, 'anchor');
      exactKeys(
        stream,
        anchored
          ? ['streamId', 'namespace', 'checkpoint', 'anchor', 'tail']
          : ['streamId', 'namespace', 'checkpoint', 'tail'],
      );
      requireValue(
        typeof stream.streamId === 'string' &&
          ['content', 'own'].includes(stream.namespace as string),
      );
      generatedId(stream.streamId);
      requireValue(Array.isArray(stream.tail) && stream.tail.length <= 256);
      count += stream.tail.length + (stream.checkpoint === null && !anchored ? 0 : 1);
      requireValue(count <= 1);
      if (anchored) {
        requireValue(stream.checkpoint === null && stream.tail.length === 0);
        await this.#admitAnchor(
          stream.streamId,
          stream.anchor as ObjectEntry,
          stream.namespace as Namespace,
        );
        continue;
      }
      if (stream.checkpoint !== null) {
        const update = await this.admit(
          stream.streamId,
          stream.checkpoint as ObjectEntry,
          'checkpoint',
          stream.namespace as Namespace,
        );
        if (update) checkpoints.push(update);
      }
      for (const entry of stream.tail) {
        const update = await this.admit(
          stream.streamId,
          entry as ObjectEntry,
          'update',
          stream.namespace as Namespace,
        );
        if (update) updates.push(update);
      }
    }
    return { checkpoints, updates };
  }
}
