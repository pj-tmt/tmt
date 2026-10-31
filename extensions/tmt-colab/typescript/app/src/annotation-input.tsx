import './annotation-composer.css';
import type { ComposerConnectionPresentation } from './router.js';
import { ConversationPresentation } from './components/conversation-presentation.js';
import { attachmentInputHandlers, useAttachmentDropTarget } from './attachment-drop.js';
import { BrowserAction, BrowserIconAction, type BrowserActionProps } from '@tmt/browser-ui/react';
import { ArrowUp } from 'lucide-react';
import {
  Fragment,
  useContext,
  useEffect,
  useId,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react';
import { useCreatorDefault } from './creator-default.js';
import { AttachmentDraft } from './attachment-draft.js';
import { AttachmentStaleError } from './attachment-service.js';
import { AttachButton, PendingAttachments } from './attachment-tray.js';
import { useAgentDirectory } from './agent-directory.js';
import { AgentChip, agentSentence, agentsSentence } from './components/agent-chip.js';
import { MessageComposer } from './components/message-composer.js';
import { MessageToolbar } from './components/message-toolbar.js';
import type { ComposerActivation, ComposerEdit } from './components/message-composer-edit.js';
import type { CreatorTarget } from './creator-default.js';
import { messageRecipient } from './message-recipient.js';
import {
  messageSendRefusal,
  messageSendCopy,
  messageSendSaved,
  messageSendFailureReference,
  type MessageSendReference,
  type MessageSendStage,
} from './message-send-refusal.js';
import { text } from './strings.js';
import { CommentSendFailures } from './comment-send-failures.js';
import type { AskBinding, PageAsk } from './ask-panel.js';
import type { ThreadBinding } from './thread-store.js';
import { MessageEchoContext, messageEcho, type MessageEcho } from './message-echo.js';
import { captureConversation } from './thread-store.js';
import type { DiscussionRef, QuoteSelector, ThreadView } from './thread-records.js';

import { sendCommentAsks, type MessageSendFailure } from './comment-send.js';
export type { MessageSendFailure } from './comment-send.js';

const sameKey = (a: { machine?: string; agent: string }, b: { machine?: string; agent: string }) =>
  a.machine === b.machine && a.agent === b.agent;

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
  activation,
  onDraft,
  onBusy,
  onFailure,
  onRecorded,
  blocked,
  recoveryRequired = false,
  connectionPresentation,
  archived = false,
  cancel,
  showCancel = false,
  committed,
}: {
  binding?: AskBinding;
  discussion?: ThreadBinding;
  anchor: QuoteSelector | null;
  thread?: ThreadView;
  asks: readonly PageAsk[];
  title: string;
  /** Admitted page/proposal UUIDs select the recipient; an optional label is display only. */
  creationRecipient?: CreatorTarget & { label?: string };
  /** A draft kept from an earlier close of the same selection. */
  initialValue?: string;
  initialEdit?: ComposerEdit;
  activation?: ComposerActivation;
  /** Reports every value, so the owner can keep an unsent draft. */
  onDraft?(value: string, edit: ComposerEdit): void;
  /** Reports this composer's send in flight; its owner keeps the composer mounted meanwhile. */
  onBusy?(busy: boolean): void;
  /** The message is recorded; its delivery may still be running (recovery is on the thread). */
  onRecorded?(): void;
  /** A closing owner retains delivery recovery outside the composer. */
  onFailure?(failure: MessageSendFailure): void;
  blocked: boolean;
  /** Recovery keeps local editing available; blocked still fences every publish. */
  recoveryRequired?: boolean;
  connectionPresentation?: ComposerConnectionPresentation;
  archived?: boolean;
  cancel(): void;
  showCancel?: boolean;
  committed(ref: DiscussionRef): void;
}) {
  const presentation = useContext(ConversationPresentation);
  const chat = presentation === 'chat';
  // With an echo host, the composer clears as the echo appears; a send that fails before it
  // is recorded leaves a failed row that holds the draft until Retry or Edit.
  const echoHost = useContext(MessageEchoContext);
  const creatorPreference = useCreatorDefault();
  const { directory, retry } = useAgentDirectory(binding);
  const agents = directory.state === 'ready' ? directory.agents : undefined;
  // Legacy preferences contain no label. Keep their existing directory lookup;
  // current creator metadata supplies both identity and presentation immediately.
  const legacyMatches =
    creationRecipient && creationRecipient.label === undefined
      ? agents?.filter(
          (agent) =>
            agent.agent === creationRecipient.agentId &&
            (creationRecipient.machineId === undefined ||
              agent.machine === creationRecipient.machineId),
        )
      : undefined;
  const legacyCreator = legacyMatches?.length === 1 ? legacyMatches[0] : undefined;
  const creatorName = creationRecipient?.label ?? legacyCreator?.agentName;
  const creator =
    creationRecipient && creatorName !== undefined
      ? {
          agent: creationRecipient.agentId,
          machine: creationRecipient.machineId ?? legacyCreator?.machine,
          agentName: creatorName,
        }
      : undefined;
  function freshEdit(): ComposerEdit {
    if (!creator || !creatorPreference.enabled) return { value: '', edited: false };
    const token = `@${creator.agentName}`;
    return {
      value: `${token} `,
      edited: false,
      mentions: [
        {
          key: { machine: creator.machine, agent: creator.agent },
          range: { start: 0, end: token.length },
          defaultCreator: true,
        },
      ],
    };
  }
  const [edit, setEdit] = useState<ComposerEdit>(
    () => initialEdit ?? (initialValue ? { value: initialValue } : freshEdit()),
  );
  const value = edit.value;
  const statusId = useId();
  const initialized = useRef(initialEdit !== undefined || !!initialValue || !!creator);
  const [failures, setFailures] = useState<MessageSendFailure[]>([]);
  function recordFailure(failure: MessageSendFailure) {
    if (onFailure) onFailure(failure);
    else setFailures((previous) => [...previous, failure]);
  }
  const [resetKey, setResetKey] = useState(0);
  const [busy, setBusy] = useState(false);
  // While an echo shows the send, the composer stays as it is: no grey, no busy mark.
  const [echoing, setEchoing] = useState(false);
  const latestEdit = useRef(edit);
  latestEdit.current = edit;
  // The draft a failed row holds. While it exists the composer is inert; only Retry and Edit act.
  const [unsent, setUnsent] = useState<{ edit: ComposerEdit; echo: MessageEcho }>();
  const unsentRef = useRef(unsent);
  unsentRef.current = unsent;
  const focusAfterEdit = useRef(false);
  const sending = useRef(false);
  const [error, setError] = useState<{
    reference: MessageSendReference;
    occurrence: number;
  }>();
  const failureSequence = useRef(0);
  const failureNotice = useRef<HTMLParagraphElement>(null);
  function refuse(reference: MessageSendReference) {
    // A repeated identical refusal must create a fresh alert for assistive technology.
    setError({ reference, occurrence: ++failureSequence.current });
  }
  useEffect(() => {
    if (error && failureNotice.current?.getClientRects().length)
      failureNotice.current.scrollIntoView({ block: 'nearest' });
  }, [error]);
  const [recorded, setRecorded] = useState<DiscussionRef>();
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
    // A failed row's draft is this composer's draft, so closing or reloading keeps it.
    const kept = unsent?.edit ?? edit;
    onDraft?.(kept.value, kept);
  }, [value, edit, unsent]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => onBusy?.(busy), [busy]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    if (initialized.current || !creator) return;
    initialized.current = true;
    setEdit((previous) => (previous.value || previous.edited ? previous : freshEdit()));
  }, [creationRecipient, directory, creatorPreference.enabled]); // eslint-disable-line react-hooks/exhaustive-deps
  const decide = (current: ComposerEdit) =>
    messageRecipient({
      writable: !archived && !blocked && !!discussion,
      edit: current,
      destinations: agents,
    });
  const decision = decide(edit);
  // The pill names everyone this message mentions, including an agent that cannot be asked now:
  // the helper line says why Send is held, and the list can still take that mention off.
  const asked = decision.mentions
    .filter(
      (mention, index, all) => all.findIndex((other) => sameKey(other.key, mention.key)) === index,
    )
    .map((mention) => ({
      ...mention.key,
      agentName:
        agents?.find((agent) => sameKey(agent, mention.key))?.agentName ??
        value.slice(mention.range.start + 1, mention.range.end),
    }));
  const quote = thread?.anchor?.exact ?? anchor?.exact ?? '';
  const audience = decision.agents.length > 0 && (
    <>
      {decision.agents.map((agent, index) => (
        <Fragment key={`${agent.machine}:${agent.agent}`}>
          {index > 0 && ', '}
          <AgentChip name={agent.agentName} mark={false} />
          {agent.presence === 'offline' && ' (offline)'}
        </Fragment>
      ))}
    </>
  );
  const status = decision.tooMany ? (
    text.messageRecipientLimit
  ) : decision.ambiguous.length ? (
    agentSentence(text.messageAmbiguous, decision.ambiguous[0])
  ) : decision.unavailable.length ? (
    agentSentence(text.messageRecipientUnavailable, decision.unavailable[0])
  ) : decision.unknown.length ? (
    audience ? (
      <>
        {text.messageUnknown(decision.unknown[0])} {agentsSentence(text.messageAsks, audience)}
      </>
    ) : (
      `${text.messageUnknown(decision.unknown[0])} ${text.messageUnknownComment}`
    )
  ) : audience ? (
    agentsSentence(text.messageAsksSend, audience)
  ) : agents?.length === 0 ? (
    chat ? (
      text.messageComment
    ) : (
      text.messageAgentsEmpty
    )
  ) : (
    text.messageCommentSend
  );
  const nextDraft = useRef(freshEdit);
  nextDraft.current = freshEdit;
  function clearDraft() {
    initialized.current = true;
    setEdit(nextDraft.current());
    setResetKey((key) => key + 1);
  }
  /** Edit: the failed row's full draft moves back into this composer and the row goes. */
  function editUnsent() {
    const held = unsentRef.current;
    if (!held) return;
    echoHost?.remove(held.echo.local);
    unsentRef.current = undefined;
    setUnsent(undefined);
    initialized.current = true;
    setEdit(held.edit);
    setResetKey((key) => key + 1);
    focusAfterEdit.current = true;
  }
  useEffect(() => {
    if (unsent || !focusAfterEdit.current) return;
    focusAfterEdit.current = false;
    section.current?.querySelector<HTMLElement>('[role="combobox"]')?.focus();
  }, [unsent]);
  const sendGate = (current: ComposerEdit, allowed: boolean, needsBinding: boolean) => ({
    sending: sending.current,
    recorded: !!recorded,
    archived,
    blocked,
    discussion: !!discussion,
    allowed,
    needsBinding,
    binding: !!binding,
    value: current.value,
  });
  async function send(current: ComposerEdit = edit, fromUnsentRow = false) {
    // While a failed row holds a draft, only its Retry sends; the composer hint says so.
    if (unsentRef.current && !fromUnsentRow) return;
    // Only this explicit retry or a deliberate text edit clears a previous refusal.
    setError(undefined);
    let stage: MessageSendStage = 'admission';
    let ownsSend = false;
    let origin: Awaited<ReturnType<ThreadBinding['create']>> | undefined;
    let echoed: MessageEcho | undefined;
    let failedReference: MessageSendReference | undefined;
    // With an echo, a failure before the record is reported once, on the failed row.
    const fail = (reference: MessageSendReference) => {
      if (echoed && !origin) failedReference = reference;
      else refuse(reference);
    };
    try {
      const admitted = decide(current);
      const refusal = messageSendRefusal(
        sendGate(current, admitted.allowed, !!admitted.agents.length),
      );
      if (refusal) {
        if (refusal !== 'S8') refuse(refusal);
        return;
      }
      stage = 'capture';
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
      ownsSend = true;
      setFailures([]);
      setBusy(true);
      if (echoHost) {
        const retried = unsentRef.current;
        if (retried) {
          echoHost.remove(retried.echo.local);
          unsentRef.current = undefined;
          setUnsent(undefined);
        }
        // The echo and the cleared composer appear in the same frame.
        echoed = messageEcho(
          captured.value,
          files.getSnapshot().chips.map(({ id, filename, mediaType, size }) => ({
            id,
            filename,
            mediaType,
            size,
            preview: files.preview(id),
          })),
          captured.destinations.map((destination) => destination.agentName),
        );
        echoHost.show(echoed);
        setEchoing(true);
        if (current.value === latestEdit.current.value) clearDraft();
      }
      stage = 'attachments';
      const prepared = await files.prepare();
      if (!prepared.ok) {
        fail('S10');
        return;
      }
      const attach = prepared.attach;
      stage = 'record';
      origin = captured.thread
        ? await discussion!.reply(captured.thread, captured.value, captured.threadRevision, attach)
        : chat
          ? await discussion!.createChat(captured.value, attach)
          : await discussion!.create(captured.value, captured.anchor, attach);
      onRecorded?.();
      stage = 'dispatch';
      if (echoed) echoHost?.show((echoed = { ...echoed, message: origin.message }));
      files.committed();
      const deliveries: MessageEcho['deliveries'][number][] = [];
      const failures = await sendCommentAsks(
        binding,
        origin,
        captured.destinations,
        {
          quote,
          comment: captured.value,
          title: captured.title,
          url: captured.url,
          conversation: captured.conversation,
        },
        undefined,
        (operationId, outcome) => {
          if (
            outcome.adopted === true &&
            outcome.state !== 'preview' &&
            outcome.state !== 'preparing'
          )
            deliveries.push({ operationId, state: outcome.state });
        },
      );
      if (echoed) echoed = { ...echoed, deliveries };
      failures.forEach(recordFailure);
      if (!echoed) clearDraft();
      setError(undefined);
      committed(origin.thread);
    } catch (failure) {
      if (!origin && failure instanceof AttachmentStaleError) {
        files.stale(failure.attachmentIds);
        fail('S14');
        return;
      }
      fail(origin ? 'S13' : messageSendFailureReference(stage, failure));
      if (origin) {
        if (!echoed) clearDraft();
        setRecorded(origin.thread);
      }
    } finally {
      // A refused second activation never releases the first send's busy fence.
      if (ownsSend) {
        sending.current = false;
        setBusy(false);
        setEchoing(false);
      }
      if (echoed && origin) echoHost?.show({ ...echoed, sending: false });
      if (echoed && !origin) {
        // Not recorded: the echo becomes the failed row, which holds this exact draft.
        const failed: MessageEcho = {
          ...echoed,
          failure: {
            reference: failedReference ?? messageSendFailureReference(stage, undefined),
            retry: () => void send(current, true),
            edit: editUnsent,
          },
        };
        echoHost?.show(failed);
        unsentRef.current = { edit: current, echo: failed };
        setUnsent(unsentRef.current);
        // Reported directly: a composer closed mid-send never runs the draft effect again.
        onDraft?.(current.value, current);
      }
    }
  }
  const attachable =
    !archived && !!discussion?.attachments && !busy && !blocked && !recorded && !unsent;
  const drop = useAttachmentDropTarget(files, attachable);
  const fileHandlers = attachmentInputHandlers(files, attachable, !!drop);
  const recipientHint = (
    <div
      id={statusId}
      role="status"
      className="annotation-hint"
      data-unsent={(!connectionPresentation && unsent) || undefined}
      tabIndex={connectionPresentation?.phase === 'ended' ? 0 : undefined}
    >
      {connectionPresentation
        ? connectionPresentation.body
        : unsent
          ? text.messageUnsentFirst
          : recoveryRequired
            ? text.sendOnHold
            : binding && directory.state === 'loading'
              ? text.messageAgentsChecking
              : binding && directory.state === 'failed'
                ? `${text.messageAgentsUnavailable} ${text.messageCommentAvailable}`
                : status}
      {!busy &&
        !recoveryRequired &&
        !connectionPresentation &&
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
    </div>
  );
  const sendFailureNotice = error && (
    <p
      key={error.occurrence}
      ref={failureNotice}
      role="alert"
      className="annotation-send-notice"
      data-send-refusal={error.reference}
    >
      {!messageSendSaved(error.reference) && (
        <span className="annotation-send-mark" aria-hidden="true">
          ✗{' '}
        </span>
      )}
      {messageSendCopy(error.reference, decision.agents[0]?.agentName)}{' '}
      {text.messageSendReference(error.reference)}
    </p>
  );
  if (archived)
    return (
      <section className="annotation-compose" data-testid="annotation-compose" ref={section}>
        <ArchivedDraft value={value} />
        <p className="annotation-hint">{text.archivePosting}</p>
        {sendFailureNotice}
      </section>
    );
  // A send in flight fences the composer without greying it; any other reason keeps the disabled look.
  const inFlight = busy && !echoing,
    inert = !!unsent || (blocked && !recoveryRequired) || !!recorded || !discussion;
  const sendActionProps = {
    type: 'button',
    label: text.askSend,
    variant: 'primary',
    busy: busy && !echoing,
    disabled:
      !!messageSendRefusal({
        ...sendGate(edit, decision.allowed, !!decision.agents.length),
        sending: busy,
      }) ||
      !!unsent ||
      staged.chips.some((chip) => chip.state.kind === 'uploading'),
    onActivate: (event) => {
      if (event.isTrusted) void send();
    },
  } satisfies BrowserActionProps;
  return (
    <section
      className="annotation-compose"
      data-chat={chat || undefined}
      data-unsent={unsent ? '' : undefined}
      data-connection={connectionPresentation?.phase}
      data-testid="annotation-compose"
      ref={section}
      {...fileHandlers}
    >
      <MessageComposer
        activation={activation}
        edit={edit}
        onChange={(next) => {
          initialized.current = true;
          // A toggle-added chip plus whitespace is still only a default seed. Manual
          // chips and nonblank body edits remain deliberately edited drafts.
          const remainder = next.mentions
            ?.filter((mention) => mention.defaultCreator)
            .reduceRight(
              (body, mention) => body.slice(0, mention.range.start) + body.slice(mention.range.end),
              next.value,
            );
          const seedOnly =
            next.mentions?.some((mention) => mention.defaultCreator) && !remainder?.trim();
          setEdit({ ...next, edited: !seedOnly });
          // Mention normalization can report a new edit object without any deliberate text edit.
          if (next.value !== value) setError(undefined);
        }}
        label={text.messageLabel}
        placeholder={text.messagePlaceholder}
        candidates={agents}
        creatorDefault={
          creator
            ? {
                key: { machine: creator.machine, agent: creator.agent },
                label: creator.agentName,
                enabled: creatorPreference.enabled,
                revision: creatorPreference.revision,
              }
            : undefined
        }
        accessory={
          <>
            {sendFailureNotice}
            {/* Sent files leave with the echo; a failed row holds them until Edit. */}
            {discussion?.attachments && !echoing && !unsent && (
              <PendingAttachments
                draft={files}
                disabled={!attachable}
                onRemoved={() => {
                  section.current
                    ?.querySelector<HTMLElement>('[role="combobox"]')
                    ?.focus({ preventScroll: true });
                }}
              />
            )}
            <MessageToolbar
              attach={
                discussion?.attachments && <AttachButton draft={files} disabled={!attachable} />
              }
              send={<BrowserIconAction {...sendActionProps} icon={<ArrowUp />} />}
              asked={asked}
              candidates={agents}
              creator={
                creator
                  ? {
                      name: creator.agentName,
                      enabled: creatorPreference.enabled,
                      toggle: creatorPreference.toggle,
                    }
                  : undefined
              }
              disabled={inFlight || inert}
            />
          </>
        }
        describedBy={statusId}
        resetKey={resetKey}
        autoFocus
        disabled={inFlight || inert}
        busy={inFlight && !inert}
        onSubmit={(event, current) => {
          if (event.isTrusted && !event.isComposing && event.keyCode !== 229) void send(current);
        }}
        onCancel={(event) => {
          if (
            event.isTrusted &&
            !event.repeat &&
            !event.isComposing &&
            event.keyCode !== 229 &&
            !event.altKey &&
            !event.ctrlKey &&
            !event.metaKey &&
            !event.shiftKey &&
            !sending.current
          )
            cancel();
        }}
      />
      <div className="annotation-helper">
        {recipientHint}
        {binding &&
          !busy &&
          !recoveryRequired &&
          !connectionPresentation &&
          (directory.state === 'failed' || retrying) && (
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
        {showCancel && (
          <BrowserAction
            type="button"
            variant="text"
            label={text.commentCancel}
            disabled={busy}
            onActivate={(event) => {
              if (event.isTrusted && !sending.current) cancel();
            }}
          />
        )}
      </div>
      <CommentSendFailures
        failures={failures}
        binding={binding}
        blocked={blocked || busy || !!recorded}
        settled={(failure, outcome) =>
          setFailures((previous) =>
            outcome.adopted === true
              ? previous.filter((value) => value !== failure)
              : outcome.adopted === false
                ? previous
                : previous.map((value) =>
                    value === failure ? { ...value, uncertain: true } : value,
                  ),
          )
        }
      />
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

/** Retained local text has no publication actions once its page is archived. */
export function ArchivedDraft({ value }: { value: string }) {
  const [failed, setFailed] = useState(false);
  return value ? (
    <div className="archive-unsent-draft">
      <p>{text.unsentDraft}</p>
      <pre>{value}</pre>
      <BrowserAction
        type="button"
        variant="text"
        label={text.copyDraft}
        onActivate={(event) => {
          if (event.isTrusted)
            void navigator.clipboard.writeText(value).catch(() => setFailed(true));
        }}
      />
      {failed && <p role="alert">{text.copyDraftFailed}</p>}
    </div>
  ) : null;
}
