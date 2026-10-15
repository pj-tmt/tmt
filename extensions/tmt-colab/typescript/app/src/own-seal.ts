import { decodeHeader, encodeBinary, Envelope, requireValue } from '@tmt/colab-client';
import type { Admission } from './admission.js';
import type { ObjectEntry } from './objects.js';

/** Admission owns fresh issuer/role/lifetime checks; sealing never recreates that policy. */
export type OwnAuthor = (device: string, revision: string) => Uint8Array | Promise<Uint8Array>;

/** The same Colab own-update envelope for native Writer and the hosted original publisher. */
export async function sealOwnUpdate(
  admission: Admission,
  head: { seq: bigint; hash: Uint8Array },
  update: Uint8Array,
  author: OwnAuthor,
): Promise<ObjectEntry> {
  const a = admission;
  requireValue(a.root !== null && a.head !== null);
  await author(a.registration.deviceId, a.head.revision.toString());
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
  return {
    seq: decodeHeader(envelope.header()).context.streamSeq,
    envelopeHash: encodeBinary(await envelope.hash()),
    envelope: encodeBinary(envelope.toJson()),
  };
}
