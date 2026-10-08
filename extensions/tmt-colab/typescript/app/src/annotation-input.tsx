import { BrowserAction } from '@tmt/browser-ui/react';
import { useEffect, useRef, useState } from 'react';
import { MessageComposer } from './components/message-composer.js';
import type { ComposerEdit, RecipientKey } from './components/message-composer-edit.js';
import { messageRecipient } from './message-recipient.js';
import { text } from './strings.js';
import type { AskBinding, PageAsk } from './ask-panel.js';
import type { AgentDestination } from './live-ask.js';
import type { ThreadBinding } from './thread-store.js';
import { captureConversation, conversationAsks } from './thread-store.js';
import type { DiscussionRef, QuoteSelector, ThreadView } from './thread-records.js';

/** One trusted input. Enter is the explicit effect; disclosure never prepares or sends an intent. */
export function AnnotationInput({
  binding,
  discussion,
  anchor,
  thread,
  asks,
  title,
  replier,
  initialValue,
  initialEdit,
  onDraft,
  onBusy,
  blocked,
  cancel,
  committed,
  chat = false,
}: {
  binding?: AskBinding;
  discussion?: ThreadBinding;
  anchor: QuoteSelector | null;
  thread?: ThreadView;
  asks: readonly PageAsk[];
  title: string;
  /** Chat only: the agent that answered last. */
  replier?: RecipientKey;
  /** A draft kept from an earlier close of the same selection. */
  initialValue?: string;
  initialEdit?: ComposerEdit;
  /** Reports every value, so the owner can keep an unsent draft. */
  onDraft?(value: string, edit: ComposerEdit): void;
  /** Reports a send in flight, which nothing outside may interrupt. */
  onBusy?(busy: boolean): void;
  blocked: boolean;
  cancel(): void;
  committed(ref: DiscussionRef): void;
  chat?: boolean;
}) {
  const [agents, setAgents] = useState<AgentDestination[]>();
  const [edit, setEdit] = useState<ComposerEdit>(initialEdit ?? { value: initialValue ?? '' });
  const value = edit.value;
  const [resetKey, setResetKey] = useState(0);
  const [busy, setBusy] = useState(false);
  const sending = useRef(false);
  const [error, setError] = useState<string>();
  const [discoveryFailed, setDiscoveryFailed] = useState(false);
  const [recorded, setRecorded] = useState<DiscussionRef>();
  useEffect(() => {
    let active = true;
    if (!binding) return;
    void binding.destinations().then(
      (destinations) => {
        if (!active) return;
        setAgents(destinations);
        setDiscoveryFailed(false);
      },
      () => {
        if (active) {
          setAgents(undefined);
          setDiscoveryFailed(true);
        }
      },
    );
    return () => {
      active = false;
    };
  }, [binding]);
  useEffect(() => {
    onDraft?.(value, edit);
  }, [value, edit]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => onBusy?.(busy), [busy]); // eslint-disable-line react-hooks/exhaustive-deps
  const prior = conversationAsks(thread, asks)
    .filter((ask) => ask.reply !== undefined)
    .map((ask) => ({ machine: ask.machine, agent: ask.agent }));
  const decide = (intent: 'comment' | 'agent') =>
    messageRecipient({
      writable: !blocked && !!discussion,
      intent,
      destinations: agents,
      selected: edit.recipient,
      replyRecipients: chat ? [] : prior,
      lastReplier: replier,
    });
  const decision = decide('agent');
  const destination = decision.kind === 'agent' ? decision.destination : undefined;
  const quote = thread?.anchor?.exact ?? anchor?.exact ?? '';
  const defaultIntent = chat || prior.length || edit.recipient ? 'agent' : 'comment';
  function clearDraft() {
    setEdit((previous) => ({ value: '', recipient: previous.recipient }));
    setResetKey((key) => key + 1);
  }
  async function send(intent: 'comment' | 'agent') {
    if (sending.current || recorded) return;
    const admitted = decide(intent);
    if (admitted.kind === 'blocked' || !discussion || (intent === 'agent' && !binding)) {
      setError(text.messageUnavailable);
      return;
    }
    if (intent === 'agent' && admitted.kind !== 'agent') {
      setError(
        decision.kind === 'unavailable'
          ? text.messageAgentsUnavailable
          : text.messageChooseRecipient,
      );
      return;
    }
    if (!value.trim()) {
      setError(text.messageWrite);
      return;
    }
    const captured = {
      value,
      destination: admitted.kind === 'agent' ? structuredClone(admitted.destination) : undefined,
      anchor: structuredClone(anchor),
      thread: thread?.ref,
      threadRevision: thread?.revision,
      title,
      conversation: captureConversation(thread, asks),
      url: location.href,
    };
    sending.current = true;
    setBusy(true);
    setError(undefined);
    let origin: Awaited<ReturnType<ThreadBinding['create']>> | undefined;
    try {
      origin = captured.thread
        ? await discussion.reply(captured.thread, captured.value, captured.threadRevision)
        : chat
          ? await discussion.createChat(captured.value)
          : await discussion.create(captured.value, captured.anchor);
      if (captured.destination && binding) {
        const attempt = await binding.prepare({
          quote,
          comment: captured.value,
          title: captured.title,
          url: captured.url,
          destination: captured.destination,
          context: { ...origin, conversation: captured.conversation },
        });
        const outcome = await attempt.send();
        if (!['accepted', 'held', 'uncertain'].includes(outcome.state))
          setError(text.messageRecordedDeliveryFailed);
      }
      clearDraft();
      committed(origin.thread);
    } catch {
      setError(origin ? text.messageRecordedUncertain : text.messageRecordFailed);
      if (origin) {
        clearDraft();
        setRecorded(origin.thread);
      }
    } finally {
      sending.current = false;
      setBusy(false);
    }
  }
  return (
    <section className="annotation-compose" data-testid="annotation-compose">
      <MessageComposer
        edit={edit}
        onChange={(next) => {
          setEdit(next);
          setError(undefined);
        }}
        label={text.messageLabel}
        placeholder={text.messagePlaceholder}
        candidates={agents}
        recipientPickerLabel={
          destination ? text.messageChangeRecipient : text.messageSelectRecipient
        }
        resetKey={resetKey}
        autoFocus
        disabled={busy || blocked || !!recorded || !discussion}
        onSubmit={(event) => {
          if (event.isTrusted && !event.isComposing && event.keyCode !== 229)
            void send(defaultIntent);
        }}
        onCancel={(event) => {
          if (event.isTrusted && !event.isComposing && event.keyCode !== 229 && !sending.current)
            cancel();
        }}
      />
      <p className="annotation-hint">{busy ? text.messageSending : text.messageKeys}</p>
      {destination && (
        <p className="annotation-hint">
          {text.messageRecipient}: {destination.agentName} · {destination.machineName}
        </p>
      )}
      {discoveryFailed && (
        <p role="status">
          {text.messageAgentsUnavailable}
          {!chat && <> {text.messageCommentAvailable}</>}
        </p>
      )}
      <div className="comment-actions">
        {!chat && (
          <BrowserAction
            type="button"
            label={thread ? text.commentPostReply : text.commentPost}
            variant={defaultIntent === 'comment' ? 'primary' : 'text'}
            busy={busy}
            disabled={blocked || !!recorded || !discussion || !value.trim()}
            onActivate={(event) => {
              if (event.isTrusted) void send('comment');
            }}
          />
        )}
        <BrowserAction
          type="button"
          label={chat ? text.askSend : text.ask}
          variant={defaultIntent === 'agent' ? 'primary' : 'text'}
          busy={busy}
          disabled={blocked || !!recorded || !discussion || !binding || !value.trim()}
          onActivate={(event) => {
            if (event.isTrusted) void send('agent');
          }}
        />
      </div>
      {error && <p role="alert">{error}</p>}
      {recorded && (chat || !thread) && (
        <BrowserAction
          type="button"
          label={chat ? text.messageAnother : text.messageOpenRecorded}
          variant="text"
          onActivate={(event) => {
            if (event.isTrusted) {
              if (chat) {
                setRecorded(undefined);
                setError(undefined);
                clearDraft();
              } else committed(recorded);
            }
          }}
        />
      )}
    </section>
  );
}
