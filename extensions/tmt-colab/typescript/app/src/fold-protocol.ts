import { validateDiscussionRecord } from './thread-records.js';
import { attachment, coreId, exactKeys, generatedId, requireValue, text } from '@tmt/colab-client';
/** Plaintext-only decoder protocol. No CryptoKeys or transport capabilities. */
// Mirror decoder.rs: UPDATE_BYTES, WRITE_TAIL_UPDATES, WRITE_TAIL_BYTES, UPDATES, STATE_BYTES and
// BASELINE_UPDATE_BYTES. Read admission is wider; prepare/check still use write limits.
export const SOURCE_BYTES = 2 * 1024 * 1024;
export const CONTENT_CHUNK_BYTES = 192 * 1024;
export const UPDATE_BYTES = 256 * 1024;
export const WRITE_TAIL_UPDATES = 200;
export const WRITE_TAIL_BYTES = 4 * 1024 * 1024;
export const READ_TAIL_UPDATES = 5_000;
export const STATE_BYTES = 24 * 1024 * 1024;
export const BASELINE_UPDATE_BYTES = STATE_BYTES + UPDATE_BYTES + 1024;
export type OwnRoot = 'threads' | 'intents' | 'messages' | 'replies';
export type FoldCommand =
  | { type: 'apply' | 'check'; updates: Uint8Array[]; own?: OwnUpdate[] }
  | { type: 'checkpoint'; update: Uint8Array; writer?: string }
  | {
      type: 'prepare-own';
      writer: string;
      records: OwnRecord[];
    }
  | {
      type: 'baseline';
      update: Uint8Array;
      title: string;
      sourceDigest: Uint8Array;
      commitment: Uint8Array;
    };
export interface OwnRecord {
  root: OwnRoot;
  key: string;
  value: JsonValue;
}
export interface CreationRecipient {
  machineId: string;
  agentId: string;
}
export function validateCreationRecipient(value: unknown): asserts value is CreationRecipient {
  requireValue(value !== null && typeof value === 'object');
  exactKeys(value, ['machineId', 'agentId']);
  requireValue(typeof value.machineId === 'string' && typeof value.agentId === 'string');
  generatedId(value.machineId);
  coreId(value.agentId);
}
export interface Projection {
  attachments?: attachment.AttachmentDescriptor[];
  creationRecipient?: CreationRecipient;
  source: string;
  title: string;
  publisherAgent?: string;
  originalAuthor?: string;
}
export interface OwnUpdate {
  writer: string;
  update: Uint8Array;
}
export interface AdmittedUpdate extends OwnUpdate {
  namespace: 'content' | 'own';
}
export type JsonValue =
  | null
  | boolean
  | number
  | string
  | JsonValue[]
  | { [key: string]: JsonValue };
