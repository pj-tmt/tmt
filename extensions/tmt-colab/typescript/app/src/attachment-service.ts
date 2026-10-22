import {
  attachment,
  digest,
  Envelope,
  decodeHeader,
  encodeBinary,
  requireValue,
  text,
} from '@tmt/colab-client';
import { FrozenAttachmentUpload, ATTACHMENT_CHUNK_BYTES } from './attachment-channel.js';
import type { AttachmentObjectOutcome, UsageDetails } from './attachment-channel.js';
import {
  AdmittedAttachmentRead,
  AttachmentReadError,
  currentBase,
  prepareAttachmentPublication,
} from './attachments.js';
import type { Connection } from './connection.js';
import type { OwnRecord } from './fold-protocol.js';
import { hex } from './export.js';

/** An upload request carries its own bound; a late reply ends the connection's generation. */
const REQUEST_MS = 15_000;
/** A read of the page's objects has no deadline: it ends with its reply or the connection closing,
 * however large the object or busy the server (#3982). */
const NO_DEADLINE = Number.POSITIVE_INFINITY;
/** Remote's cool-down after a failed object setup; an upgrade sooner would be refused the same way. */
const OBJECT_REOPEN_MS = 1_000;

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
  /** The connection was upgraded without an object channel, and one reconnect did not fix it. */
  | 'no-object-channel'
  | 'invalid'
  | 'conflict'
  | 'capacity'
  | 'too-large';
/** `refused` never took effect. `unknown` may have: only status of the same original
 * decides. `gone` is a status answer that the original is no longer stored. */
export type UploadFailure =
  | { kind: 'refused'; reason: RefusalReason; usage?: UsageDetails }
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

/** What a sealed original will belong to: a message (own namespace, revision 1) or this
 * page's document at the exact source being attached to (content namespace). */
export type AttachmentTarget =
  | { kind: 'message'; messageId: string }
  | { kind: 'document'; source: string };
/** The exact immutable reference a read resolves: a message revision, or the document at
 * its current content revision (resolved at read time, so an earlier one never matches). */
export type AttachmentReference =
  | { kind: 'message'; writer: string; messageId: string; revision: string }
  | { kind: 'document' };

