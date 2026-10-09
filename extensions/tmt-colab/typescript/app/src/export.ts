import type { CreationRecipient } from './fold-protocol.js';
import { decimal, digest, generatedId, requireValue, spaceId, text, time } from '@tmt/colab-client';
import {
  projectConversations,
  renderConversationsMarkdown,
  serializeConversations,
} from './conversations.js';
import { validateOwn, validateProjection, type OwnState } from './fold-protocol.js';
import type { attachment } from '@tmt/colab-client';
import {
  attachmentDigest,
  type ExportAttachment,
  type ExportAttachmentReason,
  type ExportAttachmentState,
} from './export-attachments.js';

export const DISCLOSURE =
  'This creates an unencrypted copy of the page. Anyone with these files can read it.';
/** A published file name: one of the four page files, or `attachments/<attachmentId>`. */
export type ExportFile = string;
/** The four page files every export publishes, in order; attachments sit between the third and the manifest. */
export const PAGE_FILES: readonly ExportFile[] = [
  'page.html',
  'conversations.json',
  'conversations.md',
  'manifest.json',
];
/** One bundle never exceeds this many bytes of conversations (both files together). */
export const CONVERSATIONS_BYTES = 8 * 1024 * 1024;
export interface ExportView {
  creationRecipient?: CreationRecipient;
  spaceId: string;
  pageId: string;
  source: string;
  title: string;
  originalAuthor?: string;
  publisherAgent?: string;
  exportedAtMs: number;
  membershipHead: { revision: string; statementHash: string };
  epoch: string;
  /** The admitted per-writer discussion state and each writer's historical signing key. */
  own: OwnState;
  signingKeys: Record<string, Uint8Array>;
  /** Writers that may resolve threads (owner-member devices); other keyed writers stay readable. */
  statusWriters: readonly string[];
  /** Every attachment of the page, already read under current authority. (Not `attachments`:
   * that name is the projection's own descriptor list, which `validateProjection` checks.) */
  attachmentEntries: readonly ExportAttachment[];
}
export interface FileInfo {
  readonly name: ExportFile;
  readonly sizeBytes: number;
  readonly sha256: string;
}
export function hex(bytes: Uint8Array): string {
  return [...bytes].map((byte) => byte.toString(16).padStart(2, '0')).join('');
}

/** Caller admission supplies the view. Copy everything before the first await;
 * hashing cannot mix a later projection/head into this plaintext bundle. */
