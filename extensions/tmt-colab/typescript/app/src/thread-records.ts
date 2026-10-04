import { decimal, exactKeys, generatedId, requireValue, spaceId, text } from '@tmt/colab-client';
import type { OwnState } from './fold-protocol.js';

export const COMMENT_BYTES = 16 * 1024;
export interface QuoteSelector {
  exact: string;
  prefix: string;
  suffix: string;
}
export interface DiscussionScope {
  spaceId: string;
  pageId: string;
  epoch: string;
}
export interface DiscussionRef {
  writer: string;
  id: string;
}
interface RecordScope extends DiscussionScope {
  version: 1;
  senderDevice: string;
  revision: string;
  deleted: boolean;
  deviceName: string;
  at: string;
}
export interface ThreadRecord extends RecordScope {
  kind: 'thread';
  threadId: string;
  anchor: QuoteSelector | null;
  resolved: boolean;
}
export interface CommentRecord extends RecordScope {
  kind: 'comment';
  messageId: string;
  thread: DiscussionRef;
  body: string;
}
export type DiscussionRecord = ThreadRecord | CommentRecord;
export interface CommentView extends CommentRecord {
  ref: DiscussionRef;
}
export interface ThreadView extends ThreadRecord {
  ref: DiscussionRef;
  comments: CommentView[];
}
export function validateSelector(value: unknown): asserts value is QuoteSelector {
  exactKeys(value, ['exact', 'prefix', 'suffix']);
  requireValue(typeof value.exact === 'string' && value.exact.length > 0);
  requireValue(text(value.exact).length <= COMMENT_BYTES);
  for (const context of [value.prefix, value.suffix])
    requireValue(
      typeof context === 'string' && [...context].length <= 32 && text(context).length <= 128,
    );
}
export function validateRef(value: unknown): asserts value is DiscussionRef {
  exactKeys(value, ['writer', 'id']);
  generatedId(value.writer as string);
  generatedId(value.id as string);
}
export function discussionKey(record: DiscussionRecord) {
  return `${record.kind === 'thread' ? record.threadId : record.messageId}:${record.revision}`;
}
/** Envelope verification belongs to Objects. This codec admits inert typed data,
 * never an author selected from a body or a publication capability. */
export function validateDiscussionRecord(
  root: string,
  key: string,
  value: unknown,
): asserts value is DiscussionRecord {
  requireValue(value !== null && typeof value === 'object' && !Array.isArray(value));
  const r = value as Record<string, unknown>;
  const scope = [
    'version',
    'kind',
    'spaceId',
    'pageId',
    'epoch',
    'senderDevice',
    'revision',
    'deleted',
    'deviceName',
    'at',
  ];
  if (r.kind === 'thread') {
    exactKeys(r, [...scope, 'threadId', 'anchor', 'resolved']);
    requireValue(root === 'threads' && typeof r.resolved === 'boolean');
    generatedId(r.threadId as string);
    if (r.anchor !== null) validateSelector(r.anchor);
    requireValue(!r.deleted || r.anchor === null);
  } else {
    exactKeys(r, [...scope, 'messageId', 'thread', 'body']);
    requireValue(root === 'messages' && r.kind === 'comment');
    generatedId(r.messageId as string);
    validateRef(r.thread);
    requireValue(typeof r.body === 'string' && text(r.body).length <= COMMENT_BYTES);
    requireValue(r.deleted ? r.body === '' : (r.body as string).length > 0);
  }
  requireValue(r.version === 1 && typeof r.deleted === 'boolean');
  spaceId(r.spaceId as string);
  generatedId(r.pageId as string);
  generatedId(r.senderDevice as string);
  decimal(r.epoch as string);
  decimal(r.revision as string);
  requireValue(typeof r.deviceName === 'string' && text(r.deviceName).length <= 128);
  requireValue(decimal(r.at as string, true) <= 8_640_000_000_000_000n);
  requireValue(key === discussionKey(r as unknown as DiscussionRecord));
}
function latest<T extends DiscussionRecord>(records: T[]): T | undefined {
  records.sort((a, b) => (BigInt(a.revision) < BigInt(b.revision) ? -1 : 1));
  const first = records[0];
  if (!first || first.revision !== '1' || first.deleted) return;
  let previous = first;
  for (const record of records.slice(1)) {
    if (BigInt(record.revision) !== BigInt(previous.revision) + 1n || previous.deleted) return;
    if (
      record.kind === 'comment' &&
      previous.kind === 'comment' &&
      (record.thread.writer !== previous.thread.writer || record.thread.id !== previous.thread.id)
    )
      return;
    previous = record;
  }
  return previous;
}
const refKey = (ref: DiscussionRef) => `${ref.writer}:${ref.id}`;
/** Only authenticated per-writer projections and historical envelope keys reach
 * this reader. An expired/revoked writer can remain readable, never writable. */
export function readThreads(
  own: OwnState,
  scope: DiscussionScope,
  signingKey: (writer: string) => Uint8Array | undefined,
): ThreadView[] {
  const threads = new Map<string, ThreadView>();
  const comments: CommentView[] = [];
  for (const [writer, roots] of Object.entries(own)) {
    if (!signingKey(writer)) continue;
    const groups = new Map<string, DiscussionRecord[]>();
    for (const root of ['threads', 'messages'] as const) {
      for (const [key, value] of Object.entries(roots[root])) {
        if (
          !value ||
          typeof value !== 'object' ||
          Array.isArray(value) ||
          !['thread', 'comment'].includes(String(value.kind))
        )
          continue;
        try {
          validateDiscussionRecord(root, key, value);
          requireValue(
            value.senderDevice === writer &&
              value.spaceId === scope.spaceId &&
              value.pageId === scope.pageId &&
              value.epoch === scope.epoch,
          );
          const id = value.kind === 'thread' ? value.threadId : value.messageId;
          const group = `${value.kind}:${id}`;
          groups.set(group, [...(groups.get(group) ?? []), structuredClone(value)]);
        } catch {
          // The containing stream grants no authority for a substituted scope.
        }
      }
    }
    for (const records of groups.values()) {
      const record = latest(records);
      if (!record) continue;
      if (record.kind === 'thread') {
        const ref = { writer, id: record.threadId };
        threads.set(refKey(ref), { ...record, ref, comments: [] });
      } else comments.push({ ...record, ref: { writer, id: record.messageId } });
    }
  }
  for (const comment of comments) threads.get(refKey(comment.thread))?.comments.push(comment);
  return [...threads.values()].sort((a, b) => refKey(a.ref).localeCompare(refKey(b.ref)));
}
