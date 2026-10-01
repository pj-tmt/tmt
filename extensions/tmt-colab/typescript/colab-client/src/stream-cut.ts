import {
  copy,
  decimal,
  decodeText,
  equal,
  fields,
  frame,
  generatedId,
  namespace,
  requireValue,
  text,
  type Bytes,
} from './bytes.js';
export interface StreamCut {
  streamId: string;
  namespace: string;
  checkpointHash: Uint8Array | null;
  checkpointSeq: string;
  tailHeadSeq: string;
  tailHeadHash: Uint8Array;
}
export function input(v: StreamCut): Bytes {
  generatedId(v.streamId);
  namespace(v.namespace);
  const cp = decimal(v.checkpointSeq, true),
    tail = decimal(v.tailHeadSeq, true);
  const hash = copy(v.tailHeadHash, 32);
  requireValue(cp <= tail && (cp === 0n) === (v.checkpointHash === null));
  requireValue(tail !== 0n || hash.every((n) => n === 0));
  return frame(
    text('tmt-colab-stream-cut-v1'),
    text('1'),
    text(v.streamId),
    text(v.namespace),
    v.checkpointHash === null ? new Uint8Array() : copy(v.checkpointHash, 32),
    text(v.checkpointSeq),
    text(v.tailHeadSeq),
    hash,
  );
}
export function decode(raw: Uint8Array): StreamCut {
  const f = fields(raw, 8);
  requireValue(decodeText(f[0]) === 'tmt-colab-stream-cut-v1' && decodeText(f[1]) === '1');
  const v = {
    streamId: decodeText(f[2]),
    namespace: decodeText(f[3]),
    checkpointHash: f[4].length === 0 ? null : copy(f[4], 32),
    checkpointSeq: decodeText(f[5]),
    tailHeadSeq: decodeText(f[6]),
    tailHeadHash: copy(f[7], 32),
  };
  requireValue(equal(input(v), raw));
  return v;
}
