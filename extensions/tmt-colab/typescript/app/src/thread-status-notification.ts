import { requireValue } from '@tmt/colab-client';
import type { DiscussionRef, ThreadView } from './thread-records.js';
import { discussionRefKey, type ThreadRecipient } from './thread-status.js';
import { conversationText } from './thread-store.js';
import type { PageAsk } from './ask-panel.js';
import type { LedgerState } from './ask-records.js';

export interface StatusNotificationContext {
  status: DiscussionRef;
  recipient: ThreadRecipient;
  thread: ThreadView;
}
/** Re-admit the committed action and frozen context in the trusted parent.
 * Rectangles, renderer clicks and display labels cannot choose a recipient or
 * operation. A later valid status does not erase this action's single attempt. */
export function statusNotificationForAsk(
  threads: readonly ThreadView[],
  context: StatusNotificationContext,
  senderDevice: string,
) {
  const captured = context.thread;
  const matches = threads.filter((value) => value.threadId === captured.threadId);
  requireValue(matches.length === 1);
  const thread = matches[0];
  requireValue(
    !thread.deleted &&
      thread.revision === captured.revision &&
      discussionRefKey(thread.ref) === discussionRefKey(captured.ref),
  );
  const status = thread.statuses?.find(
    (value) => discussionRefKey(value.ref) === discussionRefKey(context.status),
  );
  requireValue(
    status !== undefined &&
      status.senderDevice === senderDevice &&
      status.actor === 'person' &&
      status.resolved,
  );
  const recipient = status.recipients.find(
    (value) => value.operationId === context.recipient.operationId,
  );
  requireValue(
    recipient !== undefined &&
      recipient.machine === context.recipient.machine &&
      recipient.agent === context.recipient.agent,
  );
  const comments = captured.comments.filter((value) => !value.deleted);
  for (const comment of comments) {
    const current = thread.comments.filter(
      (value) => discussionRefKey(value.ref) === discussionRefKey(comment.ref),
    );
    requireValue(
      current.length === 1 &&
        !current[0].deleted &&
        current[0].revision === comment.revision &&
        current[0].body === comment.body &&
        current[0].deviceName === comment.deviceName,
    );
  }
  requireValue((thread.anchor?.exact ?? '') === (captured.anchor?.exact ?? ''));
  return {
    thread: thread.threadId,
    messageIds: [],
    operationId: recipient.operationId,
    quote: captured.anchor?.exact ?? '',
    comment: conversationText(
      comments,
      thread.threadId,
      `Thread resolved by ${status.deviceName || 'a person'}.`,
      [],
    ),
  };
}

/** Missing outcomes remain uncertain after reload. Neither this projection nor
 * result re-checking authorizes a notification dispatch. */
export interface StatusNotificationOutcome {
  status: DiscussionRef;
  recipient: ThreadRecipient;
  state: LedgerState | 'unavailable';
  reason?: string | null;
  canTrack: boolean;
  reply?: string;
}
export function projectStatusNotifications(
  thread: ThreadView,
  asks: readonly PageAsk[],
): StatusNotificationOutcome[] {
  return (thread.statuses ?? []).flatMap((status) =>
    status.recipients.map((recipient) => {
      const ask = asks.find(
        (value) =>
          value.writer === status.ref.writer &&
          value.operationId === recipient.operationId &&
          value.machine === recipient.machine &&
          value.agent === recipient.agent &&
          value.thread === thread.threadId,
      );
      const failure = thread.notifications?.find(
        (value) =>
          discussionRefKey(value.status) === discussionRefKey(status.ref) &&
          value.operationId === recipient.operationId,
      );
      return {
        status: status.ref,
        recipient,
        state: ask?.state ?? (failure ? 'unavailable' : 'uncertain'),
        reason: ask ? ask.reason : failure?.reason,
        canTrack: ask?.canTrack ?? false,
        reply: ask?.reply,
      };
    }),
  );
}

export type ThreadNotificationOutcomes = ReturnType<typeof projectStatusNotifications>;
