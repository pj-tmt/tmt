import { BrowserAction, BrowserChips, BrowserTabs, type BrowserTab } from '@tmt/browser-ui/react';
import { useEffect, useRef, useState } from 'react';
import { AgentList } from './agent-chip.js';
import type { AnchorPosition } from '../renderer.js';
import type { ComposerConnectionPresentation } from '../router.js';
import { sendAnswers, askedMessageIds, type BatchAnswer } from '../answer-batch.js';
import type { AskAgainInput } from '../ask-again.js';
import type { AskBinding, PageAsk } from '../ask-panel.js';
import { sameMessageIds } from '../ask-records.js';
import { useAgentDirectory } from '../agent-directory.js';
import { CommentSendFailures } from '../comment-send-failures.js';
import type { MessageSendFailure } from '../comment-send.js';
import type { AgentDestination } from '../live-ask.js';
import {
  messageRecipient,
  recipientEdit,
  recipientId,
  MESSAGE_RECIPIENT_LIMIT,
} from '../message-recipient.js';
import {
  answerComment,
  answerOf,
  canonicalQuestionThread,
  currentAnswer,
  type Question,
  type QuestionGroup,
} from '../questions.js';
import type { CommentContext, ThreadBinding } from '../thread-store.js';
import { conversationAsks } from '../thread-store.js';
import type { QuoteSelector, ThreadView } from '../thread-records.js';
import { text } from '../strings.js';
import { useNotificationModalPresence } from '../reply-notification-context.js';
import { QuestionCard, type QuestionDraft } from './question-card.js';

interface Draft extends QuestionDraft {
  pending?: CommentContext;
  failed?: boolean;
}
const emptyDraft = (): Draft => ({ choices: [], other: false, value: '', changing: false });

