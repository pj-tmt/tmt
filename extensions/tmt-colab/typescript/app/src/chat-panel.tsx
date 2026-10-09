import { BrowserIconAction } from '@tmt/browser-ui/react';
import { X } from 'lucide-react';
import { ConversationWindow, conversationRecordKey } from './components/conversation-window.js';
import { text } from './strings.js';
import { AnnotationInput } from './annotation-input.js';
import { AskPanel, type AskBinding, type PageAsk } from './ask-panel.js';
import type { ComposerEdit } from './components/message-composer-edit.js';
import type { CreationRecipient } from './fold-protocol.js';
import { CommentExchange } from './thread-panel.js';
import { isChatThread, type ThreadView } from './thread-records.js';
import { conversationAsks, type ThreadBinding } from './thread-store.js';

/** All chat history is page-visible; only this device's designated thread receives its next turn. */
export function ChatPanel({
  threads,
  asks,
  binding,
  discussion,
  title,
  creationRecipient,
  blocked,
  recoveryRequired = false,
  observationUnavailable,
  initialEdit,
  onDraft,
  unsaved = false,
  close,
}: {
  threads: readonly ThreadView[];
  asks: readonly PageAsk[];
  binding?: AskBinding;
  discussion?: ThreadBinding;
  title: string;
  creationRecipient?: CreationRecipient;
  blocked: boolean;
  recoveryRequired?: boolean;
  observationUnavailable?: boolean;
  /** A draft kept for this composer; read once when it mounts. */
  initialEdit?: ComposerEdit;
  onDraft?(edit: ComposerEdit): void;
  /** Device storage failed, so the draft lives in this tab only. */
  unsaved?: boolean;
  close(): void;
}) {
  const chats = threads.filter(isChatThread);
  const own = chats.find((thread) => thread.ref.writer === discussion?.deviceId);
  const legacy = asks.filter((ask) => !ask.thread);
  const messageIds = [
    ...chats.flatMap((thread) => [
      ...thread.comments.map((comment) =>
        conversationRecordKey('comment', comment.ref.writer, comment.messageId),
      ),
      ...conversationAsks(thread, asks, true)
        .filter((record) => record.reply !== undefined)
        .map((record) => conversationRecordKey('reply', record.writer, record.operationId)),
    ]),
    ...legacy.flatMap((record) => [
      conversationRecordKey('ask', record.writer, record.operationId),
      ...(record.reply !== undefined
        ? [conversationRecordKey('reply', record.writer, record.operationId)]
        : []),
    ]),
  ];
  return (
    <ConversationWindow
      className="chat-panel"
      data-testid="chat-panel"
      aria-label="Page chat"
      title="Chat"
      caption={text.chatVisible}
      actions={
        <BrowserIconAction
          type="button"
          label="Close Chat"
          variant="text"
          icon={<X />}
          onActivate={(event) => {
            if (event.isTrusted) close();
          }}
        />
      }
      messageIds={messageIds}
      ownWriter={discussion?.deviceId}
      historyClassName="chat-messages"
      notice={
        <>
          {observationUnavailable && (
            <p role="status" className="annotation-hint">
              {text.askObservationUnavailable}
            </p>
          )}
          {unsaved && (
            <p role="status" className="annotation-hint">
              {text.draftsNotSaved}
            </p>
          )}
        </>
      }
      composer={
        <AnnotationInput
          chat
          binding={binding}
          discussion={discussion}
          anchor={null}
          thread={own}
          asks={asks}
          title={title}
          creationRecipient={creationRecipient}
          initialEdit={initialEdit}
          onDraft={(_value, edit) => onDraft?.(edit)}
          blocked={blocked || !!own?.deleted}
          recoveryRequired={recoveryRequired && !own?.deleted}
          cancel={close}
          committed={() => {}}
        />
      }
    >
      {!chats.length && !legacy.length && (
        <p className="isolation-note">Talk with an agent about this page.</p>
      )}
      {chats.map((thread) => (
        <section
          key={`${thread.ref.writer}:${thread.threadId}`}
          data-testid="chat-thread"
          data-thread-id={thread.threadId}
          data-writer={thread.ref.writer}
        >
          {thread.comments.map((comment) => (
            <CommentExchange
              key={`${comment.ref.writer}:${comment.messageId}`}
              comment={comment}
              thread={thread}
              binding={discussion}
              ask={binding}
              asks={asks}
              blocked={blocked}
              allowEdit={false}
            />
          ))}
        </section>
      ))}
      {legacy.length > 0 && (
        <AskPanel records={legacy} binding={binding} blocked={blocked} inline />
      )}
    </ConversationWindow>
  );
}