export interface AttachmentBinding {
  /** Names the disclosure now in force (epoch, admission head, connection standing). Anything
   * shown or held from a read belongs to one scope and is disposed when it changes. */
  scope?(): string;
  /** Upload availability and the effective payload bound, or why there is none. */
  limits(): Promise<{ payloadBytes: number } | { reason: RefusalReason; usage?: UsageDetails }>;
  upload(
    input: AttachmentInput,
    target: AttachmentTarget,
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
  /** Bring the connection to a state a read fenced by the page revision can start from. A
   * reopen changes the disclosure scope, which ends any read under way, so a read settles first
   * and takes its scope after. */
  settle?(): Promise<void>;
  /** The verified plaintext of one attachment of this page or of a message in it. */
  open(
    reference: AttachmentReference,
    descriptor: attachment.AttachmentDescriptor,
  ): Promise<Uint8Array>;
}

export interface AttachmentServiceOptions {
  connection(): Promise<Connection>;
  /** The connection to take the page revision from, with a checkpoint the serve announced
   * recorded: the owner of the connection reopens it, once, when it has not. Without it a
   * read can only report that the cut is still unseen. */
  settled?(): Promise<Connection>;
  /** A replacement for a connection the serve named as having no object channel, taken through
   * the connection-loss path so pending writes and drafts survive it. Without it such a
   * connection is refused as it is. */
  reopen?(from: Connection): Promise<Connection>;
  available(): boolean;
  sharing: string | readonly string[];
  scope?(): string;
}

/** The epoch and admission head a connection reads under; `closed` once it is not standing. */
export function disclosureScope(c: Connection | null | undefined): string {
  const head = c?.active ? c.admission.head : null;
  return head ? `${c!.admission.epoch}:${head.revision}:${hex(head.hash)}` : 'closed';
}

/** Owns the upload, status, discard and read protocol over the connection's object
 * channel. It publishes nothing: ThreadStore adds `publication()` records to the
 * same batch as the message that references them. */
export class AttachmentService implements AttachmentBinding {
  constructor(private options: AttachmentServiceOptions) {}
  scope() {
    return this.options.scope?.() ?? '';
  }
  async #channel() {
    requireValue(this.options.available());
    const c = await this.options.connection();
    requireValue(c.active);
    return c;
  }
  /** The connection a read fenced by the page revision takes that revision from. The revision
   * counts every checkpoint the serve holds, and a combine another device made is announced to
   * this connection as a notice only. A cut still unseen after the owner settled it once is a
   * change this read cannot get past. */
  async #fenced() {
    requireValue(this.options.available());
    const c = this.options.settled ? await this.options.settled() : await this.options.connection();
    requireValue(c.active);
    if (c.objects.hasUnseenCut()) throw new AttachmentReadError('changed');
    return c;
  }
  async settle() {
    await this.options.settled?.();
  }
  #bound(ms = REQUEST_MS) {
    return performance.now() + ms;
  }
  /** A connection the serve names as having no object channel (Remote's setup missed its budget,
   * so the upgrade carried no origin) is reopened once per attachment attempt, no earlier than
   * Remote's cool-down after the refusal; the request is then asked again on the replacement. A
   * repeat is the typed refusal. A page that never attaches never reaches this. */
  #wantsReopen(outcome: AttachmentObjectOutcome) {
    return (
      'error' in outcome &&
      outcome.error.code === 'no-object-channel' &&
      this.options.reopen !== undefined
    );
  }
  async #reopened(from: Connection) {
    await new Promise((resolve) => setTimeout(resolve, OBJECT_REOPEN_MS));
    requireValue(this.options.available());
    const replacement = await this.options.reopen!(from);
    requireValue(replacement.active);
    return replacement;
  }
  async limits(): Promise<
    { payloadBytes: number } | { reason: RefusalReason; usage?: UsageDetails }
  > {
    try {
      let c = await this.#channel(),
        outcome = await c.attachmentObjects.config(NO_DEADLINE);
      if (this.#wantsReopen(outcome)) {
        c = await this.#reopened(c);
        outcome = await c.attachmentObjects.config(NO_DEADLINE);
      }
      if ('error' in outcome) return { reason: refusal(outcome), usage: outcome.error.usage };
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
    target: AttachmentTarget,
    progress: (sent: number, total: number) => void,
  ) {
    const limit = await this.limits();
    if ('reason' in limit)
      throw new AttachmentUploadError({
        kind: 'refused',
        reason: limit.reason,
        usage: limit.usage,
      });
    let original: FrozenAttachmentUpload;
    try {
      original = await this.#seal(input, target, limit.payloadBytes);
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
  async #seal(input: AttachmentInput, target: AttachmentTarget, payloadLimit: number) {
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
          namespace: target.kind === 'document' ? 'content' : 'own',
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
      namespace: target.kind === 'document' ? 'content' : 'own',
      objectId: decodeHeader(envelope.header()).objectId,
      authorDevice: device,
      membershipRevision: membership,
      source:
        target.kind === 'document'
          ? { kind: 'document', sourceDigest: hex(await digest(text(target.source))) }
          : {
              kind: 'message',
              writerId: device,
              messageId: target.messageId,
              messageRevision: '1',
            },
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
        throw new AttachmentUploadError({
          kind: 'refused',
          reason: refusal(value),
          usage: value.error.usage,
        });
      }
      return value.ok;
    };
    const c = await this.#channel().catch(() => {
      throw new AttachmentUploadError({ kind: 'refused', reason: 'unavailable' });
    });
    let channel = c.attachmentObjects;
    const ask = (ch: Connection['attachmentObjects']) =>
      first === 'begin' ? ch.begin(original, this.#bound()) : ch.status(original, this.#bound());
    const asked = async () => {
      let outcome = await ask(channel).catch(() => {
        throw unknown();
      });
      if (this.#wantsReopen(outcome)) {
        // Nothing reached the object channel, so a reopen that fails is a plain refusal.
        const replacement = await this.#reopened(c).catch(() => {
          throw new AttachmentUploadError({ kind: 'refused', reason: 'unavailable' });
        });
        channel = replacement.attachmentObjects;
        outcome = await ask(channel).catch(() => {
          throw unknown();
        });
      }
      return outcome;
    };
    let state = await answer(asked());
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
  async publication(
    stored: readonly StoredAttachment[],
    /** The document source being written; a document descriptor bound to another one is stale. */
    source?: string,
  ): Promise<OwnRecord[]> {
    const c = await this.#channel(),
      owner = { snapshot: (epoch?: string, bound?: number) => c.attachmentSnapshot(epoch, bound) },
      snapshot = await c.attachmentSnapshot(),
      sourceDigest = source === undefined ? undefined : hex(await digest(text(source))),
      stale: StoredAttachment[] = [];
    for (const item of stored)
      if (
        item.original.base !== (await currentBase(item.original.descriptor, snapshot)) ||
        (item.original.descriptor.source.kind === 'document' &&
          item.original.descriptor.source.sourceDigest !== sourceDigest)
      )
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
          NO_DEADLINE,
          this.options.sharing,
        ),
      );
    return records;
  }
  async open(reference: AttachmentReference, descriptor: attachment.AttachmentDescriptor) {
    const c = reference.kind === 'document' ? await this.#fenced() : await this.#channel(),
      d = attachment.attachmentDescriptor(descriptor),
      descriptorHash = hex(await attachment.attachmentHash(d)),
      selector = attachment.attachmentSelector(
        reference.kind === 'document'
          ? {
              kind: 'document-current',
              attachmentId: d.attachmentId,
              descriptorHash,
              // The current revision at read time: an earlier one can never satisfy this.
              contentRevision: (await c.attachmentSnapshot()).revision,
            }
          : {
              kind: 'message',
              writerId: reference.writer,
              messageId: reference.messageId,
              messageRevision: reference.revision,
              attachmentId: d.attachmentId,
              descriptorHash,
            },
      );
    return this.read(selector);
  }
  /** The verified plaintext of one exact reference under the authority in force now. Failures
   * are `AttachmentReadError`s; anything else is unavailable. */
  async read(selector: attachment.AttachmentSelector) {
    const c = selector.kind === 'document-current' ? await this.#fenced() : await this.#channel(),
      owner = { snapshot: (epoch?: string, bound?: number) => c.attachmentSnapshot(epoch, bound) },
      deadline = NO_DEADLINE,
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
    code === 'no-object-channel' ||
    code === 'invalid' ||
    code === 'conflict' ||
    code === 'capacity'
    ? code
    : 'unavailable';
}
