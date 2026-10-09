import {
  attachment,
  decimal,
  digest,
  frame,
  generatedId,
  requireValue,
  spaceId,
  text,
} from '@tmt/colab-client';
import type { Admission } from './admission.js';
import type { AttachmentSnapshot } from './attachment-history.js';
import type { Objects } from './objects.js';
import type { PageView } from './transport.js';
import type { OwnRecord } from './fold-protocol.js';

const hex = (bytes: Uint8Array) => [...bytes].map((b) => b.toString(16).padStart(2, '0')).join('');
function remaining(deadline: number) {
  requireValue(Number.isFinite(deadline) && performance.now() < deadline);
}
export async function attachmentNamespace(space: string, page: string): Promise<Uint8Array> {
  spaceId(space);
  generatedId(page);
  return digest(frame(text('tmt-colab-attachment-namespace-v1'), text(space), text(page)));
}
function objectKey(d: attachment.AttachmentDescriptor) {
  return Uint8Array.from(d.objectId.match(/../g)!, (byte) => parseInt(byte, 16));
}
/** Only the internal adapter implements this; unknown/pending/mismatched
 * originals reject. Caller JSON cannot supply committed authority. */
export interface CommittedObjectVerifier {
  readCommitted(namespace: Uint8Array, key: Uint8Array, deadline: number): Promise<Uint8Array>;
}
/** A snapshot from the connection's owned Admission/Objects/Worker executor,
 * never a public renderer/selector projection. History uses the same owners. */
export interface AttachmentReadOwner {
  snapshot(
    epoch?: string,
    deadline?: number,
  ): Promise<{ admission: Admission; objects: Objects; projection: PageView; revision: string }>;
}
function reference(
  projection: PageView,
  selector: attachment.AttachmentSelector,
  revision: string,
  a: Admission,
  epoch: string,
) {
  let list: unknown;
  if (selector.kind === 'document-current') {
    requireValue(selector.contentRevision === revision);
    list = projection.attachments;
  } else {
    const message =
      projection.own?.[selector.writerId]?.messages[
        `${selector.messageId}:${selector.messageRevision}`
      ];
    if (!message || typeof message !== 'object' || Array.isArray(message)) return undefined;
    if (
      message.kind !== 'comment' ||
      message.deleted !== false ||
      message.senderDevice !== selector.writerId ||
      message.messageId !== selector.messageId ||
      message.revision !== selector.messageRevision ||
      message.spaceId !== a.space ||
      message.pageId !== a.page ||
      message.epoch !== epoch
    )
      return undefined;
    list = message.attachments;
  }
  if (!Array.isArray(list)) return undefined;
  const matches = list
    .map(attachment.attachmentDescriptor)
    .filter((d) => d.attachmentId === selector.attachmentId);
  requireValue(matches.length <= 1);
  return matches[0];
}
async function creationProof(
  projection: PageView,
  objects: Objects,
  d: attachment.AttachmentDescriptor,
) {
  requireValue(objects.statusWriter(d.authorDevice));
  const proof = attachment.attachmentPublication(
    projection.own?.[d.authorDevice]?.intents[d.attachmentId],
  );
  await attachment.publicationMatchesDescriptor(proof, d);
}
/** Immutable, generation-bound read capture. The connection's snapshot method
 * refuses once stopped; a replacement connection cannot satisfy this owner. */
