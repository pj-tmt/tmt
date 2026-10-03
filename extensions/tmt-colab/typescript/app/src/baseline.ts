import {
  binary,
  decodeHeader,
  Envelope,
  equal,
  exactKeys,
  MAX_ENVELOPE_JSON,
  MAX_PLAINTEXT,
  payload,
  requireValue,
  strictJson,
  text,
} from '@tmt/colab-client';
import type { Admission } from './admission.js';
import { BASELINE_UPDATE_BYTES, SOURCE_BYTES, validateProjection } from './fold-protocol.js';

export interface BaselineObject {
  envelopeHash: string;
  envelope: string;
}
export interface BaselineInput {
  source: string;
  title: string;
  update: Uint8Array;
  sourceDigest: Uint8Array;
  commitment: Uint8Array;
}
/** No foreign Yjs decoding here: the parent admits exact encrypted bytes and
 * the signed descriptor before handing the one canonical update to the Worker. */
export async function openBaseline(
  a: Admission,
  descriptor: string,
  object: BaselineObject,
): Promise<BaselineInput> {
  const d = payload.decodeBaseline(binary(descriptor, 8 * 1024));
  const key = a.baseline(d);
  exactKeys(object, ['envelopeHash', 'envelope']);
  const env = Envelope.fromJson(binary(object.envelope, MAX_ENVELOPE_JSON));
  const h = decodeHeader(env.header()).context;
  requireValue(
    a.root !== null &&
      a.head !== null &&
      h.space === a.space &&
      h.page === a.page &&
      h.epoch === a.epoch &&
      h.kind === 'html' &&
      h.namespace === 'content' &&
      h.authorDevice === a.head.ownerMember.id &&
      h.streamSeq === '0' &&
      h.membershipRevision === d.membershipRevision &&
      equal(h.prevHash, new Uint8Array(32)) &&
      object.envelopeHash === d.objectEnvelopeHash &&
      equal(await env.hash(), binary(d.objectEnvelopeHash, 32, 32)),
  );
  const plain = await env.open(h, a.root, key);
  try {
    // Bound JSON before parsing; the model's non-update ceiling is 16 MiB.
    const body = strictJson(plain, MAX_PLAINTEXT, true);
    exactKeys(body, ['source', 'update']);
    requireValue(typeof body.source === 'string' && text(body.source).length <= SOURCE_BYTES);
    validateProjection({ source: body.source, title: d.title });
    const update = binary(body.update, BASELINE_UPDATE_BYTES);
    requireValue(update.length > 0);
    return {
      source: body.source,
      title: d.title,
      update,
      sourceDigest: binary(d.sourceDigest, 32, 32),
      commitment: binary(d.baselineCommitment, 32, 32),
    };
  } finally {
    plain.fill(0);
  }
}
