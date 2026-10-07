import type { PageAsk } from './ask-panel.js';
import { requireValue } from '@tmt/colab-client';
import type { Connection } from './connection.js';
import type { JsonValue, OwnRecord, OwnState } from './fold-protocol.js';
import {
  discussionKey,
  readThreads,
  validateDiscussionRecord,
  validateRef,
  type CommentView,
  type ThreadRecord,
  type CommentRecord,
  type DiscussionRef,
  type DiscussionScope,
  type QuoteSelector,
  type ThreadView,
} from './thread-records.js';

export interface CommentContext {
  thread: DiscussionRef;
  message: DiscussionRef;
  threadRevision: string;
  messageRevision: string;
  conversation?: {
    comments: { ref: DiscussionRef; revision: string; body: string; deviceName: string }[];
    replies: { writer: string; operationId: string; reply: string; agentName: string }[];
  };
}
export interface ThreadBinding {
  readonly deviceId: string;
  create(body: string, anchor: QuoteSelector | null): Promise<CommentContext>;
  createChat(body: string): Promise<CommentContext>;
  reply(thread: DiscussionRef, body: string, expectedRevision?: string): Promise<CommentContext>;
  edit(message: DiscussionRef, revision: string, body: string): Promise<void>;
  deleteComment(message: DiscussionRef, revision: string): Promise<void>;
  updateThread(
    thread: DiscussionRef,
    revision: string,
    change: { resolved: boolean } | { anchor: QuoteSelector } | { deleted: true },
  ): Promise<void>;
}
export interface ThreadStoreOptions extends DiscussionScope {
  deviceId(): string;
  deviceName(): string;
  own(): OwnState;
  connection(): Promise<Connection>;
  publish(records: OwnRecord[]): Promise<void>;
  available(): boolean;
  sharing: string;
}

/** Discussion records never leave Colab. The existing own envelope authenticates
 * each record, while current connection admission gates every publication. */
