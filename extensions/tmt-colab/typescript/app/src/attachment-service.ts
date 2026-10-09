import {
  attachment,
  digest,
  Envelope,
  decodeHeader,
  encodeBinary,
  requireValue,
} from '@tmt/colab-client';
import { FrozenAttachmentUpload, ATTACHMENT_CHUNK_BYTES } from './attachment-channel.js';
import type { AttachmentObjectOutcome } from './attachment-channel.js';
import {
  AdmittedAttachmentRead,
  currentBase,
  prepareAttachmentPublication,
} from './attachments.js';
import type { Connection } from './connection.js';
import type { OwnRecord } from './fold-protocol.js';
import { hex } from './export.js';

/** Each object request carries its own bound; a late reply ends the connection's generation. */
const REQUEST_MS = 15_000;
const READ_MS = 30_000;

export interface AttachmentInput {
  filename: string;
  mediaType: string;
  bytes: Uint8Array;
}
/** A committed original held for publication. Not proof of publication or read authority. */
export interface StoredAttachment {
  original: FrozenAttachmentUpload;
  filename: string;
  size: number;
}
export type RefusalReason =
  | 'denied'
  | 'unavailable'
  | 'invalid'
  | 'conflict'
  | 'capacity'
  | 'too-large';
/** `refused` never took effect. `unknown` may have: only status of the same original
 * decides. `gone` is a status answer that the original is no longer stored. */
export type UploadFailure =
  | { kind: 'refused'; reason: RefusalReason }
  | { kind: 'unknown'; original: FrozenAttachmentUpload }
  | { kind: 'gone' };
export class AttachmentUploadError extends Error {
  constructor(readonly failure: UploadFailure) {
    super(`Attachment ${failure.kind}`);
  }
}
/** The page moved after the original captured its base; the user attaches again. */
export class AttachmentStaleError extends Error {
  constructor(readonly attachmentIds: readonly string[]) {
    super('Attachment base is stale');
  }
}

export interface AttachmentBinding {
  /** Upload availability and the effective payload bound, or why there is none. */
  limits(): Promise<{ payloadBytes: number } | { reason: RefusalReason }>;
  upload(
    input: AttachmentInput,
    messageId: string,
    progress: (sent: number, total: number) => void,
  ): Promise<StoredAttachment>;
  /** Status of the same frozen original; continues a pending transfer, never replays one. */
  resume(
    original: FrozenAttachmentUpload,
    input: Pick<AttachmentInput, 'filename'> & { size: number },
    progress: (sent: number, total: number) => void,
  ): Promise<StoredAttachment>;
  /** Best effort release of the same original; failure is not reported as removal. */
  discard(original: FrozenAttachmentUpload): Promise<void>;
  /** The verified plaintext of one attachment on a message in this conversation. */
  open(message: MessageReference, descriptor: attachment.AttachmentDescriptor): Promise<Uint8Array>;
}
export interface MessageReference {
  writer: string;
  messageId: string;
  revision: string;
}

export interface AttachmentServiceOptions {
  connection(): Promise<Connection>;
  available(): boolean;
  sharing: string;
}

/** Owns the upload, status, discard and read protocol over the connection's object
 * channel. It publishes nothing: ThreadStore adds `publication()` records to the
 * same batch as the message that references them. */
