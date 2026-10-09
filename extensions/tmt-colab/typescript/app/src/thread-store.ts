import type { PageAsk } from './ask-panel.js';
import { requireValue } from '@tmt/colab-client';
import {
  AttachmentService,
  type AttachmentBinding,
  type StoredAttachment,
} from './attachment-service.js';
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
import {
  discussionRefKey,
  type ThreadRecipient,
  type ThreadStatusRecord,
  type ThreadStatusView,
  type ThreadNotificationRecord,
} from './thread-status.js';

/** Committed originals for one message. The caller allocates `messageId` before sealing
 * because every descriptor binds it; the message is published only with their records. */
export interface MessageAttachments {
  messageId: string;
  stored: readonly StoredAttachment[];
}

export type StatusChange = { changed: false } | { changed: true; status: ThreadStatusView };

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
  /** Absent where this binding cannot upload or open attachments. */
  readonly attachments?: AttachmentBinding;
  create(
    body: string,
    anchor: QuoteSelector | null,
    attach?: MessageAttachments,
  ): Promise<CommentContext>;
  createChat(body: string, attach?: MessageAttachments): Promise<CommentContext>;
  reply(
    thread: DiscussionRef,
    body: string,
    expectedRevision?: string,
    attach?: MessageAttachments,
  ): Promise<CommentContext>;
  edit(message: DiscussionRef, revision: string, body: string): Promise<void>;
  deleteComment(message: DiscussionRef, revision: string): Promise<void>;
  setStatus(
    thread: DiscussionRef,
    previous: DiscussionRef | null,
    resolved: boolean,
    recipients?: readonly ThreadRecipient[],
  ): Promise<StatusChange>;
  notificationFailed(
    status: ThreadStatusView,
    operationId: string,
    reason: ThreadNotificationRecord['reason'],
  ): Promise<void>;
  updateThread(
    thread: DiscussionRef,
    revision: string,
    change: { anchor: QuoteSelector } | { deleted: true },
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
  scope?(): string;
}

/** Discussion records never leave Colab. The existing own envelope authenticates
 * each record, while current connection admission gates every publication. */