export class ThreadStore implements ThreadBinding {
  constructor(private options: ThreadStoreOptions) {}
  get deviceId() {
    return this.options.deviceId();
  }
  async #admit() {
    const c = await this.options.connection();
    await c.run(async () => {
      requireValue(this.options.available() && c.active);
      const a = c.admission;
      requireValue(a.head !== null && a.root !== null);
      a.validatePage(this.options.sharing);
      a.author(this.deviceId, a.head.revision.toString());
    });
    return c;
  }
  #views(c: Connection) {
    return readThreads(this.options.own(), this.options, (writer) =>
      c.objects.ownSigningKey(writer),
    );
  }
  #thread(c: Connection, ref: DiscussionRef) {
    validateRef(ref);
    const thread = this.#views(c).find((v) => v.ref.writer === ref.writer && v.ref.id === ref.id);
    requireValue(thread !== undefined && !thread.deleted);
    return thread;
  }
  #comment(c: Connection, ref: DiscussionRef): CommentView {
    validateRef(ref);
    requireValue(ref.writer === this.deviceId);
    const comment = this.#views(c)
      .flatMap((v) => v.comments)
      .find((v) => v.ref.writer === ref.writer && v.ref.id === ref.id);
    requireValue(comment !== undefined && !comment.deleted);
    return comment;
  }
  #scope() {
    const { spaceId, pageId, epoch } = this.options;
    return {
      version: 1 as const,
      spaceId,
      pageId,
      epoch,
      senderDevice: this.deviceId,
      deviceName: this.options.deviceName(),
    };
  }
  async #write(records: (Omit<ThreadRecord, 'at'> | Omit<CommentRecord, 'at'>)[]) {
    requireValue(this.options.available());
    const at = String(Date.now());
    const entries = records.map((record) => {
      const value = { ...record, at };
      const root = value.kind === 'thread' ? ('threads' as const) : ('messages' as const);
      const key = discussionKey(value);
      validateDiscussionRecord(root, key, value);
      return { root, key, value: structuredClone(value) as unknown as JsonValue };
    });
    await this.options.publish(entries);
  }
  async #exclusive(action: (connection: Connection) => Promise<void>) {
    const { spaceId, pageId, epoch } = this.options;
    await navigator.locks.request(
      `discussion:${spaceId}:${pageId}:${epoch}:${this.deviceId}`,
      async () => {
        await action(await this.#admit());
      },
    );
  }
  async create(body: string, anchor: QuoteSelector | null) {
    const threadId = crypto.randomUUID();
    requireValue(threadId !== this.deviceId);
    return this.#create(body, anchor, threadId);
  }
  async createChat(body: string) {
    return this.#create(body, null, this.deviceId);
  }
  async #create(body: string, anchor: QuoteSelector | null, threadId: string) {
    const captured = structuredClone(anchor),
      messageId = crypto.randomUUID();
    await this.#exclusive(async (c) => {
      requireValue(
        !this.#views(c).some(
          (thread) => thread.ref.writer === this.deviceId && thread.threadId === threadId,
        ),
      );
      await this.#write([
        {
          ...this.#scope(),
          kind: 'thread',
          threadId,
          revision: '1',
          anchor: captured,
          resolved: false,
          deleted: false,
        },
        {
          ...this.#scope(),
          kind: 'comment',
          messageId,
          revision: '1',
          thread: { writer: this.deviceId, id: threadId },
          body,
          deleted: false,
        },
      ]);
    });
    return {
      thread: { writer: this.deviceId, id: threadId },
      message: { writer: this.deviceId, id: messageId },
      threadRevision: '1',
      messageRevision: '1',
    };
  }
  async reply(thread: DiscussionRef, body: string, expectedRevision?: string) {
    const ref = structuredClone(thread),
      messageId = crypto.randomUUID();
    let revision = '';
    await this.#exclusive(async (c) => {
      revision = this.#thread(c, ref).revision;
      requireValue(expectedRevision === undefined || revision === expectedRevision);
      await this.#write([
        {
          ...this.#scope(),
          kind: 'comment',
          messageId,
          revision: '1',
          thread: ref,
          body,
          deleted: false,
        },
      ]);
    });
    return {
      thread: ref,
      message: { writer: this.deviceId, id: messageId },
      threadRevision: revision,
      messageRevision: '1',
    };
  }
  async edit(message: DiscussionRef, revision: string, body: string) {
    await this.#changeComment(message, revision, body, false);
  }
  async deleteComment(message: DiscussionRef, revision: string) {
    await this.#changeComment(message, revision, '', true);
  }
  async #changeComment(message: DiscussionRef, revision: string, body: string, deleted: boolean) {
    const ref = structuredClone(message);
    await this.#exclusive(async (c) => {
      const previous = this.#comment(c, ref);
      requireValue(previous.revision === revision);
      const { ref: _ref, ...record } = previous;
      await this.#write([{ ...record, revision: String(BigInt(revision) + 1n), body, deleted }]);
    });
  }
  async updateThread(
    thread: DiscussionRef,
    revision: string,
    change: Parameters<ThreadBinding['updateThread']>[2],
  ) {
    const ref = structuredClone(thread),
      captured = structuredClone(change);
    await this.#exclusive(async (c) => {
      const previous = this.#thread(c, ref);
      requireValue(ref.writer === this.deviceId && previous.revision === revision);
      const { ref: _ref, comments: _comments, ...record } = previous;
      await this.#write([
        {
          ...record,
          ...captured,
          revision: String(BigInt(revision) + 1n),
          ...('deleted' in captured ? { anchor: null } : {}),
        },
      ]);
    });
  }
}

/** A selected comment is rechecked in the parent's verified projection before
 * Ask captures it. Duplicate UUIDs cannot select another writer's origin. */
