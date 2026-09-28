import type { ConversationState } from './conversation-state.js';

export interface ConversationCue {
  kind: 'idle' | 'waiting' | 'reply' | 'notice';
  text: string;
  requestId?: string;
}

/** Presentation over canonical history, not an execution or acknowledgment state. */
export function conversationCue(
  state: ReturnType<ConversationState['getSnapshot']>,
  submittedId?: string,
  seenReplyIds: readonly string[] = []
): ConversationCue {
  if (state.paused) return { kind: 'notice', text: 'Updates paused' };
  if (state.error) return { kind: 'notice', text: 'Could not check reply' };
  if (state.before) return { kind: 'idle', text: 'Viewing earlier messages' };
  const items = state.page?.items ?? [];
  const unseenReply = items.find(
    (item) => item.final.status === 'retained' && !seenReplyIds.includes(item.requestId)
  );
  const item =
    unseenReply ?? (submittedId ? items.find((item) => item.requestId === submittedId) : items[0]);
  if (!item) return { kind: 'idle', text: submittedId ? 'Request submitted' : 'Open chat' };
  const requestId = item.requestId;
  if (item.final.status === 'retained')
    return {
      kind: seenReplyIds.includes(requestId) ? 'idle' : 'reply',
      text: seenReplyIds.includes(requestId) ? 'Reply received' : 'Reply ready',
      requestId,
    };
  if (item.final.status === 'expired' || item.final.status === 'unavailable')
    return { kind: 'notice', text: `Reply ${item.final.status}`, requestId };
  if (item.delivery === 'definitely_failed')
    return { kind: 'notice', text: 'Not delivered', requestId };
  if (item.delivery === 'uncertain')
    return { kind: 'notice', text: 'Delivery uncertain', requestId };
  if (item.final.status === 'not_required')
    return { kind: 'idle', text: 'No reply requested', requestId };
  return { kind: 'waiting', text: 'Awaiting reply', requestId };
}
