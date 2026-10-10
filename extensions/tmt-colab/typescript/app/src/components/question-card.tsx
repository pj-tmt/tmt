import { BrowserAction } from '@tmt/browser-ui/react';
import { useEffect, useId, useRef, useState } from 'react';
import { useAgentDirectory } from '../agent-directory.js';
import { CommentSendFailures } from '../comment-send-failures.js';
import { sendCommentAsks, type MessageSendFailure } from '../comment-send.js';
import { messageRecipient } from '../message-recipient.js';
import {
  answerComment,
  canonicalQuestionThread,
  currentAnswer,
  QUESTION_SHORT_POINTS,
  QUESTION_LONG_BYTES,
  type Question,
} from '../questions.js';
import { CommentExchange } from '../thread-panel.js';
import { captureConversation } from '../thread-store.js';
import type { CommentContext } from '../thread-store.js';
import type { PageAsk, AskBinding } from '../ask-panel.js';
import type { ThreadBinding } from '../thread-store.js';
import type { QuoteSelector, ThreadView } from '../thread-records.js';
import { ArchivedDraft } from '../annotation-input.js';
import { MessageComposer } from './message-composer.js';
import type { ComposerEdit } from './message-composer-edit.js';
import { text } from '../strings.js';

/** All controls and answer state live in the parent. Markup names never grant routing authority. */
export function QuestionCard({
  question,
  selector,
  threads,
  discussion,
  ask,
  asks,
  title,
  blocked,
  readOnly = false,
}: {
  question: Question;
  selector?: QuoteSelector;
  threads: readonly ThreadView[];
  discussion?: ThreadBinding;
  ask?: AskBinding;
  asks: readonly PageAsk[];
  title: string;
  blocked: boolean;
  readOnly?: boolean;
}) {
  const heading = useId();
  const [choices, setChoices] = useState<string[]>([]);
  const [other, setOther] = useState(false);
  const [value, setValue] = useState('');
  const [recipient, setRecipient] = useState<ComposerEdit>({ value: '' });
  const editedRecipient = useRef(false);
  const [busy, setBusy] = useState(false);
  const sending = useRef(false);
  const [error, setError] = useState<string>();
  const [failures, setFailures] = useState<MessageSendFailure[]>([]);
  const [changing, setChanging] = useState(false);
  const [recorded, setRecorded] = useState<CommentContext>();
  const section = useRef<HTMLElement>(null);
  const { directory, retry } = useAgentDirectory(ask);
  const agents = directory.state === 'ready' ? directory.agents : undefined;
  useEffect(() => {
    if (editedRecipient.current || !agents) return;
    editedRecipient.current = true;
    const matches = agents.filter((agent) => agent.agentName === question.agent);
    if (matches.length !== 1) return;
    const agent = matches[0];
    const token = `@${agent.agentName}`;
    setRecipient({
      value: token,
      mentions: [
        {
          key: { machine: agent.machine, agent: agent.agent },
          range: { start: 0, end: token.length },
        },
      ],
    });
  }, [agents, question.agent]);
  useEffect(() => {
    if (
      recorded &&
      threads.some((thread) =>
        thread.comments.some(
          (comment) =>
            comment.ref.writer === recorded.message.writer &&
            comment.ref.id === recorded.message.id,
        ),
      )
    )
      setRecorded(undefined);
  }, [recorded, threads]);
  const thread = canonicalQuestionThread(question, threads);
  const answer = currentAnswer(question, thread?.comments ?? []);
  const answerText = answer?.body.slice(`Answer to “${question.title}”: `.length);
  const pageLine = answerText?.lastIndexOf('\nPage: ') ?? -1;
  const decision = messageRecipient({
    writable: !readOnly && !blocked && !!discussion,
    edit: recipient,
    destinations: agents,
  });
  const audience = decision.agents.map((agent) => `@${agent.agentName}`).join(', ');
  const body = answerComment(question, choices, value, other, title, recipient.value);
  async function send() {
    if (
      sending.current ||
      recorded ||
      readOnly ||
      blocked ||
      !discussion ||
      !selector ||
      !body ||
      !decision.allowed
    )
      return;
    // Freeze recipients from their visible field only: question/choice @text cannot add recipients.
    const captured = {
      body,
      selector: structuredClone(selector),
      thread,
      destinations: structuredClone(decision.agents),
      conversation: captureConversation(thread, asks),
      title,
      url: location.href,
    };
    sending.current = true;
    setBusy(true);
    setError(undefined);
    let origin: CommentContext | undefined;
    try {
      origin = captured.thread
        ? await discussion.reply(captured.thread.ref, captured.body, captured.thread.revision)
        : await discussion.create(captured.body, captured.selector);
      setRecorded(origin);
      const failed = await sendCommentAsks(ask, origin, captured.destinations, {
        quote: captured.selector.exact,
        comment: captured.body,
        title: captured.title,
        url: captured.url,
        conversation: captured.conversation,
      });
      setFailures((previous) => [...previous, ...failed]);
      setChanging(false);
      // The committed view owns Answered. A local callback/iframe claim cannot supply it.
      section.current?.focus({ preventScroll: true });
    } catch {
      setError(origin ? text.messageRecordedUncertain : text.messageRecordFailed);
    } finally {
      sending.current = false;
      setBusy(false);
    }
  }
  const disabled = blocked || busy || !!recorded || !discussion;
  const compose = !answer || changing;
  return (
    <section className="question-card" aria-label={question.title} ref={section} tabIndex={-1}>
      <p className="question-state" role="status">
        {answer ? `✓ ${answer.deviceName} answered` : '◆ Waiting for you'}
      </p>
      {answer && (
        <>
          <p className="question-answer">
            {pageLine >= 0 ? answerText?.slice(0, pageLine) : answerText}
          </p>
          {!readOnly && !changing && (
            <BrowserAction
              label="Change answer"
              type="button"
              variant="text"
              disabled={disabled}
              onActivate={(event) => {
                if (event.isTrusted) {
                  setChanging(true);
                  queueMicrotask(() =>
                    section.current?.querySelector<HTMLInputElement>('input,textarea')?.focus(),
                  );
                }
              }}
            />
          )}
        </>
      )}
      {readOnly && compose && body && <ArchivedDraft value={body} />}
      {!readOnly && compose && (
        <>
          {(question.kind === 'single' || question.kind === 'multiple') && (
            <fieldset disabled={disabled}>
              <legend>{question.kind === 'single' ? 'Choose one' : 'Choose any that apply'}</legend>
              {question.choices.map((choice) => (
                <label key={choice}>
                  <input
                    type={question.kind === 'single' ? 'radio' : 'checkbox'}
                    name={heading}
                    checked={choices.includes(choice)}
                    onChange={(event) => {
                      if (question.kind === 'single') {
                        setChoices([choice]);
                        setOther(false);
                      } else
                        setChoices((previous) =>
                          event.target.checked
                            ? [...previous, choice]
                            : previous.filter((item) => item !== choice),
                        );
                    }}
                  />
                  {choice}
                </label>
              ))}
              {question.other && (
                <label>
                  <input
                    type={question.kind === 'single' ? 'radio' : 'checkbox'}
                    name={heading}
                    checked={other}
                    onChange={(event) => {
                      setOther(event.target.checked);
                      if (question.kind === 'single') setChoices([]);
                    }}
                  />
                  Other
                </label>
              )}
            </fieldset>
          )}
          {(question.kind === 'short' || question.kind === 'long' || other) && (
            <div className="question-text">
              <label htmlFor={`${heading}-answer`}>{other ? 'Other answer' : 'Your answer'}</label>
              {question.kind === 'short' ? (
                <input
                  id={`${heading}-answer`}
                  value={value}
                  disabled={disabled}
                  onChange={(event) => setValue(event.target.value)}
                  maxLength={QUESTION_SHORT_POINTS * 2}
                />
              ) : (
                <textarea
                  id={`${heading}-answer`}
                  value={value}
                  disabled={disabled}
                  onChange={(event) => setValue(event.target.value)}
                  maxLength={QUESTION_LONG_BYTES}
                  rows={question.kind === 'long' ? 4 : 2}
                />
              )}
            </div>
          )}
          <MessageComposer
            edit={recipient}
            onChange={(edit) => {
              editedRecipient.current = true;
              setRecipient(edit);
            }}
            label="Answer recipient"
            placeholder="Choose an @agent, or leave blank for the thread only"
            candidates={agents}
            disabled={disabled}
          />
          {!audience ||
          directory.state !== 'ready' ||
          decision.ambiguous.length ||
          decision.unavailable.length ||
          decision.unknown.length ? (
            <p className="annotation-hint" role="status">
              {directory.state === 'failed'
                ? text.messageAgentsUnavailable
                : directory.state === 'loading' && ask
                  ? text.messageAgentsChecking
                  : decision.ambiguous.length
                    ? text.messageAmbiguous(decision.ambiguous[0])
                    : decision.unavailable.length
                      ? text.messageRecipientUnavailable(decision.unavailable[0])
                      : decision.unknown.length
                        ? text.messageUnknown(decision.unknown[0], audience)
                        : audience
                          ? text.messageAsks(audience)
                          : text.messageComment}
            </p>
          ) : null}
          {directory.state === 'failed' && (
            <BrowserAction
              label={text.messageAgentsRetry}
              variant="text"
              type="button"
              disabled={busy}
              onActivate={(event) => {
                if (event.isTrusted) retry();
              }}
            />
          )}
          <BrowserAction
            label={`Send answer · to ${audience || 'thread only'}`}
            type="button"
            variant="primary"
            busy={busy}
            disabled={disabled || !selector || !body || !decision.allowed}
            onActivate={(event) => {
              if (event.isTrusted) void send();
            }}
          />
        </>
      )}
      {recorded && <p role="status">Answer recorded; waiting for the verified view.</p>}
      {error && <p role="alert">{error}</p>}
      {!readOnly && (
        <CommentSendFailures
          failures={failures.filter(
            (failure) =>
              !asks.some(
                (record) =>
                  record.messageIds?.includes(failure.message) &&
                  record.agent === failure.recipient.agent &&
                  record.machine === failure.recipient.machine,
              ),
          )}
          binding={ask}
          blocked={blocked || busy}
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
      )}
      {threads.some((thread) => thread.comments.length) && (
        <details className="question-history">
          <summary>
            Conversation
            {asks.some(
              (record) =>
                record.reply && threads.some((thread) => thread.threadId === record.thread),
            )
              ? ' · Agent replied'
              : ''}
          </summary>
          {threads.map((thread) => (
            <div key={`${thread.ref.writer}:${thread.ref.id}`} className="question-history">
              {thread.comments.map((comment) => (
                <CommentExchange
                  readOnly={readOnly}
                  key={`${comment.ref.writer}:${comment.ref.id}`}
                  comment={comment}
                  thread={thread}
                  binding={discussion}
                  ask={ask}
                  asks={asks}
                  blocked={blocked}
                  title={title}
                />
              ))}
            </div>
          ))}
        </details>
      )}
    </section>
  );
}
