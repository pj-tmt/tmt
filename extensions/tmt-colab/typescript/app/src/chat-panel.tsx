import { BrowserIconAction } from '@tmt/browser-ui/react';
import { X } from 'lucide-react';
import { ConversationWindow } from './components/conversation-window.js';
import { text } from './strings.js';
import { useEffect, useRef } from 'react';
import { AnnotationInput } from './annotation-input.js';
import { AskPanel, type AskBinding, type PageAsk } from './ask-panel.js';
import { conversationAsks } from './thread-store.js';
import { CommentExchange } from './thread-panel.js';
import { isChatThread, type ThreadView } from './thread-records.js';
import type { ThreadBinding } from './thread-store.js';

/** All chat history is page-visible; only this device's designated thread receives its next turn. */
export function ChatPanel({
  threads,
  asks,
  binding,
  discussion,
  title,
  blocked,
  recoveryRequired = false,
  observationUnavailable,
  close,
}: {
  threads: readonly ThreadView[];
  asks: readonly PageAsk[];
  binding?: AskBinding;
  discussion?: ThreadBinding;
  title: string;
  blocked: boolean;
  recoveryRequired?: boolean;
  observationUnavailable?: boolean;
  close(): void;
}) {
  const history = useRef<HTMLDivElement>(null);
  const chats = threads.filter(isChatThread);
  const own = chats.find((thread) => thread.ref.writer === discussion?.deviceId);
  const legacy = asks.filter((ask) => !ask.thread);
  const latest = conversationAsks(own, asks)
    .filter((ask) => ask.reply !== undefined)
    .reduce<PageAsk | undefined>(
      (latest, ask) => (!latest || ask.issuedAt > latest.issuedAt ? ask : latest),
      undefined,
    );
  const replier = latest ? { machine: latest.machine, agent: latest.agent } : undefined;
  useEffect(() => {
    const node = history.current;
    if (node) node.scrollTop = node.scrollHeight;
  }, [threads, asks]);
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
      historyRef={history}
      historyClassName="chat-messages"
      notice={
        observationUnavailable && (
          <p role="status" className="annotation-hint">
            {text.askObservationUnavailable}
          </p>
        )
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
          replier={replier}
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
