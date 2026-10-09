import {
  binary,
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
export const UPDATE_ENVELOPE_BYTES = Math.floor(((UPDATE_BYTES + 2048) * 4) / 3) + 2048;
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
    const c = decodeHeader(env.header()).context,
      a = this.admission;
    requireValue(
      c.space === a.space &&
        c.page === a.page &&
        c.epoch === this.epoch &&
        c.authorDevice === stream &&
        c.streamSeq === entry.seq &&
        c.kind === kind &&
        (namespace === undefined || c.namespace === namespace) &&
        equal(await env.hash(), hash),
    );
    const author = a.readAuthor(c, hash),
      key = author.key;
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
    const plaintext = await env.open(c, a.readRoot(this.epoch), key);
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
      this.#ownSigningKeys.set(stream, key.slice());
      this.#ownerWriters.set(
        stream,
        (this.#ownerWriters.get(stream) ?? true) && author.ownerDevice,
      );
    }
    return { namespace: ns, writer: stream, update: plaintext };
  }
  async streams(
    value: unknown,
  ): Promise<{ checkpoints: AdmittedUpdate[]; updates: AdmittedUpdate[] }> {
    requireValue(Array.isArray(value) && value.length <= 256);
    const updates: AdmittedUpdate[] = [],
      checkpoints: AdmittedUpdate[] = [];
    let count = 0;
    for (const stream of value) {
      exactKeys(stream, ['streamId', 'namespace', 'checkpoint', 'tail']);
      requireValue(
        typeof stream.streamId === 'string' &&
          ['content', 'own'].includes(stream.namespace as string),
      );
      generatedId(stream.streamId);
      requireValue(Array.isArray(stream.tail) && stream.tail.length <= 256);
      count += stream.tail.length + (stream.checkpoint === null ? 0 : 1);
      requireValue(count <= 1);
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