export async function prepareExport(input: ExportView): Promise<ExportBundle> {
  const view = {
    ...input,
    ...(input.creationRecipient === undefined
      ? {}
      : {
          creationRecipient: {
            machineId: input.creationRecipient.machineId,
            agentId: input.creationRecipient.agentId,
          },
        }),
    membershipHead: { ...input.membershipHead },
  };
  const own = structuredClone(input.own);
  const copiedAttachments = copyAttachments(input.attachmentEntries);
  const keys = new Map(
    Object.entries(input.signingKeys).map(([writer, key]) => [writer, key.slice()]),
  );
  const statusWriters = new Set(input.statusWriters);
  spaceId(view.spaceId);
  generatedId(view.pageId);
  decimal(view.epoch);
  decimal(view.membershipHead.revision);
  time(view.exportedAtMs);
  requireValue(
    typeof view.membershipHead.statementHash === 'string' &&
      /^[0-9a-f]{64}$/.test(view.membershipHead.statementHash),
  );
  validateProjection(view);
  validateOwn(own);
  const conversations = await projectConversations({
    spaceId: view.spaceId,
    pageId: view.pageId,
    title: view.title,
    epoch: view.epoch,
    membershipHead: view.membershipHead,
    own,
    signingKey: (writer) => keys.get(writer)?.slice(),
    statusWriter: (writer) => statusWriters.has(writer),
  });
  const attachments = await listAttachments(copiedAttachments);
  const html = text(view.source);
  const json = text(serializeConversations(conversations));
  const markdown = text(renderConversationsMarkdown(conversations));
  if (json.length + markdown.length > CONVERSATIONS_BYTES) throw new Error('EXPORT_TOO_LARGE');
  const describe = async (name: ExportFile, bytes: Uint8Array): Promise<FileInfo> =>
    Object.freeze({ name, sizeBytes: bytes.length, sha256: hex(await digest(bytes)) });
  const files = [
    await describe('page.html', html),
    await describe('conversations.json', json),
    await describe('conversations.md', markdown),
  ];
  // Order and spelling mirror native Manifest; export-v1.json pins identical bytes.
  const manifest = text(
    JSON.stringify({
      format: 'tmt-colab-page-export',
      version: 1,
      spaceId: view.spaceId,
      pageId: view.pageId,
      title: view.title,
      ...(view.originalAuthor === undefined ? {} : { originalAuthor: view.originalAuthor }),
      ...(view.publisherAgent === undefined ? {} : { publisherAgent: view.publisherAgent }),
      ...(view.creationRecipient === undefined
        ? {}
        : { creationRecipient: view.creationRecipient }),
      exportedAtMs: view.exportedAtMs,
      membershipHead: {
        revision: view.membershipHead.revision,
        statementHash: view.membershipHead.statementHash,
      },
      epoch: view.epoch,
      plaintext: true,
      discussions: {
        included: true,
        scope: 'current-epoch',
        format: conversations.format,
        version: conversations.version,
      },
      attachments: attachments.map((item) => item.row),
      files,
    }),
  );
  const included = attachments.filter((item) => item.bytes !== undefined);
  return new ExportBundle(
    new Map<ExportFile, Uint8Array>([
      ['page.html', html],
      ['conversations.json', json],
      ['conversations.md', markdown],
      ...included.map((item): [ExportFile, Uint8Array] => [item.row.file!, item.bytes!]),
      ['manifest.json', manifest],
    ]),
    [
      ...files,
      ...(await Promise.all(included.map((item) => describe(item.row.file!, item.bytes!)))),
      await describe('manifest.json', manifest),
    ],
    attachments.map(({ row }) => ({
      attachmentId: row.attachmentId,
      filename: row.filename,
      state: row.state,
      ...(row.reason === undefined ? {} : { reason: row.reason }),
      ...(row.file === undefined ? {} : { file: row.file }),
    })),
  );
}

/** A manifest `attachments` row, in the native serializer's field order. */
interface ManifestAttachment {
  attachmentId: string;
  source: 'document' | 'message';
  reference: attachment.AttachmentSelector;
  filename: string;
  mediaType: string;
  plaintextBytes: string;
  state: ExportAttachmentState;
  reason?: ExportAttachmentReason;
  sha256?: string;
  file?: string;
}
/** The reference with its keys in the contract's order, whatever order the caller built. */
function orderedReference(reference: attachment.AttachmentSelector): attachment.AttachmentSelector {
  return reference.kind === 'document-current'
    ? {
        kind: 'document-current',
        attachmentId: reference.attachmentId,
        descriptorHash: reference.descriptorHash,
        contentRevision: reference.contentRevision,
      }
    : {
        kind: 'message',
        writerId: reference.writerId,
        messageId: reference.messageId,
        messageRevision: reference.messageRevision,
        attachmentId: reference.attachmentId,
        descriptorHash: reference.descriptorHash,
      };
}
/** Copy the attachments before the first await, like the rest of the view: a later write to the
 * caller's bytes or references cannot reach this bundle. */
function copyAttachments(input: readonly ExportAttachment[]): ExportAttachment[] {
  return input.map((item) => ({
    ...item,
    reference: orderedReference(structuredClone(item.reference)),
    ...(item.bytes === undefined ? {} : { bytes: item.bytes.slice() }),
  }));
}
/** Check each copied attachment and describe it. An `included` entry holds exactly the bytes
 * its descriptor declares; any other state holds none. */