export interface OwnProjection {
  threads: Record<string, JsonValue>;
  messages: Record<string, JsonValue>;
  intents: Record<string, JsonValue>;
  replies: Record<string, JsonValue>;
}
export type OwnState = Record<string, OwnProjection>;
export function validateOwn(value: unknown): asserts value is OwnState {
  requireValue(value !== null && typeof value === 'object' && !Array.isArray(value));
  requireValue(text(JSON.stringify(value)).length <= STATE_BYTES);
  const pending: unknown[] = [value];
  while (pending.length) {
    const item = pending.pop();
    if (item === null || typeof item === 'boolean') continue;
    if (typeof item === 'string') {
      text(item);
      continue;
    }
    if (typeof item === 'number') {
      requireValue(Number.isFinite(item));
      continue;
    }
    requireValue(item !== undefined && typeof item === 'object');
    if (Array.isArray(item)) for (const child of item) pending.push(child);
    else {
      requireValue(
        Object.getPrototypeOf(item) === Object.prototype || Object.getPrototypeOf(item) === null,
      );
      for (const child of Object.values(item)) pending.push(child);
    }
  }
  let threads = 0;
  const proposals = new Set<string>();
  const entries = Object.entries(value);
  requireValue(entries.length <= 256);
  for (const [writer, roots] of entries) {
    generatedId(writer);
    exactKeys(roots, ['threads', 'messages', 'intents', 'replies']);
    for (const map of Object.values(roots))
      requireValue(map !== null && typeof map === 'object' && !Array.isArray(map));
    const projection = roots as unknown as OwnProjection;
    for (const [root, map] of Object.entries(projection)) {
      for (const [key, record] of Object.entries(map)) {
        if (
          record &&
          typeof record === 'object' &&
          !Array.isArray(record) &&
          (record as Record<string, unknown>).kind === 'attachment-publication'
        ) {
          const publication = attachment.attachmentPublication(record);
          requireValue(root === 'intents' && key === publication.attachmentId);
        }
        if (
          record &&
          typeof record === 'object' &&
          !Array.isArray(record) &&
          [
            'thread',
            'comment',
            'thread-status',
            'thread-notification',
            'proposal-decision',
          ].includes(String((record as Record<string, unknown>).kind))
        )
          validateDiscussionRecord(root, key, record);
      }
    }
    threads += Object.keys(projection.threads).length;
    for (const record of Object.values(projection.threads)) {
      if (
        record &&
        typeof record === 'object' &&
        !Array.isArray(record) &&
        record.kind === 'thread' &&
        record.proposal &&
        typeof record.proposal === 'object' &&
        !Array.isArray(record.proposal)
      )
        proposals.add(`${writer}:${record.proposal.proposalId}`);
    }
    for (const message of Object.values(projection.messages)) {
      if (message !== null && typeof message === 'object' && Object.hasOwn(message, 'body')) {
        const body = (message as Record<string, unknown>).body;
        requireValue(typeof body === 'string' && text(body).length <= 16 * 1024);
      }
    }
  }
  requireValue(threads <= 1000);
  requireValue(proposals.size <= 200);
}
export interface FoldResult extends Projection {
  own: OwnState;
  update: Uint8Array;
}
export function validateProjection(value: unknown): asserts value is Projection {
  if (!value || typeof value !== 'object') throw new Error('Invalid decoder projection');
  const { source, title, publisherAgent, originalAuthor, creationRecipient } = value as Projection;
  if (Object.hasOwn(value, 'attachments'))
    attachment.attachmentList((value as Projection).attachments, attachment.DOCUMENT_ATTACHMENTS);
  if (Object.hasOwn(value, 'creationRecipient')) validateCreationRecipient(creationRecipient);
  if (
    typeof source !== 'string' ||
    typeof title !== 'string' ||
    (publisherAgent !== undefined &&
      (typeof publisherAgent !== 'string' ||
        !publisherAgent ||
        text(publisherAgent).length > 128 ||
        /[\p{Cc}]/u.test(publisherAgent))) ||
    (originalAuthor !== undefined &&
      (typeof originalAuthor !== 'string' ||
        !originalAuthor ||
        text(originalAuthor).length > 128 ||
        /[\p{Cc}]/u.test(originalAuthor))) ||
    text(source).length > SOURCE_BYTES ||
    text(title).length > UPDATE_BYTES
  )
    throw new Error('Invalid decoder projection');
}

/** Detached admitted snapshot. Batch preparation has no publication capability. */
export interface ContentSnapshot extends Projection {
  own: OwnState;
}
export interface PrepareContentCommand {
  type: 'prepare-content';
  source: string;
  base: ContentSnapshot;
  /** A typed change to `meta.attachments`, bound to `source`; absent keeps the list. */
  attachments?: attachment.DocumentChange;
}
export type ContentPreparation =
  | { kind: 'noop'; projection: ContentSnapshot }
  | { kind: 'updates'; projection: ContentSnapshot; updates: Uint8Array[] };
export type DecoderCommand = FoldCommand | PrepareContentCommand;

/** A batch's envelope-sized deltas are bounded independently of read state. */
export function validateContentUpdates(updates: unknown): asserts updates is Uint8Array[] {
  requireValue(
    Array.isArray(updates) && updates.length > 0 && updates.length <= WRITE_TAIL_UPDATES,
  );
  let bytes = 0;
  for (const update of updates) {
    requireValue(
      update instanceof Uint8Array && update.length > 0 && update.length <= UPDATE_BYTES,
    );
    bytes += update.length;
    requireValue(bytes <= WRITE_TAIL_BYTES);
  }
}
export function sameContent(a: ContentSnapshot, b: ContentSnapshot): boolean {
  return (
    a.source === b.source &&
    a.title === b.title &&
    a.publisherAgent === b.publisherAgent &&
    a.originalAuthor === b.originalAuthor &&
    Object.hasOwn(a, 'creationRecipient') === Object.hasOwn(b, 'creationRecipient') &&
    a.creationRecipient?.machineId === b.creationRecipient?.machineId &&
    a.creationRecipient?.agentId === b.creationRecipient?.agentId &&
    Object.hasOwn(a, 'attachments') === Object.hasOwn(b, 'attachments') &&
    JSON.stringify(a.attachments?.map(attachment.attachmentDescriptor)) ===
      JSON.stringify(b.attachments?.map(attachment.attachmentDescriptor)) &&
    JSON.stringify(a.own) === JSON.stringify(b.own)
  );
}
