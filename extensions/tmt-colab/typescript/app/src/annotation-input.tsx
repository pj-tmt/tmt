import { BrowserAction } from '@tmt/browser-ui/react';
import {
  useEffect,
  useId,
  useRef,
  useState,
  useSyncExternalStore,
  type ClipboardEvent,
  type DragEvent,
} from 'react';
import { AttachmentDraft } from './attachment-draft.js';
import { AttachmentStaleError } from './attachment-service.js';
import { AttachButton, AttachmentChips } from './attachment-tray.js';
import { useAgentDirectory } from './agent-directory.js';
import { MessageComposer } from './components/message-composer.js';
import type { ComposerEdit } from './components/message-composer-edit.js';
import type { CreationRecipient } from './fold-protocol.js';
import { messageRecipient } from './message-recipient.js';
import { text } from './strings.js';
import { AskAgainAction, type AskAgainInput } from './ask-again.js';
import type { AskBinding, PageAsk } from './ask-panel.js';
import type { ThreadBinding } from './thread-store.js';
import { captureConversation } from './thread-store.js';
import type { DiscussionRef, QuoteSelector, ThreadView } from './thread-records.js';

/** One trusted input. Enter is the explicit effect; disclosure never prepares or sends an intent. */
export function AnnotationInput({
  binding,
  discussion,
  anchor,
  thread,
  asks,
  title,
  creationRecipient,
  initialValue,
  initialEdit,
  onDraft,
  onBusy,
  blocked,
  recoveryRequired = false,
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
  /** Stable creation UUIDs from the admitted page projection, never a publisher label. */
  creationRecipient?: CreationRecipient;
  /** A draft kept from an earlier close of the same selection. */
  initialValue?: string;
  initialEdit?: ComposerEdit;
  /** Reports every value, so the owner can keep an unsent draft. */
  onDraft?(value: string, edit: ComposerEdit): void;
  /** Reports a send in flight, which nothing outside may interrupt. */
  onBusy?(busy: boolean): void;
  blocked: boolean;
  /** Recovery keeps local editing available; blocked still fences every publish. */
  recoveryRequired?: boolean;
  cancel(): void;
  committed(ref: DiscussionRef): void;
  chat?: boolean;
}) {
  const [edit, setEdit] = useState<ComposerEdit>(initialEdit ?? { value: initialValue ?? '' });
  const value = edit.value;
  const statusId = useId();
  const initialized = useRef(initialEdit !== undefined || !!initialValue);
  const [failures, setFailures] = useState<
    {
      agent: string;
      message: string;
      uncertain: boolean;
      recipient: { machine: string; agent: string };
      input: AskAgainInput;
    }[]
  >([]);
  const [resetKey, setResetKey] = useState(0);
  const [busy, setBusy] = useState(false);
  const sending = useRef(false);
  const [error, setError] = useState<string>();
  const [recorded, setRecorded] = useState<DiscussionRef>();
  const { directory, retry } = useAgentDirectory(binding);
  const agents = directory.state === 'ready' ? directory.agents : undefined;
  const section = useRef<HTMLElement>(null);
  const [retrying, setRetrying] = useState(false);
  const focusAfterRetry = useRef(false);
  const attachments = useRef(discussion?.attachments);
  useEffect(() => {
    attachments.current = discussion?.attachments;
  }, [discussion?.attachments]);
  const [files] = useState(() => new AttachmentDraft(() => attachments.current));
  const staged = useSyncExternalStore(files.subscribe, files.getSnapshot);
  // Leaving unsent releases what the composer stored; the local bytes just drop.
  useEffect(() => () => files.dispose(), [files]);
  useEffect(() => {
    if (directory.state !== 'loading') setRetrying(false);
  }, [directory.state]);
  useEffect(() => {
    // A retry that had keyboard focus keeps it. The busy action is natively disabled, so focus
    // moves only once it is enabled again (failed) or gone (ready: the composer takes it).
    if (directory.state === 'loading' || retrying || !focusAfterRetry.current) return;
    focusAfterRetry.current = false;
    (directory.state === 'failed'
      ? section.current?.querySelector<HTMLElement>('[data-agent-retry] button')
      : section.current?.querySelector<HTMLElement>('[role="combobox"]')
    )?.focus();
  }, [directory.state, retrying]);
  useEffect(() => {
    onDraft?.(value, edit);
  }, [value, edit]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => onBusy?.(busy), [busy]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    if (initialized.current || directory.state === 'loading') return;
    initialized.current = true;
    const matches =
      creationRecipient && directory.state === 'ready'
        ? directory.agents.filter(
            (agent) =>
              agent.machine === creationRecipient.machineId &&
              agent.agent === creationRecipient.agentId,
          )
        : [];
    setEdit((previous) => {
      if (previous.value || previous.edited || !creationRecipient)
        return { ...previous, edited: previous.edited ?? false };
      const token = `@${matches.length === 1 ? matches[0].agentName : text.messageCreator}`;
      return {
        value: `${token} `,
        edited: false,
        mentions: [
          {
            key: { machine: creationRecipient.machineId, agent: creationRecipient.agentId },
            range: { start: 0, end: token.length },
          },
        ],
      };
    });
  }, [directory, creationRecipient]);
  const decide = (current: ComposerEdit) =>
    messageRecipient({
      writable: !blocked && !!discussion,
      edit: current,
      destinations: agents,
    });
  const decision = decide(edit);
  const quote = thread?.anchor?.exact ?? anchor?.exact ?? '';
  const audience = decision.agents
    .map((agent) => `@${agent.agentName}${agent.presence === 'offline' ? ' (offline)' : ''}`)
    .join(', ');
  const status = decision.tooMany
    ? text.messageRecipientLimit
    : decision.ambiguous.length
      ? text.messageAmbiguous(decision.ambiguous[0])
      : decision.unavailable.length
        ? text.messageRecipientUnavailable(decision.unavailable[0])
        : decision.unknown.length
          ? text.messageUnknown(decision.unknown[0], audience)
          : audience
            ? text.messageAsks(audience)
            : agents?.length === 0
              ? text.messageAgentsEmpty
              : text.messageComment;
  function clearDraft() {
    initialized.current = true;
    setEdit({ value: '', edited: true });
    setResetKey((key) => key + 1);
  }
  async function send(current: ComposerEdit = edit) {
    if (sending.current || recorded || blocked || !discussion) return;
    const admitted = decide(current);
    if (!admitted.allowed || (admitted.agents.length && !binding)) return;
    if (!current.value.trim()) {
      setError(text.messageWrite);
      return;
    }
    const captured = {
      value: current.value,
      destinations: structuredClone(admitted.agents),
      anchor: structuredClone(anchor),
      thread: thread?.ref,
      threadRevision: thread?.revision,
      title,
      conversation: captureConversation(thread, asks),
      url: location.href,
    };
    sending.current = true;
    setFailures([]);
    setBusy(true);
    setError(undefined);
    let origin: Awaited<ReturnType<ThreadBinding['create']>> | undefined;
    try {
      // Files upload only here, on the explicit Send, and the message is recorded only
      // once every one of them is stored; a refusal leaves text and chips as they are.
      const prepared = await files.prepare();
      if (!prepared.ok) {
        setError(text.attachBlocked[prepared.why]);
        return;
      }
      const attach = prepared.attach;
      origin = captured.thread
        ? await discussion.reply(captured.thread, captured.value, captured.threadRevision, attach)
        : chat
          ? await discussion.createChat(captured.value, attach)
          : await discussion.create(captured.value, captured.anchor, attach);
      files.committed();
      for (const destination of captured.destinations) {
        if (!binding) break;
        const input: AskAgainInput = {
          quote,
          comment: captured.value,
          title: captured.title,
          url: captured.url,
          context: { ...origin, conversation: captured.conversation },
        };
        const failure = {
          agent: destination.agentName,
          message: origin.message.id,
          recipient: { machine: destination.machine, agent: destination.agent },
          input,
        };
        let attempt: Awaited<ReturnType<AskBinding['prepare']>>;
        try {
          attempt = await binding.prepare({
            ...input,
            destination,
          });
        } catch {
          // Failed preparation grants no authority to invent an Ask record or retry a sibling.
          setFailures((previous) => [...previous, { ...failure, uncertain: false }]);
          continue;
        }
        try {
          const result = await attempt.send();
          if (
            result.adopted === false ||
            (result.adopted === undefined && result.state === 'uncertain')
          )
            setFailures((previous) => [
              ...previous,
              {
                ...failure,
                uncertain: result.adopted !== false,
              },
            ]);
        } catch {
          setFailures((previous) => [...previous, { ...failure, uncertain: true }]);
        }
      }
      clearDraft();
      committed(origin.thread);
    } catch (failure) {
      if (failure instanceof AttachmentStaleError) {
        files.stale(failure.attachmentIds);
        setError(text.attachBlocked.stale);
        return;
      }
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
  const attachable = !!discussion?.attachments && !busy && !blocked && !recorded;
  const takeFiles = (list: FileList | null | undefined, event: ClipboardEvent | DragEvent) => {
    // Adding only makes an inert local chip; the trusted Send is the first effect.
    if (!attachable || !list?.length) return false;
    event.preventDefault();
    void files.add([...list]);
    return true;
  };
  return (
    <section
      className="annotation-compose"
      data-testid="annotation-compose"
      ref={section}
      onDragOver={(event) => {
        if (attachable && event.dataTransfer.types.includes('Files')) event.preventDefault();
      }}
      onDrop={(event) => takeFiles(event.dataTransfer.files, event)}
      onPaste={(event) => {
        // A paste that carries text stays a text paste; only a files-only paste attaches.
        if (!event.clipboardData.types.includes('text/plain'))
          takeFiles(event.clipboardData.files, event);
      }}
    >
      <MessageComposer
        edit={edit}
        onChange={(next) => {
          initialized.current = true;
          setEdit({ ...next, edited: true });
          setError(undefined);
        }}
        label={text.messageLabel}
        placeholder={text.messagePlaceholder}
        candidates={agents}
        describedBy={statusId}
        resetKey={resetKey}
        autoFocus
        disabled={busy || (blocked && !recoveryRequired) || !!recorded || !discussion}
        onSubmit={(event, current) => {
          if (event.isTrusted && !event.isComposing && event.keyCode !== 229) void send(current);
        }}
        onCancel={(event) => {
          if (event.isTrusted && !event.isComposing && event.keyCode !== 229 && !sending.current)
            cancel();
        }}
      />
      {discussion?.attachments && <AttachmentChips draft={files} disabled={!attachable} />}
      <div className="annotation-status-row">
        {discussion?.attachments && <AttachButton draft={files} disabled={!attachable} />}
        <p id={statusId} role="status" className="annotation-hint">
          {busy
            ? text.messageSending
            : recoveryRequired
              ? text.reconnectToSend
              : binding && directory.state === 'loading'
                ? text.messageAgentsChecking
                : binding && directory.state === 'failed'
                  ? `${text.messageAgentsUnavailable} ${text.messageCommentAvailable}`
                  : status}
          {!busy &&
            !recoveryRequired &&
            binding &&
            directory.state === 'failed' &&
            directory.code && (
              <>
                {' '}
                <small className="failure-reference">
                  {text.failureCodeLabel}{' '}
                  <code className="tmt-ui-code" data-failure-reference>
                    {directory.code}
                  </code>
                </small>
              </>
            )}
        </p>
        {binding && !busy && !recoveryRequired && (directory.state === 'failed' || retrying) && (
          <span data-agent-retry>
            <BrowserAction
              type="button"
              label={text.messageAgentsRetry}
              variant="text"
              busy={retrying}
              onActivate={(event) => {
                if (!event.isTrusted) return;
                focusAfterRetry.current = event.currentTarget === document.activeElement;
                setRetrying(true);
                retry();
              }}
            />
          </span>
        )}
        <BrowserAction
          type="button"
          label={text.askSend}
          variant="primary"
          busy={busy}
          disabled={
            blocked ||
            staged.chips.some((chip) => chip.state.kind === 'uploading') ||
            !!recorded ||
            !discussion ||
            !value.trim() ||
            !decision.allowed ||
            (!!decision.agents.length && !binding)
          }
          onActivate={(event) => {
            if (event.isTrusted) void send();
          }}
        />
      </div>
      {failures.map((failure, index) => (
        <div
          role="status"
          data-testid="recipient-failure"
          data-message-id={failure.message}
          key={index}
        >
          <p>
            @{failure.agent} · {failure.uncertain ? text.askUnconfirmed : text.askNotDelivered}{' '}
            {!failure.uncertain && (
              <AskAgainAction
                binding={binding}
                blocked={blocked || busy || !!recorded}
                input={failure.input}
                recipient={failure.recipient}
                retryOf={null}
                settled={(outcome) => {
                  setFailures((previous) =>
                    outcome.adopted === true
                      ? previous.filter((value) => value !== failure)
                      : outcome.adopted === false
                        ? previous
                        : previous.map((value) =>
                            value === failure ? { ...value, uncertain: true } : value,
                          ),
                  );
                }}
              />
            )}
          </p>
          {failure.uncertain && <p className="ask-supporting">{text.askUncertain}</p>}
        </div>
      ))}
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