export class ThreadStore implements ThreadBinding {
  readonly attachments: AttachmentService;
  constructor(private options: ThreadStoreOptions) {
    this.attachments = new AttachmentService(options);
  }
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
    return readThreads(
      this.options.own(),
      this.options,
      (writer) => c.objects.ownSigningKey(writer),
      (writer) => c.objects.statusWriter(writer),
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
  async #write(
    records: (
      | Omit<ThreadRecord, 'at'>
      | Omit<CommentRecord, 'at'>
      | Omit<ThreadStatusRecord, 'at'>
      | Omit<ThreadNotificationRecord, 'at'>
    )[],
    prefix: readonly OwnRecord[] = [],
  ) {
    requireValue(this.options.available());
    const at = String(Date.now());
    const values = records.map((record) => ({ ...record, at }));
    const entries = values.map((value) => {
      const root = value.kind === 'thread' ? ('threads' as const) : ('messages' as const);
      const key = discussionKey(value);
      validateDiscussionRecord(root, key, value);
      return { root, key, value: structuredClone(value) as unknown as JsonValue };
    });
    await this.options.publish([...prefix, ...entries]);
    return values;
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
  async create(body: string, anchor: QuoteSelector | null, attach?: MessageAttachments) {
    const threadId = crypto.randomUUID();
    requireValue(threadId !== this.deviceId);
    return this.#create(body, anchor, threadId, attach);
  }
  async createChat(body: string, attach?: MessageAttachments) {
    return this.#create(body, null, this.deviceId, attach);
  }
  /** The message record's attachment list and the verified publication records for it.
   * Preparation runs under the discussion lock, so the batch is checked against the
   * same admission it is written with. */
  async #attached(attach: MessageAttachments | undefined) {
    if (!attach?.stored.length) return { records: [] as OwnRecord[], list: {} };
    const descriptors = attach.stored.map(({ original }) => original.descriptor);
    requireValue(
      descriptors.every(
        (d) =>
          d.source.kind === 'message' &&
          d.source.messageId === attach.messageId &&
          d.source.writerId === this.deviceId &&
          d.source.messageRevision === '1',
      ),
    );
    return {
      records: await this.attachments.publication(attach.stored),
      list: { attachments: descriptors },
    };
  }
  async #create(
    body: string,
    anchor: QuoteSelector | null,
    threadId: string,
    attach?: MessageAttachments,
  ) {
    const captured = structuredClone(anchor),
      messageId = attach?.messageId ?? crypto.randomUUID();
    await this.#exclusive(async (c) => {
      const attached = await this.#attached(attach);
      requireValue(
        !this.#views(c).some(
          (thread) => thread.ref.writer === this.deviceId && thread.threadId === threadId,
        ),
      );
      await this.#write(
        [
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
            ...attached.list,
            deleted: false,
          },
        ],
        attached.records,
      );
    });
    return {
      thread: { writer: this.deviceId, id: threadId },
      message: { writer: this.deviceId, id: messageId },
      threadRevision: '1',
      messageRevision: '1',
    };
  }
  async reply(
    thread: DiscussionRef,
    body: string,
    expectedRevision?: string,
    attach?: MessageAttachments,
  ) {
    const ref = structuredClone(thread),
      messageId = attach?.messageId ?? crypto.randomUUID();
    let revision = '';
    await this.#exclusive(async (c) => {
      revision = this.#thread(c, ref).revision;
      requireValue(expectedRevision === undefined || revision === expectedRevision);
      const attached = await this.#attached(attach);
      await this.#write(
        [
          {
            ...this.#scope(),
            kind: 'comment',
            messageId,
            revision: '1',
            thread: ref,
            body,
            ...attached.list,
            deleted: false,
          },
        ],
        attached.records,
      );
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
      const { ref: _ref, attachments, ...record } = previous;
      // A descriptor binds its message revision, so an edit never carries one forward;
      // deletion drops the references and the bytes stay unreachable.
      requireValue(deleted || !attachments?.length);
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
      requireValue(previous.revision === revision);
      // Anchor/deletion stay restricted to the originating stream and never copy
      // folded metadata. Status has its own action: setStatus.
      requireValue(ref.writer === this.deviceId);
      const record: unknown = this.options.own()[this.deviceId]?.threads[`${ref.id}:${revision}`];
      validateDiscussionRecord('threads', `${ref.id}:${revision}`, record);
      requireValue(record.kind === 'thread');
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
  async #status(
    previous: ThreadView,
    resolved: boolean,
    recipients: readonly ThreadRecipient[],
  ): Promise<StatusChange> {
    requireValue(!(previous.threadId === previous.ref.writer && previous.anchor === null));
    if (previous.resolved === resolved) return { changed: false };
    const actionId = crypto.randomUUID();
    const record = {
      ...this.#scope(),
      kind: 'thread-status' as const,
      actionId,
      thread: { ...previous.ref },
      previous: previous.status?.ref ?? null,
      revision: '1',
      deleted: false,
      resolved,
      actor: 'person' as const,
      agentName: null,
      recipients: structuredClone([...recipients]),
    };
    const [value] = await this.#write([record]);
    validateDiscussionRecord('messages', `${actionId}:thread-status`, value);
    requireValue(value.kind === 'thread-status');
    return {
      changed: true,
      status: {
        ...value,
        ref: { writer: this.deviceId, id: actionId },
        depth: (previous.status?.depth ?? 0) + 1,
      },
    };
  }
  async setStatus(
    thread: DiscussionRef,
    previous: DiscussionRef | null,
    resolved: boolean,
    recipients: readonly ThreadRecipient[] = [],
  ): Promise<StatusChange> {
    const captured = structuredClone({ thread, previous, recipients });
    let result: StatusChange = { changed: false };
    await this.#exclusive(async (c) => {
      const view = this.#thread(c, captured.thread);
      const current = view.status?.ref ?? null;
      requireValue(
        current === null
          ? captured.previous === null
          : captured.previous !== null &&
              discussionRefKey(current) === discussionRefKey(captured.previous),
      );
      result = await this.#status(view, resolved, captured.recipients);
    });
    return result;
  }
  async notificationFailed(
    status: ThreadStatusView,
    operationId: string,
    reason: ThreadNotificationRecord['reason'],
  ) {
    const captured = structuredClone(status);
    await this.#exclusive(async () => {
      requireValue(
        captured.senderDevice === this.deviceId &&
          captured.ref.writer === this.deviceId &&
          captured.ref.id === captured.actionId &&
          captured.recipients.some((recipient) => recipient.operationId === operationId),
      );
      const stored =
        this.options.own()[this.deviceId]?.messages[`${captured.actionId}:thread-status`];
      requireValue(stored !== undefined);
      validateDiscussionRecord('messages', `${captured.actionId}:thread-status`, stored);
      requireValue(
        stored.kind === 'thread-status' &&
          stored.senderDevice === this.deviceId &&
          stored.spaceId === this.options.spaceId &&
          stored.pageId === this.options.pageId &&
          stored.epoch === this.options.epoch &&
          stored.recipients.some((recipient) => recipient.operationId === operationId),
      );
      const existing: unknown =
        this.options.own()[this.deviceId]?.messages[`${operationId}:thread-notification`];
      if (existing !== undefined) {
        validateDiscussionRecord('messages', `${operationId}:thread-notification`, existing);
        requireValue(
          existing.kind === 'thread-notification' &&
            discussionRefKey(existing.status) === discussionRefKey(captured.ref) &&
            existing.reason === reason,
        );
        return;
      }
      await this.#write([
        {
          ...this.#scope(),
          kind: 'thread-notification',
          operationId,
          status: { ...captured.ref },
          reason,
          revision: '1',
          deleted: false,
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
