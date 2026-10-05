import { coreId, exactKeys, generatedId, requireValue, text } from '@tmt/colab-client';
import type { DiscussionRef, DiscussionRecordScope, ThreadRecord } from './thread-records.js';

export interface ThreadRecipient {
  machine: string;
  agent: string;
  agentName: string;
  operationId: string;
}
export interface ThreadStatusRecord extends DiscussionRecordScope {
  kind: 'thread-status';
  actionId: string;
  thread: DiscussionRef;
  previous: DiscussionRef | null;
  resolved: boolean;
  actor: 'person' | 'agent';
  agentName: string | null;
  recipients: ThreadRecipient[];
}
export interface ThreadStatusView extends ThreadStatusRecord {
  ref: DiscussionRef;
  depth: number;
}
export interface ThreadNotificationRecord extends DiscussionRecordScope {
  kind: 'thread-notification';
  operationId: string;
  status: DiscussionRef;
  reason: 'RECIPIENT_UNAVAILABLE' | 'PREPARATION_FAILED';
}
const fields = [
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
function ref(value: unknown): asserts value is DiscussionRef {
  exactKeys(value, ['writer', 'id']);
  generatedId(value.writer as string);
  generatedId(value.id as string);
}
export function statusKey(value: ThreadStatusRecord | ThreadNotificationRecord) {
  return `${value.kind === 'thread-status' ? value.actionId : value.operationId}:${value.kind}`;
}
export function validateStatus(value: Record<string, unknown>) {
  requireValue(value.revision === '1' && value.deleted === false);
  if (value.kind === 'thread-notification') {
    exactKeys(value, [...fields, 'operationId', 'status', 'reason']);
    generatedId(value.operationId as string);
    ref(value.status);
    requireValue(value.status.writer === value.senderDevice);
    requireValue(['RECIPIENT_UNAVAILABLE', 'PREPARATION_FAILED'].includes(String(value.reason)));
    return;
  }
  exactKeys(value, [
    ...fields,
    'actionId',
    'thread',
    'previous',
    'resolved',
    'actor',
    'agentName',
    'recipients',
  ]);
  generatedId(value.actionId as string);
  ref(value.thread);
  if (value.previous !== null) ref(value.previous);
  requireValue(typeof value.resolved === 'boolean');
  requireValue(value.actor === 'person' || value.actor === 'agent');
  requireValue(
    value.agentName === null ||
      (typeof value.agentName === 'string' &&
        value.agentName.length > 0 &&
        text(value.agentName).length <= 128 &&
        !/\p{Cc}/u.test(value.agentName)),
  );
  requireValue(value.actor === 'agent' || value.agentName === null);
  requireValue(Array.isArray(value.recipients) && value.recipients.length <= 1000);
  requireValue((value.actor === 'person' && value.resolved) || value.recipients.length === 0);
  const agents = new Set<string>(),
    operations = new Set<string>();
  for (const recipient of value.recipients) {
    exactKeys(recipient, ['machine', 'agent', 'agentName', 'operationId']);
    for (const key of ['machine', 'operationId']) generatedId(recipient[key] as string);
    coreId(recipient.agent as string);
    requireValue(
      typeof recipient.agentName === 'string' && text(recipient.agentName).length <= 128,
    );
    const target = `${recipient.machine}:${recipient.agent}`;
    requireValue(!agents.has(target) && !operations.has(recipient.operationId as string));
    agents.add(target);
    operations.add(recipient.operationId as string);
  }
}
export const discussionRefKey = (value: DiscussionRef) => `${value.writer}:${value.id}`;

/** Only authenticated, scope-checked records enter this fold. Causal descendants
 * outrank ancestors; concurrent branches use IDs, never publisher clocks/labels.
 * Missing parents, cycles and references into other threads remain inert. */
export function foldThreadStatus(
  thread: ThreadRecord,
  writer: string,
  records: readonly Omit<ThreadStatusView, 'depth'>[],
): { resolved: boolean; status?: ThreadStatusView; statuses?: ThreadStatusView[] } {
  if (thread.deleted || (thread.threadId === writer && thread.anchor === null))
    return { resolved: thread.resolved };
  const candidates = records.filter(
    (value) => value.thread.writer === writer && value.thread.id === thread.threadId,
  );
  const children = new Map<string, Omit<ThreadStatusView, 'depth'>[]>();
  const queue: ThreadStatusView[] = [];
  for (const value of candidates) {
    if (value.previous === null) queue.push({ ...value, depth: 1 });
    else {
      const key = discussionRefKey(value.previous);
      children.set(key, [...(children.get(key) ?? []), value]);
    }
  }
  let winner: ThreadStatusView | undefined;
  for (let index = 0; index < queue.length; index++) {
    const value = queue[index];
    if (
      !winner ||
      value.depth > winner.depth ||
      (value.depth === winner.depth && discussionRefKey(value.ref) > discussionRefKey(winner.ref))
    )
      winner = value;
    for (const child of children.get(discussionRefKey(value.ref)) ?? [])
      queue.push({ ...child, depth: value.depth + 1 });
  }
  return winner
    ? {
        resolved: winner.resolved,
        status: winner,
        statuses: [...queue].sort(
          (a, b) =>
            a.depth - b.depth || (discussionRefKey(a.ref) < discussionRefKey(b.ref) ? -1 : 1),
        ),
      }
    : { resolved: thread.resolved };
}
