import { decimal, exactKeys, generatedId, requireValue, spaceId, text } from '@tmt/colab-client';
import type { OwnState } from './fold-protocol.js';
import {
  foldThreadStatus,
  statusKey,
  validateStatus,
  type ThreadStatusRecord,
  type ThreadStatusView,
  type ThreadNotificationRecord,
} from './thread-status.js';

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
export interface DiscussionRecordScope extends DiscussionScope {
  version: 1;
  senderDevice: string;
  revision: string;
  deleted: boolean;
  deviceName: string;
  at: string;
}
export interface ThreadRecord extends DiscussionRecordScope {
  kind: 'thread';
  threadId: string;
  anchor: QuoteSelector | null;
  resolved: boolean;
}
export interface CommentRecord extends DiscussionRecordScope {
  kind: 'comment';
  messageId: string;
  thread: DiscussionRef;
  body: string;
}
export type DiscussionRecord =
  | ThreadRecord
  | CommentRecord
  | ThreadStatusRecord
  | ThreadNotificationRecord;
export interface CommentView extends CommentRecord {
  ref: DiscussionRef;
}
export interface ThreadView extends ThreadRecord {
  ref: DiscussionRef;
  comments: CommentView[];
  status?: ThreadStatusView;
  statuses?: ThreadStatusView[];
  notifications?: ThreadNotificationRecord[];
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
  if (record.kind === 'thread-status' || record.kind === 'thread-notification')
    return statusKey(record);
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
  } else if (r.kind === 'thread-status' || r.kind === 'thread-notification') {
    requireValue(root === 'messages');
    validateStatus(r);
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
function latest<T extends ThreadRecord | CommentRecord>(records: T[]): T | undefined {
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
  const statuses: Omit<ThreadStatusView, 'depth'>[] = [];
  const notifications: ThreadNotificationRecord[] = [];
  for (const [writer, roots] of Object.entries(own)) {
    if (!signingKey(writer)) continue;
    const groups = new Map<string, (ThreadRecord | CommentRecord)[]>();
    for (const root of ['threads', 'messages'] as const) {
      for (const [key, value] of Object.entries(roots[root])) {
        if (
          !value ||
          typeof value !== 'object' ||
          Array.isArray(value) ||
          !['thread', 'comment', 'thread-status', 'thread-notification'].includes(
            String(value.kind),
          )
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
          if (value.kind === 'thread-notification') {
            notifications.push(structuredClone(value));
            continue;
          }
          if (value.kind === 'thread-status') {
            statuses.push({ ...structuredClone(value), ref: { writer, id: value.actionId } });
            continue;
          }
          if (value.kind !== 'thread' && value.kind !== 'comment') continue;
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
  for (const thread of threads.values()) {
    Object.assign(thread, foldThreadStatus(thread, thread.ref.writer, statuses));
    const failures = notifications.filter((notification) =>
      thread.statuses?.some(
        (status) =>
          status.ref.writer === notification.status.writer &&
          status.ref.id === notification.status.id &&
          status.recipients.some((recipient) => recipient.operationId === notification.operationId),
      ),
    );
    if (failures.length) thread.notifications = failures;
  }
  return [...threads.values()].sort((a, b) => refKey(a.ref).localeCompare(refKey(b.ref)));
}

/** Chat is a designated writer-owned page thread, not a separate record kind. */
export function isChatThread(thread: ThreadView) {
  return thread.threadId === thread.ref.writer && thread.anchor === null;
}
