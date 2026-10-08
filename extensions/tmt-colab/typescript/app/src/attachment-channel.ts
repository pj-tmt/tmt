import { attachment, decimal, digest, generatedId, requireValue, text } from '@tmt/colab-client';

const POLICY_BYTES = 2048;
export const ATTACHMENT_CHUNK_BYTES = 32 * 1024;
function bounded(value: unknown): Uint8Array {
  const bytes = text(JSON.stringify(value));
  requireValue(bytes.length <= POLICY_BYTES);
  return bytes;
}
/** Exact minimal opaque binding persisted by Remote. Labels and the complete
 * descriptor remain in Colab's encrypted content/captured transfer owner. */
export async function uploadPolicy(raw: attachment.AttachmentDescriptor): Promise<Uint8Array> {
  const d = attachment.attachmentDescriptor(raw);
  return bounded({
    version: 1,
    space: d.space,
    page: d.page,
    epoch: d.epoch,
    target: d.source,
    attachmentId: d.attachmentId,
    descriptorHash: hex(await attachment.attachmentHash(d)),
    objectId: d.objectId,
    envelopeHash: d.envelopeHash,
    payloadSha256: d.payloadSha256,
    payloadBytes: d.payloadBytes,
    plaintextBytes: d.plaintextBytes,
  });
}
export function readPolicy(
  raw: attachment.AttachmentDescriptor,
  peerEpoch: string,
  selector: attachment.AttachmentSelector,
): Uint8Array {
  const d = attachment.attachmentDescriptor(raw),
    reference = attachment.attachmentSelector(selector);
  decimal(peerEpoch);
  return bounded({ version: 1, space: d.space, page: d.page, epoch: peerEpoch, reference });
}
function hex(bytes: Uint8Array) {
  return [...bytes].map((b) => b.toString(16).padStart(2, '0')).join('');
}
/** Immutable byte/target retention, not authority or proof of commit. The sync
 * peer and actual Session are freshly admitted by native callbacks for every call.
 * The caller allocates transferId once; uncertain recovery keeps this same owner. */
export class FrozenAttachmentUpload {
  private constructor(
    readonly transferId: string,
    private readonly capturedDescriptor: attachment.AttachmentDescriptor,
    readonly base: string,
    private readonly ciphertext: Uint8Array,
    private readonly capturedPolicy: Uint8Array,
  ) {}
  static async capture(
    raw: attachment.AttachmentDescriptor,
    base: string,
    transferId: string,
    bytes: Uint8Array,
  ): Promise<FrozenAttachmentUpload> {
    generatedId(transferId);
    requireValue(/^v1:[0-9a-f]{64}$/.test(base));
    const descriptor = attachment.attachmentDescriptor(raw),
      ciphertext = bytes.slice();
    requireValue(
      ciphertext.length === Number(decimal(descriptor.payloadBytes)) &&
        ciphertext.length <= attachment.ATTACHMENT_PAYLOAD_BYTES,
    );
    requireValue(hex(await digest(ciphertext)) === descriptor.payloadSha256);
    return new FrozenAttachmentUpload(
      transferId,
      descriptor,
      base,
      ciphertext,
      await uploadPolicy(descriptor),
    );
  }
  get descriptor(): attachment.AttachmentDescriptor {
    return attachment.attachmentDescriptor(this.capturedDescriptor);
  }
  /** Returns a copy; a changed draft cannot rewrite an original's status input. */
  policy(): Uint8Array {
    return this.capturedPolicy.slice();
  }
  part(index: number): Uint8Array {
    requireValue(Number.isSafeInteger(index) && index >= 0);
    const start = index * ATTACHMENT_CHUNK_BYTES;
    requireValue(Number.isSafeInteger(start) && start < this.ciphertext.length);
    return this.ciphertext.slice(start, start + ATTACHMENT_CHUNK_BYTES);
  }
}
