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
}
export interface ThreadBinding {
  readonly deviceId: string;
  create(body: string, anchor: QuoteSelector | null): Promise<void>;
  reply(thread: DiscussionRef, body: string): Promise<void>;
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
    const captured = structuredClone(anchor),
      threadId = crypto.randomUUID(),
      messageId = crypto.randomUUID();
    await this.#exclusive(async () => {
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
  }
  async reply(thread: DiscussionRef, body: string) {
    const ref = structuredClone(thread),
      messageId = crypto.randomUUID();
    await this.#exclusive(async (c) => {
      this.#thread(c, ref);
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
