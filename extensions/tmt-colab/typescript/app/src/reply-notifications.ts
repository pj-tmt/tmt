import type { PageAsk } from './ask-panel.js';
import { conversationRecordKey } from './conversation-record-key.js';
import { conversationAsks } from './thread-store.js';
import {
  isChatThread,
  isOwnDiscussionWriter,
  type DiscussionRef,
  type ThreadView,
} from './thread-records.js';
import { text } from './strings.js';

export interface ReplySources {
  asks?: readonly PageAsk[];
  threads?: readonly ThreadView[];
  askUnavailable?: boolean;
}
export interface ReplyNotification {
  id: string;
  destination: 'Chat' | 'Thread' | 'Answer' | 'Proposal';
  questionId?: string;
  firstLine: string;
  quote?: string;
  thread?: DiscussionRef;
  recordKey: string;
  revision?: string;
}
export interface ReplyClassification {
  /** Undefined while the existing question layer is resolving associations. */
  questionThreads: ReadonlyMap<string, string | null> | undefined;
}
const threadKey = (thread: DiscussionRef) => JSON.stringify([thread.writer, thread.id]);
const askId = (ask: PageAsk) =>
  JSON.stringify(['reply', ask.writer, ask.operationId, ask.requestId]);
const commentId = (writer: string, id: string) => JSON.stringify(['comment', writer, id]);
const firstLine = (body: string) =>
  body
    .split(/\r?\n/u)
    .find((line) => line.trim())
    ?.trim() ?? text.askEmptyReply;

/** Consumes admitted projections only. Display names, author HTML and Send outcomes
 * cannot establish participation or an Ask/comment association. */
export function projectReplyNotifications(
  sources: ReplySources,
  deviceId: string | undefined,
  classification: ReplyClassification,
) {
  const candidates = new Map<string, ReplyNotification>();
  const deferred = new Set<string>();
  const threads = sources.threads;
  const classify = (thread: ThreadView) => {
    if (thread.proposal) return 'Proposal' as const;
    if (isChatThread(thread)) return 'Chat' as const;
    if (!thread.anchor) return;
    if (classification.questionThreads === undefined) return 'pending' as const;
    if (!classification.questionThreads.has(threadKey(thread.ref))) return 'Thread' as const;
    if (classification.questionThreads.get(threadKey(thread.ref))) return 'Answer' as const;
  };
  if (!sources.askUnavailable)
    for (const ask of sources.asks ?? []) {
      if (ask.reply === undefined || !ask.requestId || !isOwnDiscussionWriter(ask.writer, deviceId))
        continue;
      const id = askId(ask);
      const matches = threads?.filter(
        (thread) => thread.threadId === ask.thread && !thread.deleted,
      );
      if (ask.thread && threads === undefined) {
        deferred.add(id);
        continue;
      }
      const thread = matches?.length === 1 ? matches[0] : undefined;
      if (ask.thread && (!thread || !conversationAsks(thread, [ask]).length)) continue;
      const destination = thread ? classify(thread) : 'Chat';
      if (destination === 'pending') {
        deferred.add(id);
        continue;
      }
      if (!destination) continue;
      candidates.set(id, {
        id,
        destination,
        firstLine: firstLine(ask.reply),
        ...(destination === 'Answer'
          ? { questionId: classification.questionThreads!.get(threadKey(thread!.ref))! }
          : {}),
        ...(thread ? { thread: thread.ref } : {}),
        ...(destination === 'Thread' ? { quote: thread!.anchor!.exact } : {}),
        recordKey: conversationRecordKey('reply', ask.writer, ask.operationId),
      });
    }
  for (const thread of threads ?? []) {
    if (thread.deleted || !deviceId) continue;
    const participates =
      isOwnDiscussionWriter(thread.ref.writer, deviceId) ||
      thread.comments.some(
        (comment) => !comment.deleted && isOwnDiscussionWriter(comment.ref.writer, deviceId),
      );
    if (!participates) continue;
    const destination = classify(thread);
    for (const comment of thread.comments) {
      if (comment.deleted || isOwnDiscussionWriter(comment.ref.writer, deviceId)) continue;
      const id = commentId(comment.ref.writer, comment.messageId);
      if (destination === 'pending') {
        deferred.add(id);
        continue;
      }
      if (!destination) continue;
      candidates.set(id, {
        id,
        destination,
        firstLine: firstLine(comment.body),
        ...(destination === 'Answer'
          ? { questionId: classification.questionThreads!.get(threadKey(thread.ref))! }
          : {}),
        thread: thread.ref,
        ...(destination === 'Thread' ? { quote: thread.anchor!.exact } : {}),
        recordKey: conversationRecordKey('comment', comment.ref.writer, comment.messageId),
        revision: comment.revision,
      });
    }
  }
  return { candidates, deferred };
}

/** Separate baselines prevent an unavailable ledger or discussion projection from
 * turning its first successful read into a batch of historical notifications. */
export class ReplyNotificationObserver {
  private seen = new Set<string>();
  private asksReady = false;
  private threadsReady = false;
  constructor(initial: ReplySources) {
    this.baseline(initial);
  }

  /** A cached loader snapshot can precede the binding's first current view. */
  baselineFirstView(sources: ReplySources) {
    this.asksReady = false;
    this.threadsReady = false;
    this.baseline(sources);
  }

  private baseline(sources: ReplySources) {
    if (!this.asksReady && sources.asks !== undefined && !sources.askUnavailable) {
      for (const ask of sources.asks) if (ask.reply !== undefined) this.seen.add(askId(ask));
      this.asksReady = true;
    }
    if (!this.threadsReady && sources.threads !== undefined) {
      for (const thread of sources.threads)
        for (const comment of thread.comments)
          this.seen.add(commentId(comment.ref.writer, comment.messageId));
      this.threadsReady = true;
    }
  }

  observe(sources: ReplySources, projection: ReturnType<typeof projectReplyNotifications>) {
    this.baseline(sources);
    const arrivals: ReplyNotification[] = [];
    const consider = (id: string) => {
      if (this.seen.has(id) || projection.deferred.has(id)) return;
      this.seen.add(id);
      const candidate = projection.candidates.get(id);
      if (candidate) arrivals.push(candidate);
    };
    if (!sources.askUnavailable)
      for (const ask of sources.asks ?? []) if (ask.reply !== undefined) consider(askId(ask));
    for (const thread of sources.threads ?? [])
      for (const comment of thread.comments)
        consider(commentId(comment.ref.writer, comment.messageId));
    return arrivals;
  }
}
