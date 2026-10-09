import { attachment, decimal, digest, requireValue } from '@tmt/colab-client';
import { readFailure } from './attachments.js';
import type { OwnState } from './fold-protocol.js';

/** All included attachment bytes of one export never exceed this many (native `TOTAL_BYTES`). */
export const ATTACHMENT_TOTAL_BYTES = 64 * 1024 * 1024;
/** One export spends at most this long reading attachments; later entries are unavailable. */
export const ATTACHMENT_BUDGET_MS = 120_000;

export type ExportAttachmentState = 'included' | 'missing' | 'unavailable';
export type ExportAttachmentReason = 'denied' | 'changed' | 'too-large' | 'unavailable';
/** One attachment of an export, read under current authority. `bytes` is present exactly when
 * the state is `included`; nothing of an object is disclosed otherwise. */
export interface ExportAttachment {
  readonly attachmentId: string;
  readonly source: 'document' | 'message';
  readonly reference: attachment.AttachmentSelector;
  readonly filename: string;
  readonly mediaType: string;
  readonly plaintextBytes: string;
  readonly state: ExportAttachmentState;
  readonly reason?: ExportAttachmentReason;
  readonly bytes?: Uint8Array;
}
export interface GatherScope {
  spaceId: string;
  pageId: string;
  epoch: string;
  /** The page revision a document attachment is fenced by. */
  revision: string;
  document: readonly attachment.AttachmentDescriptor[];
  own: OwnState;
  totalBytes?: number;
  budgetMs?: number;
}
const hex = (bytes: Uint8Array) => [...bytes].map((b) => b.toString(16).padStart(2, '0')).join('');
interface Candidate {
  source: 'document' | 'message';
  reference: attachment.AttachmentSelector;
  descriptor: attachment.AttachmentDescriptor;
}
const compareText = (a: string, b: string) => (a < b ? -1 : a > b ? 1 : 0);

/** Document attachments in list order, then the live messages of the exported epoch ordered by
 * writer, message and revision (the native `candidates`). Superseded and deleted messages list
 * nothing. A descriptor of another space or page refuses the whole export. */
async function candidates(scope: GatherScope): Promise<Candidate[]> {
  const found: Candidate[] = [];
  const add = async (
    source: 'document' | 'message',
    list: readonly unknown[] | undefined,
    reference: (id: string, descriptorHash: string) => attachment.AttachmentSelector,
  ) => {
    for (const raw of list ?? []) {
      const descriptor = attachment.attachmentDescriptor(raw as attachment.AttachmentDescriptor);
      requireValue(descriptor.space === scope.spaceId && descriptor.page === scope.pageId);
      found.push({
        source,
        reference: reference(
          descriptor.attachmentId,
          hex(await attachment.attachmentHash(descriptor)),
        ),
        descriptor,
      });
    }
  };
  await add('document', scope.document, (attachmentId, descriptorHash) => ({
    kind: 'document-current',
    attachmentId,
    descriptorHash,
    contentRevision: scope.revision,
  }));
  for (const writer of Object.keys(scope.own).sort(compareText)) {
    const live: { id: string; revision: string; message: Record<string, unknown> }[] = [];
    for (const [key, message] of Object.entries(scope.own[writer].messages)) {
      const [id, revision] = key.split(':');
      if (
        typeof message === 'object' &&
        message !== null &&
        !Array.isArray(message) &&
        message.kind === 'comment' &&
        message.deleted === false &&
        message.senderDevice === writer &&
        message.messageId === id &&
        message.revision === revision &&
        message.spaceId === scope.spaceId &&
        message.pageId === scope.pageId &&
        message.epoch === scope.epoch
      )
        live.push({ id, revision, message });
    }
    live.sort(
      (a, b) =>
        compareText(a.id, b.id) ||
        Number(BigInt(decimal(a.revision)) - BigInt(decimal(b.revision))),
    );
    for (const { id, revision, message } of live)
      await add(
        'message',
        message.attachments as readonly unknown[] | undefined,
        (attachmentId, descriptorHash) => ({
          kind: 'message',
          writerId: writer,
          messageId: id,
          messageRevision: revision,
          attachmentId,
          descriptorHash,
        }),
      );
  }
  return found;
}

/** Every attachment of the page and the included bytes, each read through `read` (verified
 * plaintext or an `AttachmentReadError`). A length the descriptor does not declare is never
 * disclosed. The total cap and the time budget leave later entries unread. */
export async function gatherAttachments(
  scope: GatherScope,
  read: (selector: attachment.AttachmentSelector) => Promise<Uint8Array>,
): Promise<ExportAttachment[]> {
  const totalBytes = scope.totalBytes ?? ATTACHMENT_TOTAL_BYTES;
  const budgetMs = scope.budgetMs ?? ATTACHMENT_BUDGET_MS;
  const started = performance.now();
  const entries: ExportAttachment[] = [];
  let total = 0;
  for (const { source, reference, descriptor } of await candidates(scope)) {
    const base = {
      attachmentId: descriptor.attachmentId,
      source,
      reference,
      filename: descriptor.filename,
      mediaType: descriptor.mediaType,
      plaintextBytes: descriptor.plaintextBytes,
    };
    const declared = Number(decimal(descriptor.plaintextBytes));
    if (total + declared > totalBytes) {
      entries.push({ ...base, state: 'unavailable', reason: 'too-large' });
    } else if (performance.now() - started >= budgetMs) {
      entries.push({ ...base, state: 'unavailable', reason: 'unavailable' });
    } else {
      try {
        const bytes = await read(reference);
        if (bytes.length !== declared) {
          entries.push({ ...base, state: 'unavailable', reason: 'unavailable' });
        } else {
          total += declared;
          entries.push({ ...base, state: 'included', bytes });
        }
      } catch (error) {
        const reason = readFailure(error);
        entries.push(
          reason === 'not-found'
            ? { ...base, state: 'missing' }
            : { ...base, state: 'unavailable', reason },
        );
      }
    }
  }
  return entries;
}
/** `sha256` of an included entry's bytes, as the manifest lists it. */
export async function attachmentDigest(bytes: Uint8Array): Promise<string> {
  return hex(await digest(bytes));
}
