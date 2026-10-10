import {
  attachment,
  decimal,
  exactKeys,
  generatedId,
  requireValue,
  spaceId,
  text,
  coreId,
} from '@tmt/colab-client';
import type { OwnState } from './fold-protocol.js';
import {
  foldThreadStatus,
  statusKey,
  validateStatus,
  validateDecision,
  foldProposalDecision,
  type ProposalDecisionRecord,
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
  proposal?: Proposal;
}
export interface Proposal {
  proposalId: string;
  title: string;
  body: string;
  proposer: { machineId: string; agentId: string; label: string };
}
export function validateProposal(value: unknown): asserts value is Proposal {
  exactKeys(value, ['proposalId', 'title', 'body', 'proposer']);
  generatedId(value.proposalId as string);
  requireValue(
    typeof value.title === 'string' && value.title.length > 0 && [...value.title].length <= 200,
  );
  requireValue(
    typeof value.body === 'string' && value.body.length > 0 && text(value.body).length <= 4096,
  );
  exactKeys(value.proposer, ['machineId', 'agentId', 'label']);
  generatedId(value.proposer.machineId as string);
  coreId(value.proposer.agentId as string);
  requireValue(
    typeof value.proposer.label === 'string' &&
      value.proposer.label.length > 0 &&
      [...value.proposer.label].length <= 64,
  );
}
const proposalKey = (value?: Proposal) =>
  value
    ? JSON.stringify([
        value.proposalId,
        value.title,
        value.body,
        value.proposer.machineId,
        value.proposer.agentId,
        value.proposer.label,
      ])
    : '';
export interface CommentRecord extends DiscussionRecordScope {
  kind: 'comment';
  messageId: string;
  thread: DiscussionRef;
  body: string;
  /** Lamport position in the thread (#2442). Absent on a comment from before the field. */
  sequence?: string;
  attachments?: attachment.AttachmentDescriptor[];
}
export type DiscussionRecord =
  | ThreadRecord
  | CommentRecord
  | ThreadStatusRecord
  | ThreadNotificationRecord
  | ProposalDecisionRecord;