/** One admitted group owns all drafts, visible recipients, publication outcomes and its footer. */
export function QuestionGroupCard({
  group,
  members,
  selected,
  onSelect,
  associated,
  selectors,
  attached,
  placement,
  discussion,
  ask,
  asks,
  title,
  blocked,
  readOnly = false,
  chip = false,
  connectionPresentation,
  onRecorded,
}: {
  group: QuestionGroup;
  members: readonly Question[];
  selected: string;
  onSelect(id: string): void;
  associated(id: string): readonly ThreadView[];
  selectors: ReadonlyMap<string, QuoteSelector>;
  attached: boolean;
  placement?: AnchorPosition;
  discussion?: ThreadBinding;
  ask?: AskBinding;
  asks: readonly PageAsk[];
  title: string;
  blocked: boolean;
  readOnly?: boolean;
  /** Chip-form panel: each card shows its admitted title and the band only once it is answered. */
  chip?: boolean;
  connectionPresentation?: ComposerConnectionPresentation;
  /** Called after an explicit Send recorded answers, with the recorded comments. */
  onRecorded?(questionIds: readonly string[], contexts: readonly CommentContext[]): void;
}) {
  const [drafts, setDrafts] = useState<Record<string, Draft>>({});
  const [recipientState, setRecipientState] = useState<{
    recipients: AgentDestination[];
    initialized: boolean;
  }>({ recipients: [], initialized: false });
  const recipients = recipientState.recipients;
  const [query, setQuery] = useState('');
  const [sending, setSending] = useState<readonly string[]>([]);
  const busy = useRef(false);
  const [error, setError] = useState<string>();
  const [failures, setFailures] = useState<MessageSendFailure[]>([]);
  const [focusReply, setFocusReply] = useState<{ id: string; placement?: AnchorPosition }>();
  const [focusChange, setFocusChange] = useState(false);
  const section = useRef<HTMLDivElement>(null);
  const modalPresence = useNotificationModalPresence();
  const { directory, retry } = useAgentDirectory(ask);
  const agents = directory.state === 'ready' ? directory.agents : undefined;
  // Commit the defaults in the same render as the resolved directory: an effect
  // would briefly expose an enabled comment-only Send before its chips appear.
  if (!recipientState.initialized && agents) {
    const defaults: AgentDestination[] = [];
    for (const question of members) {
      const matches = agents.filter((agent) => agent.agentName === question.agent);
      if (matches.length !== 1) continue;
      const agent = matches[0];
      if (!defaults.some((other) => recipientId(other) === recipientId(agent)))
        defaults.push(agent);
      if (defaults.length === MESSAGE_RECIPIENT_LIMIT) break;
    }
    setRecipientState({ recipients: defaults, initialized: true });
  }
  const resolvingDefaults = !!ask && !recipientState.initialized && directory.state === 'loading';
  useEffect(() => {
    setDrafts((previous) => {
      const verified = members.filter((question) => {
        const context = previous[question.id]?.pending;
        return (
          context &&
          associated(question.id).some((thread) =>
            thread.comments.some(
              (comment) =>
                !comment.deleted &&
                comment.deviceAuthor === true &&
                comment.ref.writer === context.message.writer &&
                comment.ref.id === context.message.id,
            ),
          )
        );
      });
      if (!verified.length) return previous;
      const next = { ...previous };
      for (const question of verified) next[question.id] = emptyDraft();
      return next;
    });
  }, [members, associated]);
  useEffect(() => {
    if (!focusReply || focusReply.id !== selected || !attached || !selectors.has(selected)) return;
    // A tab change retires the old description band. Focus after the slot owner
    // admits the destination's placement, rather than on the old visible geometry.
    if (!placement || placement === focusReply.placement) return;
    const node = section.current
      ?.querySelector<HTMLElement>(`[id="question-panel-${selected}"] [data-testid="ask-reply"]`)
      ?.closest<HTMLElement>('[data-operation-id]');
    if (!node || section.current?.closest('[data-detached="true"]')) return;
    node.focus({ preventScroll: true });
    if (node.ownerDocument.activeElement === node) setFocusReply(undefined);
  }, [focusReply, selected, attached, placement, selectors, associated, asks]);
  const edit = recipientEdit(recipients);
  const decision = messageRecipient({
    writable: !readOnly && !blocked && !!discussion,
    edit,
    destinations: agents,
  });
  const audience = recipients.map((agent) => `@${agent.agentName}`).join(', ');
  const rows = members.map((question) => {
    const draft = drafts[question.id] ?? emptyDraft();
    const threads = associated(question.id);
    const thread = canonicalQuestionThread(question, threads);
    const answer = currentAnswer(question, thread?.comments ?? []);
    const body = answerComment(
      question,
      draft.choices,
      draft.value,
      draft.other,
      title,
      edit.value,
    );
    const eligible = !!body && (!answer || draft.changing || draft.failed) && !draft.pending;
    return { question, draft, threads, thread, answer, body, eligible };
  });
  // The Send row and its recipients belong to an answer being written; a folded answered line
  // never shows them and Change answer brings them back. A tabbed group's row also stays while
  // another tab still has drafts to send or a Send is in flight.
  const active = rows.find(
    (row) => row.question.id === (members.length > 1 ? selected : members[0].id),
  );
  const composing = !!active && (!active.answer || active.draft.changing);
  const activeAnswered = !!active && !composing;
  const pending = rows.filter((row) => row.eligible);
  const sendable = composing || (members.length > 1 && (pending.length > 0 || sending.length > 0));

  // After a Send, focus rests on the group until the answer is verified; then it moves to the folded
  // line's Change answer action, unless the reader has already moved on.
  useEffect(() => {
    if (!focusChange || !activeAnswered) return;
    const root =
      members.length > 1
        ? section.current?.querySelector(`[id="question-panel-${selected}"]`)
        : section.current;
    const target = root?.querySelector<HTMLElement>('.needs-you-actions button:not(:disabled)');
    // Change answer stays disabled until the verified answer replaces the recorded draft.
    if (!target) return;
    const focused = section.current?.ownerDocument.activeElement;
    if (!focused || focused === section.current || focused.tagName === 'BODY')
      target.focus({ preventScroll: true });
    setFocusChange(false);
  }, [focusChange, activeAnswered, members.length, selected, drafts, sending, blocked, discussion]);
  const complete = rows.filter((row) => row.answer || row.body).length;
  const sendingCount = sending.length || pending.length;
  const sendLabel =
    members.length === 1
      ? 'Send answer'
      : sendingCount === 0
        ? 'Send answers'
        : `Send ${sendingCount} ${sendingCount === 1 ? 'answer' : 'answers'}`;
  const disabled = blocked || readOnly || !!sending.length || !discussion;
  async function send() {
    if (
      busy.current ||
      disabled ||
      resolvingDefaults ||
      !decision.allowed ||
      !pending.length ||
      !attached
    )
      return;
    const answers: BatchAnswer[] = [];
    for (const row of pending) {
      const selector = selectors.get(row.question.id);
      if (!selector) return;
      answers.push({
        question: structuredClone(row.question),
        body: row.body!,
        selector: structuredClone(selector),
        ...(row.thread ? { thread: structuredClone(row.thread) } : {}),
      });
    }
    setFocusReply(undefined);
    setFocusChange(false);
    busy.current = true;
    setSending(answers.map((answer) => answer.question.id));
    setError(undefined);
    // Keep the chosen tab even when the verified answer arrives; Send never advances it.
    onSelect(selected);
    try {
      const result = await sendAnswers(
        { discussion: discussion!, ask },
        {
          group: structuredClone(group),
          answers,
          recipients: structuredClone(decision.agents),
          title,
          url: location.href,
        },
        asks,
      );
      if ('refused' in result) {
        setError('These answers are too long to send together. Send fewer answers.');
        return;
      }
      setDrafts((previous) => {
        const next = { ...previous };
        for (const outcome of result.answers)
          next[outcome.question] = {
            ...(previous[outcome.question] ?? emptyDraft()),
            pending: outcome.state === 'recorded' ? outcome.context : undefined,
            failed: outcome.state === 'failed',
            ...(outcome.state === 'failed' ? { changing: true } : {}),
          };
        return next;
      });
      setFailures((previous) => [...previous, ...result.failures]);
      const recorded = result.answers.filter((outcome) => outcome.state === 'recorded');
      if (recorded.length)
        onRecorded?.(
          recorded.map((outcome) => outcome.question),
          recorded.map((outcome) => outcome.context!),
        );
      if (recorded.length) setFocusChange(true);
      section.current?.focus({ preventScroll: true });
    } catch {
      setError('Could not confirm these answers. Your drafts are kept.');
    } finally {
      busy.current = false;
      setSending([]);
    }
  }
  /** Reload derives the same batch from admitted records and verified own answer comments. */
  function retryInput(record: PageAsk): AskAgainInput | undefined {
    if (!record.messageIds || record.messageIds.length < 2) return;
    const answers = rows.flatMap(({ question, threads }) =>
      threads.flatMap((thread) =>
        thread.comments
          .filter(
            (comment) =>
              !comment.deleted &&
              comment.deviceAuthor === true &&
              comment.ref.writer === record.writer &&
              record.messageIds!.includes(comment.messageId) &&
              answerOf(question, comment.body) !== undefined,
          )
          .map((comment) => ({
            question: question.id,
            thread,
            comment,
            context: {
              thread: thread.ref,
              threadRevision: thread.revision,
              message: comment.ref,
              messageRevision: comment.revision,
            },
          })),
      ),
    );
    if (answers.length !== record.messageIds.length) return;
    const canonical = answers.find((answer) => answer.thread.threadId === record.thread);
    if (
      !canonical ||
      canonical !== answers[0] ||
      !sameMessageIds(record.messageIds, answers.map((answer) => answer.context.message.id).sort())
    )
      return;
    return {
      context: canonical.context,
      quote: canonical.thread.anchor?.exact ?? '',
      comment: canonical.comment.body,
      title,
      url: location.href,
      batch: {
        group: group.id,
        answers: answers.map(({ question, context }) => ({ question, context })),
      },
    };
  }
  const tabs: BrowserTab[] = rows.map(({ question, answer, body }, index) => ({
    id: question.id,
    tabId: `question-tab-${question.id}`,
    panelId: `question-panel-${question.id}`,
    label: question.label ?? `Q${index + 1}`,
    state:
      answer || body
        ? { mark: '✓', word: 'Answered', tone: 'working' }
        : { mark: '◆', word: 'Waiting', tone: 'waiting' },
  }));
  function card(question: Question) {
    const row = rows.find((value) => value.question.id === question.id)!;
    const records = conversationAsks(row.thread, asks).filter(
      (record) => row.answer && record.messageIds?.includes(row.answer.messageId),
    );
    const batch = records.filter((record) => (record.messageIds?.length ?? 0) > 1);
    const canonical = rows.find((value) =>
      value.threads.some((thread) => thread.threadId === batch[0]?.thread),
    );
    const names = [
      ...new Map(records.map((record) => [recipientId(record), record.agentName])).values(),
    ];
    const sharedAsk = records.length ? (
      <>
        Asked <AgentList names={names} />
        {batch.length ? ` with ${batch[0].messageIds!.length} answers.` : ''}
      </>
    ) : undefined;
    const hasOtherReply =
      canonical &&
      canonical.question.id !== question.id &&
      batch.some((record) => record.reply !== undefined);
    return (
      <QuestionCard
        key={question.id}
        question={question}
        draft={row.draft}
        onDraft={(patch) =>
          setDrafts((previous) => ({
            ...previous,
            [question.id]: { ...(previous[question.id] ?? emptyDraft()), ...patch, failed: false },
          }))
        }
        threads={row.threads}
        discussion={discussion}
        ask={ask}
        asks={asks.filter(
          (record) =>
            (record.messageIds?.length ?? 0) < 2 ||
            row.threads.some((thread) => thread.threadId === record.thread),
        )}
        title={title}
        blocked={blocked}
        readOnly={readOnly}
        connectionPresentation={connectionPresentation}
        descriptionHeight={placement?.descriptionHeight ?? 0}
        titled={chip}
        busy={sending.includes(question.id)}
        recorded={!!row.draft.pending}
        body={row.body}
        error={
          row.draft.failed
            ? 'Could not confirm this answer. Your draft is kept for the next Send.'
            : undefined
        }
        sharedAsk={sharedAsk}
        replyLabel={canonical?.question.label ?? `Q${canonical ? rows.indexOf(canonical) + 1 : 1}`}
        onOpenReply={
          hasOtherReply
            ? () => {
                onSelect(canonical.question.id);
                setFocusReply({ id: canonical.question.id, placement });
              }
            : undefined
        }
        onSend={
          members.length === 1
            ? () => {
                void send();
              }
            : undefined
        }
        retryInput={retryInput}
      />
    );
  }
  const options = (agents ?? [])
    .filter((agent) => agent.agentName.toLowerCase().includes(query.toLowerCase()))
    .map((agent) => ({
      id: recipientId(agent),
      label: `@${agent.agentName}`,
      description: agent.machineName,
      disabled: agent.online !== 'online',
    }));
  return (
    <div className="question-group-card" ref={section} tabIndex={-1}>
      {members.length > 1 ? (
        <BrowserTabs
          label={group.title ?? 'Questions'}
          tabs={tabs}
          selected={selected}
          status={`${complete} of ${members.length} answered`}
          onSelect={(id, _reason, event) => {
            if (event.isTrusted) {
              setFocusReply(undefined);
              setFocusChange(false);
              onSelect(id);
            }
          }}
        >
          {(tab) => card(members.find((question) => question.id === tab.id)!)}
        </BrowserTabs>
      ) : (
        card(members[0])
      )}
      {!readOnly && (
        <div className="question-group-footer">
          {sendable && (
            <>
              <div className="question-group-send">
                <BrowserChips
                  onOpenChange={modalPresence}
                  label="Answer recipients"
                  items={recipients.map((agent) => ({
                    id: recipientId(agent),
                    label: `@${agent.agentName}`,
                    removeLabel: `Remove @${agent.agentName}`,
                  }))}
                  options={options}
                  query={query}
                  onQueryChange={setQuery}
                  addLabel="Add recipient"
                  searchLabel="Find an agent"
                  emptyText={resolvingDefaults ? '' : 'No agents match.'}
                  maxItems={MESSAGE_RECIPIENT_LIMIT}
                  limitText="At most 8"
                  statusText={
                    directory.state === 'failed'
                      ? text.messageAgentsUnavailable
                      : directory.state === 'loading' && ask && !resolvingDefaults
                        ? text.messageAgentsChecking
                        : undefined
                  }
                  disabled={disabled}
                  addDisabled={directory.state !== 'ready'}
                  focusFallback={() => section.current}
                  onAdd={(id, event) => {
                    if (
                      !event.isTrusted ||
                      disabled ||
                      recipients.length >= MESSAGE_RECIPIENT_LIMIT
                    )
                      return;
                    const matches =
                      agents?.filter(
                        (agent) => recipientId(agent) === id && agent.online === 'online',
                      ) ?? [];
                    if (
                      matches.length !== 1 ||
                      recipients.some((agent) => recipientId(agent) === id)
                    )
                      return;
                    setRecipientState((previous) => ({
                      recipients: [...previous.recipients, matches[0]],
                      initialized: true,
                    }));
                    setQuery('');
                  }}
                  onRemove={(id, event) => {
                    if (!event.isTrusted || disabled) return;
                    setRecipientState((previous) => ({
                      recipients: previous.recipients.filter((agent) => recipientId(agent) !== id),
                      initialized: true,
                    }));
                  }}
                />
                <BrowserAction
                  label={`${sendLabel} · to ${audience || 'thread only'}`}
                  visibleLabel={sendLabel}
                  type="button"
                  variant="text"
                  busy={!!sending.length || resolvingDefaults}
                  disabled={
                    disabled ||
                    resolvingDefaults ||
                    !attached ||
                    !pending.length ||
                    !decision.allowed ||
                    pending.some((row) => !selectors.has(row.question.id))
                  }
                  onActivate={(event) => {
                    if (event.isTrusted) void send();
                  }}
                />
              </div>
              {connectionPresentation ? (
                <p className="annotation-hint" role="status">
                  {connectionPresentation.body}
                </p>
              ) : (
                <p className="annotation-hint" role="status">
                  {resolvingDefaults
                    ? text.messageAgentsChecking
                    : decision.unavailable.length
                      ? text.messageRecipientUnavailable(decision.unavailable[0])
                      : !recipients.length
                        ? 'Stays on the page, no one is asked'
                        : undefined}
                </p>
              )}
              {!connectionPresentation && directory.state === 'failed' && (
                <BrowserAction
                  label={text.messageAgentsRetry}
                  variant="text"
                  type="button"
                  disabled={!!sending.length}
                  onActivate={(event) => {
                    if (event.isTrusted) retry();
                  }}
                />
              )}
            </>
          )}
          {error && <p role="alert">{error}</p>}
          <CommentSendFailures
            failures={failures.filter(
              (failure) =>
                !asks.some(
                  (record) =>
                    record.agent === failure.recipient.agent &&
                    record.machine === failure.recipient.machine &&
                    sameMessageIds(record.messageIds ?? [], askedMessageIds(failure.input)),
                ),
            )}
            binding={ask}
            blocked={disabled}
            settled={(failure, outcome) =>
              setFailures((previous) =>
                outcome.adopted === true
                  ? previous.filter((item) => item !== failure)
                  : outcome.adopted === false
                    ? previous
                    : previous.map((item) =>
                        item === failure ? { ...item, uncertain: true } : item,
                      ),
              )
            }
          />
        </div>
      )}
    </div>
  );
}
