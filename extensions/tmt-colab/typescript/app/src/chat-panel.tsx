import { ConversationPresentation } from './components/conversation-presentation.js';
import { AttachmentDropPanel } from './attachment-drop.js';
import { BrowserIconAction } from '@tmt/browser-ui/react';
import { ChevronUp } from 'lucide-react';
import { Fragment } from 'react';
import { ConversationWindow, conversationRecordKey } from './components/conversation-window.js';
import { text } from './strings.js';
import { AnnotationInput } from './annotation-input.js';
import { AskPanel, type AskBinding, type PageAsk } from './ask-panel.js';
import type { ComposerActivation, ComposerEdit } from './components/message-composer-edit.js';
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
  archived = false,
  observationUnavailable,
  initialEdit,
  activation,
  onDraft,
  onBusy,
  unsaved = false,
  close,
  escape = close,
}: {
  threads: readonly ThreadView[];
  asks: readonly PageAsk[];
  binding?: AskBinding;
  discussion?: ThreadBinding;
  title: string;
  creationRecipient?: CreationRecipient;
  blocked: boolean;
  recoveryRequired?: boolean;
  archived?: boolean;
  observationUnavailable?: boolean;
  /** A draft kept for this composer; read once when it mounts. */
  initialEdit?: ComposerEdit;
  activation?: ComposerActivation;
  onDraft?(edit: ComposerEdit): void;
  onBusy?(busy: boolean): void;
  /** Device storage failed, so the draft lives in this tab only. */
  unsaved?: boolean;
  close(): void;
  escape?(): void;
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
  let previousDay: string | undefined;
  const dayHeading = (at: string | number) => {
    const date = new Date(Number(at));
    const day = date.toDateString();
    if (day === previousDay) return null;
    previousDay = day;
    return (
      <p className="chat-day">
        <time dateTime={date.toISOString()}>
          {day === new Date().toDateString()
            ? text.chatToday
            : date.toLocaleDateString(undefined, {
                month: 'short',
                day: 'numeric',
                year: 'numeric',
              })}
        </time>
      </p>
    );
  };
  const history = chats.map((thread) => (
    <section
      key={`${thread.ref.writer}:${thread.threadId}`}
      data-testid="chat-thread"
      data-thread-id={thread.threadId}
      data-writer={thread.ref.writer}
    >
      {thread.comments.map((comment) => (
        <Fragment key={`${comment.ref.writer}:${comment.messageId}`}>
          {dayHeading(comment.at)}
          <CommentExchange
            readOnly={archived}
            key={`${comment.ref.writer}:${comment.messageId}`}
            comment={comment}
            thread={thread}
            binding={discussion}
            ask={binding}
            asks={asks}
            blocked={blocked || archived}
            allowEdit={false}
            title={title}
          />
        </Fragment>
      ))}
    </section>
  ));
  const legacyDays = legacy.map((record) => dayHeading(record.issuedAt));
  return (
    <ConversationPresentation value="chat">
      <AttachmentDropPanel target="chat">
        <ConversationWindow
          className="chat-panel"
          data-testid="chat-panel"
          aria-label="Page chat"
          title="Chat"
          caption={text.chatVisible}
          actions={
            <BrowserIconAction
              type="button"
              label={text.chatCollapse}
              tooltipLabel={text.chatCollapseHint}
              ariaKeyShortcuts="Escape"
              variant="text"
              icon={<ChevronUp />}
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
              archived={archived}
              onBusy={onBusy}
              binding={binding}
              discussion={discussion}
              anchor={null}
              thread={own}
              asks={asks}
              title={title}
              creationRecipient={creationRecipient}
              initialEdit={initialEdit}
              activation={activation}
              onDraft={(_value, edit) => onDraft?.(edit)}
              blocked={blocked || !!own?.deleted}
              recoveryRequired={recoveryRequired && !own?.deleted}
              cancel={escape}
              committed={() => {}}
            />
          }
        >
          {!chats.length && !legacy.length && (
            <div className="chat-empty">
              <span aria-hidden="true">○</span>
              <h3>{text.chatEmpty}</h3>
              {!archived && !blocked && !!discussion && !recoveryRequired && <p>{text.chatEmptyHint}</p>}
            </div>
          )}
          {history}
          {legacy.length > 0 && (
            <AskPanel
              readOnly={archived}
              dayHeadings={legacyDays}
              records={legacy}
              binding={binding}
              blocked={blocked}
              inline
            />
          )}
        </ConversationWindow>
      </AttachmentDropPanel>
    </ConversationPresentation>
  );
}