async function listAttachments(copied: readonly ExportAttachment[]) {
  const rows: { row: ManifestAttachment; bytes?: Uint8Array }[] = [];
  for (const item of copied) {
    generatedId(item.attachmentId);
    requireValue(
      item.reference.attachmentId === item.attachmentId &&
        (item.source === 'document') === (item.reference.kind === 'document-current'),
    );
    decimal(item.plaintextBytes);
    const row: ManifestAttachment = {
      attachmentId: item.attachmentId,
      source: item.source,
      reference: item.reference,
      filename: item.filename,
      mediaType: item.mediaType,
      plaintextBytes: item.plaintextBytes,
      state: item.state,
    };
    if (item.state === 'included') {
      requireValue(
        item.reason === undefined &&
          item.bytes !== undefined &&
          item.bytes.length === Number(item.plaintextBytes),
      );
      row.sha256 = await attachmentDigest(item.bytes!);
      row.file = `attachments/${item.attachmentId}`;
      rows.push({ row, bytes: item.bytes! });
      continue;
    }
    requireValue(item.bytes === undefined);
    requireValue(item.state === 'missing' ? item.reason === undefined : item.reason !== undefined);
    if (item.reason !== undefined) row.reason = item.reason;
    rows.push({ row });
  }
  requireValue(new Set(rows.map(({ row }) => row.attachmentId)).size === rows.length);
  return rows;
}

/** What the panel shows of one attachment; the bytes stay in the bundle. */
export interface ListedAttachment {
  readonly attachmentId: string;
  readonly filename: string;
  readonly state: ExportAttachmentState;
  readonly reason?: ExportAttachmentReason;
  /** The published path of an included attachment. */
  readonly file?: string;
}
export class ExportBundle {
  readonly files: readonly FileInfo[];
  readonly attachments: readonly ListedAttachment[];
  #bytes: Map<ExportFile, Uint8Array>;
  constructor(
    bytes: Map<ExportFile, Uint8Array>,
    files: readonly FileInfo[],
    attachments: readonly ListedAttachment[] = [],
  ) {
    this.#bytes = new Map([...bytes].map(([name, value]) => [name, value.slice()]));
    this.files = Object.freeze(files.map((file) => Object.freeze({ ...file })));
    this.attachments = Object.freeze(attachments.map((item) => Object.freeze({ ...item })));
  }
  /** The name a browser download is saved under: the attachment's own filename, or the page file. */
  downloadName(name: ExportFile): string {
    return this.attachments.find((item) => item.file === name)?.filename ?? name;
  }
  blob(name: ExportFile): Blob {
    const bytes = this.#bytes.get(name);
    requireValue(bytes !== undefined);
    return new Blob([new Uint8Array(bytes)], { type: 'application/octet-stream' });
  }
}

/** Parent-only blob download lifecycle: one URL per request, revoked after hand-off and
 * on close. A request is not evidence of a saved file. */
export class BlobDownloads {
  #urls = new Map<string, ReturnType<typeof setTimeout>>();
  #closed = false;
  request(blob: Blob, filename: string): void {
    requireValue(!this.#closed);
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement('a');
    try {
      anchor.href = url;
      anchor.download = filename;
      anchor.hidden = true;
      document.body.append(anchor);
      anchor.click();
      // Let the browser consume its download request before revoking the URL.
      this.#urls.set(
        url,
        setTimeout(() => this.#revoke(url), 1000),
      );
    } catch (error) {
      URL.revokeObjectURL(url);
      throw error;
    } finally {
      anchor.remove();
    }
  }
  #revoke(url: string) {
    clearTimeout(this.#urls.get(url));
    this.#urls.delete(url);
    URL.revokeObjectURL(url);
  }
  close() {
    this.#closed = true;
    for (const url of this.#urls.keys()) this.#revoke(url);
  }
}

/** The export bundle's files through the shared download lifecycle. */
export class Downloads {
  #blobs = new BlobDownloads();
  constructor(readonly bundle: ExportBundle) {}
  request(name: ExportFile): void {
    this.#blobs.request(this.bundle.blob(name), this.bundle.downloadName(name));
  }
  close() {
    this.#blobs.close();
  }
}