export class AdmittedAttachmentRead {
  private constructor(
    private readonly capturedDescriptor: attachment.AttachmentDescriptor,
    private readonly owner: AttachmentReadOwner,
    private readonly revision: string,
    private readonly originalRevision: string,
    private readonly root: CryptoKey,
    private readonly creator: Uint8Array,
    private readonly deadline: number,
  ) {}
  get descriptor() {
    return attachment.attachmentDescriptor(this.capturedDescriptor);
  }
  static async capture(
    owner: AttachmentReadOwner,
    rawSelector: attachment.AttachmentSelector,
    deadline: number,
    sharing: string | readonly string[],
  ) {
    remaining(deadline);
    const selector = attachment.attachmentSelector(rawSelector),
      current = await owner.snapshot(undefined, deadline),
      revision = current.revision,
      a = current.admission;
    a.validateRead(sharing);
    let d = reference(current.projection, selector, revision, a, current.objects.epoch);
    if (!d && selector.kind === 'message') {
      const epoch = decimal(a.epoch),
        low = epoch > 63n ? epoch - 63n : 1n;
      for (let old = epoch - 1n; old >= low; old--) {
        remaining(deadline);
        const history = await owner.snapshot(old.toString(), deadline);
        d = reference(history.projection, selector, revision, a, old.toString());
        if (d) break;
      }
    }
    requireValue(
      d !== undefined &&
        d.space === a.space &&
        d.page === a.page &&
        hex(await attachment.attachmentHash(d)) === selector.descriptorHash,
    );
    const original = d.epoch === a.epoch ? current : await owner.snapshot(d.epoch, deadline);
    requireValue(original.admission === a && original.objects.epoch === d.epoch);
    await creationProof(original.projection, original.objects, d);
    const root = a.readRoot(d.epoch),
      creator = a.assetAuthor(d),
      admitted = new AdmittedAttachmentRead(
        d,
        owner,
        revision,
        original.revision,
        root,
        creator,
        deadline,
      );
    await admitted.recheck(sharing);
    return admitted;
  }
  async recheck(sharing: string | readonly string[]) {
    remaining(this.deadline);
    const current = await this.owner.snapshot(undefined, this.deadline);
    current.admission.validateRead(sharing, this.descriptor.epoch);
    const original =
      this.descriptor.epoch === current.admission.epoch
        ? current
        : await this.owner.snapshot(this.descriptor.epoch, this.deadline);
    requireValue(
      original.admission === current.admission && original.revision === this.originalRevision,
    );
    requireValue(
      current.revision === this.revision &&
        current.admission.readRoot(this.descriptor.epoch) === this.root,
    );
  }
  async disclose(objects: CommittedObjectVerifier, sharing: string | readonly string[]) {
    await this.recheck(sharing);
    const raw = await objects.readCommitted(
      await attachmentNamespace(this.descriptor.space, this.descriptor.page),
      objectKey(this.descriptor),
      this.deadline,
    );
    await this.recheck(sharing);
    const plaintext = await attachment.openAttachment(
      this.descriptor,
      raw,
      attachment.attachmentContext(this.descriptor),
      this.root,
      this.creator,
    );
    try {
      await this.recheck(sharing);
      return plaintext;
    } catch (error) {
      plaintext.fill(0);
      throw error;
    }
  }
}
/** The base this descriptor's creation is fenced by in `snapshot`. A document attachment binds
 * the source it was written against, so it needs the whole page revision; a message attachment
 * binds only its writer and message, so it needs the membership head and epoch it was sealed
 * under and nothing a foreign write can move. Mirrors native `current_base`. */
export async function currentBase(
  d: attachment.AttachmentDescriptor,
  snapshot: AttachmentSnapshot,
): Promise<string> {
  if (d.source.kind === 'document') return snapshot.revision;
  const a = snapshot.admission;
  requireValue(a.head !== null);
  return attachment.messageFence({
    space: a.space,
    page: a.page,
    epoch: a.epoch,
    membershipRevision: a.head.revision.toString(),
    membershipHash: Uint8Array.from(a.head.hash),
    author: d.authorDevice,
  });
}
/** No submission/sealing here. The existing Writer prepares these immutable
 * own records only after the exact committed asset and captured base pass. */
export async function prepareAttachmentPublication(
  owner: AttachmentReadOwner,
  raw: attachment.AttachmentDescriptor,
  base: string,
  objects: CommittedObjectVerifier,
  deadline: number,
  sharing: string | readonly string[],
): Promise<OwnRecord> {
  remaining(deadline);
  const d = attachment.attachmentDescriptor(raw),
    current = await owner.snapshot(undefined, deadline),
    a = current.admission;
  requireValue(!a.reader);
  a.validatePage(sharing);
  requireValue(
    (await currentBase(d, current)) === base &&
      d.space === a.space &&
      d.page === a.page &&
      d.epoch === a.epoch &&
      d.authorDevice === a.registration.deviceId &&
      d.membershipRevision === a.head!.revision.toString(),
  );
  a.author(d.authorDevice, d.membershipRevision);
  if (d.source.kind === 'document')
    requireValue(d.source.sourceDigest === hex(await digest(text(current.projection.source))));
  else requireValue(d.source.writerId === d.authorDevice);
  const root = a.readRoot(d.epoch),
    creator = a.assetAuthor(d),
    bytes = await objects.readCommitted(
      await attachmentNamespace(d.space, d.page),
      objectKey(d),
      deadline,
    ),
    plaintext = await attachment.openAttachment(
      d,
      bytes,
      attachment.attachmentContext(d),
      root,
      creator,
    );
  plaintext.fill(0);
  const fresh = await owner.snapshot(undefined, deadline);
  remaining(deadline);
  a.validatePage(sharing);
  requireValue(
    fresh.admission === a && (await currentBase(d, fresh)) === base && a.readRoot(d.epoch) === root,
  );
  const value = attachment.attachmentPublication({
    version: 1,
    kind: 'attachment-publication',
    spaceId: d.space,
    pageId: d.page,
    epoch: d.epoch,
    senderDevice: d.authorDevice,
    membershipRevision: d.membershipRevision,
    attachmentId: d.attachmentId,
    descriptorHash: hex(await attachment.attachmentHash(d)),
    source: d.source,
    baseRevision: base,
  });
  remaining(deadline);
  return { root: 'intents', key: d.attachmentId, value: JSON.parse(JSON.stringify(value)) };
}
