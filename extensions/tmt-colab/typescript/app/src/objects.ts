import {
  binary,
  decimal,
  decodeHeader,
  Envelope,
  equal,
  encodeBinary,
  exactKeys,
  generatedId,
  requireValue,
} from '@tmt/colab-client';
import { Admission } from './admission.js';
import { UPDATE_BYTES } from './fold-protocol.js';

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
// The implemented browser slice accepts update-v1 only, matching the server's inbound cap.
export const UPDATE_ENVELOPE_BYTES = Math.floor(((UPDATE_BYTES + 2048) * 4) / 3) + 2048;

/** Scoped, unpruned content chains only. Unsupported history blocks the page;
 * authenticated bytes are admitted before any foreign decoder invocation. */
export class Objects {
  #heads = new Map<string, { seq: bigint; hash: Uint8Array }>();
  #seen = new Map<string, Uint8Array>();
  constructor(readonly admission: Admission) {}
  head(stream: string) {
    const head = this.#heads.get(stream);
    return { seq: head?.seq ?? 0n, hash: head?.hash.slice() ?? new Uint8Array(32) };
  }
  cursors() {
    return [...this.#heads]
      .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
      .map(([streamId, head]) => ({
        streamId,
        namespace: 'content',
        seq: head.seq.toString(),
        envelopeHash: encodeBinary(head.hash),
      }));
  }
  async admit(stream: string, entry: ObjectEntry): Promise<Uint8Array | null> {
    exactKeys(entry, ['seq', 'envelopeHash', 'envelope']);
    generatedId(stream);
    const seq = decimal(entry.seq),
      expected = binary(entry.envelopeHash, 32, 32);
    const envelope = Envelope.fromJson(binary(entry.envelope, UPDATE_ENVELOPE_BYTES)),
      context = decodeHeader(envelope.header()).context,
      a = this.admission;
    requireValue(
      context.space === a.space &&
        context.page === a.page &&
        context.epoch === a.epoch &&
        context.authorDevice === stream &&
        context.streamSeq === entry.seq,
    );
    if (context.kind !== 'update' || context.namespace !== 'content')
      throw new Error('Checkpoint or own-namespace loading is not available yet');
    requireValue(equal(await envelope.hash(), expected) && a.root !== null);
    const plaintext = await envelope.open(
      context,
      a.root,
      a.author(stream, context.membershipRevision),
    );
    const key = `${stream}:${seq}`,
      prior = this.#seen.get(key),
      head = this.head(stream);
    if (prior) {
      requireValue(equal(prior, expected));
      plaintext.fill(0);
      return null;
    }
    requireValue(seq === head.seq + 1n && equal(context.prevHash, head.hash));
    requireValue(this.#seen.size < 4096 && (this.#heads.has(stream) || this.#heads.size < 256));
    this.#seen.set(key, expected);
    this.#heads.set(stream, { seq, hash: expected });
    return plaintext;
  }
  async streams(value: unknown): Promise<Uint8Array[]> {
    requireValue(Array.isArray(value) && value.length <= 256);
    const updates: Uint8Array[] = [];
    let objects = 0;
    for (const stream of value) {
      exactKeys(stream, ['streamId', 'namespace', 'checkpoint', 'tail']);
      requireValue(typeof stream.streamId === 'string');
      generatedId(stream.streamId);
      if (stream.namespace !== 'content' || stream.checkpoint !== null)
        throw new Error('Checkpoint or own-namespace loading is not available yet');
      requireValue(Array.isArray(stream.tail) && stream.tail.length <= 256);
      objects += stream.tail.length;
      requireValue(objects <= 1); // One server-driven catchup object per page.
      for (const entry of stream.tail) {
        const update = await this.admit(stream.streamId, entry as ObjectEntry);
        if (update) updates.push(update);
      }
    }
    return updates;
  }
}
