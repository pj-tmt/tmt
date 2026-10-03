import {
  binary,
  concat,
  decodeText,
  decodeHeader,
  Envelope,
  MAX_ENVELOPE_JSON,
  encodeBinary,
  exactKeys,
  requireValue,
  strictJson,
  text,
} from '@tmt/colab-client';
import type { Admission } from './admission.js';
import { UPDATE_ENVELOPE_BYTES } from './objects.js';

/** One bounded transfer, absolute deadline, no partially admitted envelope. */
export class Frames {
  #pending: {
    frame: Record<string, unknown>;
    target: Record<string, unknown>;
    id: string;
    hash: string;
    parts: Uint8Array[];
    size: number;
    count: number | null;
    limit: number;
  } | null = null;
  #timer: ReturnType<typeof setTimeout> | undefined;
  constructor(
    readonly admission: Admission,
    readonly fail: (error: Error) => void,
  ) {}
  close() {
    clearTimeout(this.#timer);
    this.#pending = null;
  }
  receive(raw: unknown): Record<string, unknown> | null {
    requireValue(typeof raw === 'string');
    const value = strictJson(text(raw), 64 * 1024, true);
    requireValue(value !== null && typeof value === 'object' && !Array.isArray(value));
    const frame = value as Record<string, unknown>,
      a = this.admission;
    requireValue(
      frame.version === 1 &&
        frame.space === a.space &&
        frame.page === a.page &&
        frame.epoch === a.epoch,
    );
    if (frame.type === 'chunk') {
      exactKeys(frame, [
        'version',
        'type',
        'space',
        'page',
        'epoch',
        'objectId',
        'envelopeHash',
        'index',
        'count',
        'bytes',
      ]);
      const p = this.#pending;
      requireValue(
        p !== null &&
          frame.objectId === p.id &&
          frame.envelopeHash === p.hash &&
          Number.isSafeInteger(frame.count) &&
          (frame.count as number) > 0 &&
          (frame.count as number) <= Math.ceil(p.limit / (32 * 1024)) &&
          frame.index === p.parts.length &&
          (frame.index as number) < (frame.count as number) &&
          (p.count === null || p.count === frame.count),
      );
      const bytes = binary(frame.bytes, 32 * 1024);
      requireValue(
        bytes.length > 0 &&
          (frame.index === (frame.count as number) - 1 || bytes.length === 32 * 1024),
      );
      p.size += bytes.length;
      requireValue(p.size <= p.limit);
      p.count = frame.count as number;
      p.parts.push(bytes);
      if (p.parts.length !== p.count) return null;
      const joined = concat(...p.parts);
      // Valid UTF-8 and JSON are subsequently checked by the envelope model.
      decodeText(joined);
      requireValue(decodeHeader(Envelope.fromJson(joined).header()).objectId === p.id);
      p.target.envelope = encodeBinary(joined);
      const complete = p.frame;
      this.close();
      return complete;
    }
    if (frame.type === 'error') {
      this.close();
      return frame;
    }
    requireValue(this.#pending === null);
    let target: Record<string, unknown> | undefined;
    let limit = UPDATE_ENVELOPE_BYTES;
    if (frame.type === 'broadcast') target = frame;
    if (frame.type === 'catchup') {
      requireValue(Array.isArray(frame.streams) && frame.streams.length <= 256);
      if (Object.hasOwn(frame, 'baselineObject')) {
        requireValue(typeof frame.baseline === 'string' && frame.streams.length === 0);
        binary(frame.baseline, 8 * 1024);
        exactKeys(frame.baselineObject, ['envelopeHash', 'envelope']);
        target = frame.baselineObject;
        limit = MAX_ENVELOPE_JSON;
      }
      let count = 0;
      for (const stream of frame.streams) {
        exactKeys(stream, ['streamId', 'namespace', 'checkpoint', 'tail']);
        if (stream.namespace !== 'content' || stream.checkpoint !== null)
          throw new Error('Checkpoint or own-namespace loading is not available yet');
        requireValue(Array.isArray(stream.tail) && stream.tail.length <= 256);
        count += stream.tail.length;
        requireValue(count <= 1);
        if (stream.tail.length) target = stream.tail[0] as Record<string, unknown>;
      }
    }
    if (target && typeof target.envelope !== 'string') {
      exactKeys(target.envelope, ['objectId']);
      const id = target.envelope.objectId;
      requireValue(
        typeof id === 'string' &&
          /^[0-9a-f]{64}$/.test(id) &&
          typeof target.envelopeHash === 'string',
      );
      binary(target.envelopeHash, 32, 32);
      this.#pending = {
        frame,
        target,
        id,
        hash: target.envelopeHash,
        parts: [],
        size: 0,
        count: null,
        limit,
      };
      this.#timer = setTimeout(() => {
        this.close();
        this.fail(new Error('Object acquisition timed out'));
      }, 2000);
      return null;
    }
    return frame;
  }
}