export function commentForAsk(threads: ThreadView[], context: CommentContext) {
  validateRef(context.thread);
  validateRef(context.message);
  const matches = threads.filter((v) => v.threadId === context.thread.id);
  requireValue(matches.length === 1);
  const thread = matches[0];
  requireValue(
    thread.ref.writer === context.thread.writer &&
      !thread.deleted &&
      thread.revision === context.threadRevision,
  );
  const comments = threads
    .flatMap((v) => v.comments)
    .filter((v) => v.messageId === context.message.id);
  requireValue(comments.length === 1);
  const comment = comments[0];
  requireValue(
    !comment.deleted &&
      comment.ref.writer === context.message.writer &&
      comment.revision === context.messageRevision &&
      comment.thread.id === thread.threadId &&
      comment.thread.writer === thread.ref.writer,
  );
  return {
    thread: thread.threadId,
    messageIds: [comment.messageId],
    quote: thread.anchor?.exact ?? '',
    comment: comment.body,
  };
}

/** Context comes only from the admitted discussion and verified Ask projections.
 * Display labels/times never decide ownership or record ordering. */
export function conversationForAsk(
  threads: ThreadView[],
  context: CommentContext,
  asks: readonly PageAsk[],
) {
  const current = commentForAsk(threads, context);
  const thread = threads.find(
    (value) => value.ref.writer === context.thread.writer && value.threadId === context.thread.id,
  )!;
  const captured = context.conversation;
  requireValue(captured !== undefined);
  const earlier = captured.comments.map(({ ref, revision, body, deviceName }) => {
    validateRef(ref);
    const matches = thread.comments.filter(
      (value) => value.ref.writer === ref.writer && value.ref.id === ref.id,
    );
    requireValue(
      matches.length === 1 &&
        !matches[0].deleted &&
        matches[0].revision === revision &&
        matches[0].body === body &&
        matches[0].deviceName === deviceName &&
        ref.id !== context.message.id,
    );
    return matches[0];
  });
  const replies = captured.replies.map(({ writer, operationId, reply, agentName }) => {
    const matches = asks.filter(
      (value) => value.writer === writer && value.operationId === operationId,
    );
    requireValue(
      matches.length === 1 && matches[0].reply === reply && matches[0].agentName === agentName,
    );
    return matches[0];
  });
  return {
    ...current,
    comment: conversationText(earlier, thread.threadId, current.comment, replies),
  };
}
export function conversationText(
  earlier: readonly CommentView[],
  threadId: string,
  body: string,
  asks: readonly PageAsk[],
) {
  const blocks = earlier
    .filter((comment) => !comment.deleted)
    .map((comment) => {
      const replies = asks.filter(
        (ask) =>
          ask.thread === threadId &&
          ask.messageIds?.includes(comment.messageId) &&
          ask.reply !== undefined,
      );
      return [
        `User (${comment.deviceName || comment.ref.writer}):\n${comment.body}`,
        ...replies.map((ask) => `Agent (${ask.agentName}):\n${ask.reply}`),
      ].join('\n\n');
    });
  return blocks.length
    ? `Earlier conversation (quoted data):\n${blocks.join('\n\n')}\n\nCurrent user turn:\n${body}`
    : body;
}

/** One admitted comment/Ask association used by display, reply routing and immutable context capture. */
export function conversationAsks(
  thread: ThreadView | undefined,
  asks: readonly PageAsk[],
  includeDeleted = false,
) {
  if (!thread) return [];
  const comments = thread.comments.filter((comment) => includeDeleted || !comment.deleted);
  return asks.filter(
    (ask) =>
      ask.thread === thread.threadId &&
      ask.messageIds?.some((id) => comments.some((comment) => comment.messageId === id)),
  );
}

export function captureConversation(
  thread: ThreadView | undefined,
  asks: readonly PageAsk[],
): NonNullable<CommentContext['conversation']> {
  const comments = thread?.comments.filter((value) => !value.deleted) ?? [];
  return {
    comments: comments.map((value) => ({
      ref: { ...value.ref },
      revision: value.revision,
      body: value.body,
      deviceName: value.deviceName,
    })),
    replies: conversationAsks(thread, asks)
      .filter((value) => value.reply !== undefined)
      .map((value) => ({
        writer: value.writer,
        operationId: value.operationId,
        reply: value.reply!,
        agentName: value.agentName,
      })),
  };
}