// Proposal decisions share the admitted status path, not the resolution field.
export interface CommentView extends CommentRecord {
  ref: DiscussionRef;
}
export interface ThreadView extends ThreadRecord {
  ref: DiscussionRef;
  comments: CommentView[];
  status?: ThreadStatusView;
  statuses?: ThreadStatusView[];
  notifications?: ThreadNotificationRecord[];
  decision?: ProposalDecisionRecord;
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
  if (record.kind === 'proposal-decision') return `${record.actionId}:proposal-decision`;
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
    exactKeys(r, [
      ...scope,
      'threadId',
      'anchor',
      'resolved',
      ...(Object.hasOwn(r, 'proposal') ? ['proposal'] : []),
    ]);
    requireValue(root === 'threads' && typeof r.resolved === 'boolean');
    generatedId(r.threadId as string);
    if (r.anchor !== null) validateSelector(r.anchor);
    requireValue(!r.deleted || r.anchor === null);
    if (Object.hasOwn(r, 'proposal')) {
      validateProposal(r.proposal);
      requireValue(
        r.proposal.proposalId === r.threadId &&
          r.proposal.proposalId !== r.senderDevice &&
          r.anchor === null,
      );
    }
  } else if (r.kind === 'proposal-decision') {
    requireValue(root === 'messages');
    validateDecision(r);
  } else if (r.kind === 'thread-status' || r.kind === 'thread-notification') {
    requireValue(root === 'messages');
    validateStatus(r);
  } else {
    exactKeys(r, [
      ...scope,
      'messageId',
      'thread',
      'body',
      ...(Object.hasOwn(r, 'sequence') ? ['sequence'] : []),
      ...(Object.hasOwn(r, 'attachments') ? ['attachments'] : []),
    ]);
    requireValue(root === 'messages' && r.kind === 'comment');
    generatedId(r.messageId as string);
    validateRef(r.thread);
    if (Object.hasOwn(r, 'sequence')) validateSequence(r.sequence);
    requireValue(typeof r.body === 'string' && text(r.body).length <= COMMENT_BYTES);
    const attachments = Object.hasOwn(r, 'attachments')
      ? attachment.attachmentList(
          r.attachments,
          attachment.MESSAGE_ATTACHMENTS,
          r.spaceId as string,
          r.pageId as string,
        )
      : [];
    requireValue(
      r.deleted
        ? r.body === '' && attachments.length === 0
        : (r.body as string).length > 0 || attachments.length > 0,
    );
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
export const MAX_SEQUENCE = Number.MAX_SAFE_INTEGER;
/** A canonical decimal in 1..2^53-1: display order only, never authority. */
function validateSequence(value: unknown): asserts value is string {
  requireValue(typeof value === 'string' && /^[1-9][0-9]*$/.test(value));
  requireValue(BigInt(value) <= BigInt(MAX_SEQUENCE));
}
const sequenceOf = (comment: Pick<CommentRecord, 'sequence'>) => Number(comment.sequence ?? 0);
export interface CommentOrderKey {
  sequence: number;
  writer: string;
  id: string;
}
/** The one comment order for live, reload, Chat, export and the Ask context: Lamport
 * sequence, then writer and ID bytes (identifiers are ASCII, so UTF-16 order is byte
 * order). A comment without a sequence reads as 0. Timestamps never decide. */
export function compareCommentOrder(a: CommentOrderKey, b: CommentOrderKey) {
  return (
    a.sequence - b.sequence ||
    (a.writer < b.writer ? -1 : a.writer > b.writer ? 1 : 0) ||
    (a.id < b.id ? -1 : a.id > b.id ? 1 : 0)
  );
}
const orderKey = (comment: CommentView): CommentOrderKey => ({
  sequence: sequenceOf(comment),
  writer: comment.ref.writer,
  id: comment.ref.id,
});
const compareComments = (a: CommentView, b: CommentView) =>
  compareCommentOrder(orderKey(a), orderKey(b));
/** The sequence a new comment takes: one past the highest in the thread, tombstones included.
 * A writer who claimed the maximum cannot block replies; they tie at it and fall to ID order. */
export function nextSequence(comments: readonly Pick<CommentRecord, 'sequence'>[]) {
  return String(Math.min(Math.max(0, ...comments.map(sequenceOf)) + 1, MAX_SEQUENCE));
}
function latest<T extends ThreadRecord | CommentRecord>(records: T[]): T | undefined {
  records.sort((a, b) => (BigInt(a.revision) < BigInt(b.revision) ? -1 : 1));
  const first = records[0];
  if (!first || first.revision !== '1' || first.deleted) return;
  let previous = first;
  for (const record of records.slice(1)) {
    if (BigInt(record.revision) !== BigInt(previous.revision) + 1n || previous.deleted) return;
    if (
      record.kind === 'thread' &&
      first.kind === 'thread' &&
      proposalKey(record.proposal) !== proposalKey(first.proposal)
    )
      return;
    if (
      record.kind === 'comment' &&
      previous.kind === 'comment' &&
      (record.thread.writer !== previous.thread.writer ||
        record.thread.id !== previous.thread.id ||
        sequenceOf(record) !== sequenceOf(previous))
    )
      return;
    previous = record;
  }
  return previous;
}
const refKey = (ref: DiscussionRef) => `${ref.writer}:${ref.id}`;
/** Only authenticated per-writer projections and historical envelope keys reach
 * this reader. An expired/revoked writer can remain readable, never writable.
 * `statusWriter` says whether a writer may resolve threads (an owner-member device). */
export function readThreads(
  own: OwnState,
  scope: DiscussionScope,
  signingKey: (writer: string) => Uint8Array | undefined,
  statusWriter: (writer: string) => boolean,
): ThreadView[] {
  const threads = new Map<string, ThreadView>();
  const comments: CommentView[] = [];
  const statuses: Omit<ThreadStatusView, 'depth'>[] = [];
  const notifications: ThreadNotificationRecord[] = [];
  const decisions: ProposalDecisionRecord[] = [];
  for (const [writer, roots] of Object.entries(own)) {
    // A signing key exists only for a writer admitted by Admission.readAuthor. Threads and
    // comments count from any such writer, a bridge included. Status actions count only
    // from a writer with owner-member provenance, native `status_writers`.
    if (!signingKey(writer)) continue;
    const groups = new Map<string, (ThreadRecord | CommentRecord)[]>();
    for (const root of ['threads', 'messages'] as const) {
      for (const [key, value] of Object.entries(roots[root])) {
        if (
          !value ||
          typeof value !== 'object' ||
          Array.isArray(value) ||
          ![
            'thread',
            'comment',
            'thread-status',
            'thread-notification',
            'proposal-decision',
          ].includes(String(value.kind))
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
            if (!statusWriter(writer)) continue;
            statuses.push({ ...structuredClone(value), ref: { writer, id: value.actionId } });
            continue;
          }
          if (value.kind === 'proposal-decision') {
            if (statusWriter(writer)) decisions.push(structuredClone(value));
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
  for (const thread of threads.values()) thread.comments.sort(compareComments);
  for (const thread of threads.values()) {
    if (thread.proposal && !thread.deleted)
      thread.decision = foldProposalDecision(thread.ref, decisions);
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