export class AttachmentService implements AttachmentBinding {
  constructor(private options: AttachmentServiceOptions) {}
  async #channel() {
    requireValue(this.options.available());
    const c = await this.options.connection();
    requireValue(c.active);
    return c;
  }
  #bound(ms = REQUEST_MS) {
    return performance.now() + ms;
  }
  async limits(): Promise<{ payloadBytes: number } | { reason: RefusalReason }> {
    try {
      const c = await this.#channel(),
        outcome = await c.attachmentObjects.config(this.#bound());
      if ('error' in outcome) return { reason: refusal(outcome) };
      const { capabilities, limits } = outcome.ok as {
        capabilities: Record<string, unknown>;
        limits: Record<string, unknown>;
      };
      if (!capabilities.immutableCreate || !capabilities.recoverByOriginalId)
        return { reason: 'unavailable' as const };
      const payload = Number(limits.payloadBytes);
      requireValue(Number.isSafeInteger(payload) && payload > 0);
      return { payloadBytes: Math.min(payload, attachment.ATTACHMENT_PAYLOAD_BYTES) };
    } catch {
      return { reason: 'unavailable' as const };
    }
  }
  async upload(
    input: AttachmentInput,
    messageId: string,
    progress: (sent: number, total: number) => void,
  ) {
    const limit = await this.limits();
    if ('reason' in limit)
      throw new AttachmentUploadError({ kind: 'refused', reason: limit.reason });
    let original: FrozenAttachmentUpload;
    try {
      original = await this.#seal(input, messageId, limit.payloadBytes);
    } catch (error) {
      if (error instanceof AttachmentUploadError) throw error;
      throw new AttachmentUploadError({ kind: 'refused', reason: 'unavailable' });
    }
    const stored = { original, filename: input.filename, size: input.bytes.length };
    return this.#drive(stored, 'begin', progress);
  }
  async resume(
    original: FrozenAttachmentUpload,
    input: Pick<AttachmentInput, 'filename'> & { size: number },
    progress: (sent: number, total: number) => void,
  ) {
    return this.#drive(
      { original, filename: input.filename, size: input.size },
      'status',
      progress,
    );
  }
  async #seal(input: AttachmentInput, messageId: string, payloadLimit: number) {
    const c = await this.#channel(),
      a = c.admission;
    requireValue(!a.reader && a.head !== null && a.root !== null);
    a.validatePage(this.options.sharing);
    const device = a.registration.deviceId,
      membership = a.head.revision.toString();
    a.author(device, membership);
    const envelope = await Envelope.seal(
        {
          space: a.space,
          page: a.page,
          epoch: a.epoch,
          kind: 'asset',
          namespace: 'own',
          authorDevice: device,
          membershipRevision: membership,
          streamSeq: '0',
          prevHash: new Uint8Array(32),
        },
        a.readRoot(a.epoch),
        a.registration.keys.sign,
        input.bytes,
      ),
      raw = envelope.toJson();
    if (raw.length > payloadLimit)
      throw new AttachmentUploadError({ kind: 'refused', reason: 'too-large' });
    const descriptor = attachment.attachmentDescriptor({
      version: 1,
      attachmentId: crypto.randomUUID(),
      space: a.space,
      page: a.page,
      epoch: a.epoch,
      namespace: 'own',
      objectId: decodeHeader(envelope.header()).objectId,
      authorDevice: device,
      membershipRevision: membership,
      source: { kind: 'message', writerId: device, messageId, messageRevision: '1' },
      envelopeHash: hex(await envelope.hash()),
      signature: encodeBinary(envelope.signature()),
      payloadSha256: hex(await digest(raw)),
      payloadBytes: String(raw.length),
      plaintextBytes: String(input.bytes.length),
      filename: input.filename,
      mediaType: input.mediaType,
    });
    // The base is the connection's own revision, the one publication fences against.
    const base = await currentBase(descriptor, await c.attachmentSnapshot());
    return FrozenAttachmentUpload.capture(descriptor, base, crypto.randomUUID(), raw);
  }
  /** Begin (or ask status of) the one original, send the missing parts in order, commit.
   * A throw or `unknown` once the first request left is possibly effected. */
  async #drive(
    stored: StoredAttachment,
    first: 'begin' | 'status',
    progress: (sent: number, total: number) => void,
  ): Promise<StoredAttachment> {
    const { original } = stored,
      total = Math.ceil(Number(original.descriptor.payloadBytes) / ATTACHMENT_CHUNK_BYTES);
    const unknown = () => new AttachmentUploadError({ kind: 'unknown', original });
    // Past this point any thrown request may have taken effect, and so may a status
    // question's transfer, so none of them is ever reported as a plain refusal.
    const answer = async (outcome: Promise<AttachmentObjectOutcome>) => {
      let value: AttachmentObjectOutcome;
      try {
        value = await outcome;
      } catch {
        throw unknown();
      }
      if ('error' in value) {
        if (value.error.code === 'unknown') throw unknown();
        throw new AttachmentUploadError({ kind: 'refused', reason: refusal(value) });
      }
      return value.ok;
    };
    const c = await this.#channel().catch(() => {
      throw new AttachmentUploadError({ kind: 'refused', reason: 'unavailable' });
    });
    const channel = c.attachmentObjects;
    let state = await answer(
      first === 'begin'
        ? channel.begin(original, this.#bound())
        : channel.status(original, this.#bound()),
    );
    if (state.result === 'state') {
      // `unavailable` says nothing about the original: it stays unknown, never gone.
      if (state.state === 'unavailable') throw unknown();
      throw new AttachmentUploadError({ kind: 'gone' });
    }
    if (state.result === 'pending') {
      for (let index = state.nextIndex as number; index < total; index++) {
        progress(index, total);
        const part = await answer(channel.part(original, index, this.#bound()));
        requireValue(part.result === 'progress' && part.nextIndex === index + 1);
      }
      state = await answer(channel.commit(original, this.#bound()));
    }
    // A receipt is data about the committed bytes, never a publication permit.
    if (
      state.result !== 'committed' ||
      state.payloadSha256 !== original.descriptor.payloadSha256 ||
      String(state.payloadBytes) !== original.descriptor.payloadBytes
    )
      throw new AttachmentUploadError({ kind: 'refused', reason: 'invalid' });
    progress(total, total);
    return stored;
  }
  async discard(original: FrozenAttachmentUpload) {
    try {
      const c = await this.#channel();
      await c.attachmentObjects.discard(original, this.#bound());
    } catch {
      // Best effort: the original stays unreferenced either way.
    }
  }
  /** Records for the same batch as the message: verified committed bytes only. */
  async publication(stored: readonly StoredAttachment[]): Promise<OwnRecord[]> {
    const c = await this.#channel(),
      owner = { snapshot: (epoch?: string, bound?: number) => c.attachmentSnapshot(epoch, bound) },
      snapshot = await c.attachmentSnapshot(),
      stale: StoredAttachment[] = [];
    for (const item of stored)
      if (item.original.base !== (await currentBase(item.original.descriptor, snapshot)))
        stale.push(item);
    if (stale.length)
      throw new AttachmentStaleError(stale.map((s) => s.original.descriptor.attachmentId));
    const records: OwnRecord[] = [];
    for (const { original } of stored)
      records.push(
        await prepareAttachmentPublication(
          owner,
          original.descriptor,
          original.base,
          c.attachmentObjects.verifier(original),
          this.#bound(READ_MS),
          this.options.sharing,
        ),
      );
    return records;
  }
  async open(message: MessageReference, descriptor: attachment.AttachmentDescriptor) {
    const c = await this.#channel(),
      d = attachment.attachmentDescriptor(descriptor),
      selector = attachment.attachmentSelector({
        kind: 'message',
        writerId: message.writer,
        messageId: message.messageId,
        messageRevision: message.revision,
        attachmentId: d.attachmentId,
        descriptorHash: hex(await attachment.attachmentHash(d)),
      }),
      owner = { snapshot: (epoch?: string, bound?: number) => c.attachmentSnapshot(epoch, bound) },
      deadline = this.#bound(READ_MS),
      admitted = await AdmittedAttachmentRead.capture(
        owner,
        selector,
        deadline,
        this.options.sharing,
      );
    return admitted.disclose(
      c.attachmentObjects.read(selector, admitted.descriptor),
      this.options.sharing,
    );
  }
}

function refusal(outcome: Extract<AttachmentObjectOutcome, { error: unknown }>): RefusalReason {
  const code = outcome.error.code;
  return code === 'denied' ||
    code === 'unavailable' ||
    code === 'invalid' ||
    code === 'conflict' ||
    code === 'capacity'
    ? code
    : 'unavailable';
}
