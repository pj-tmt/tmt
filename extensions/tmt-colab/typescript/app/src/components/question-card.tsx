import type { ComposerConnectionPresentation } from '../router.js';
import { BrowserAction, useBusyMark } from '@tmt/browser-ui/react';
import { LoaderCircle } from 'lucide-react';
import { useId, useRef, type ReactNode } from 'react';
import {
  answerOf,
  canonicalQuestionThread,
  currentAnswer,
  QUESTION_SHORT_POINTS,
  QUESTION_LONG_BYTES,
  type Question,
} from '../questions.js';
import { CommentExchange } from '../thread-panel.js';
import type { PageAsk, AskBinding, AskRetryInput } from '../ask-panel.js';
import type { ThreadBinding } from '../thread-store.js';
import type { ThreadView } from '../thread-records.js';
import { ArchivedDraft } from '../annotation-input.js';
import { useNotificationBlock } from '../use-notification-block.js';
import { answerNotificationDestination } from '../reply-notification-context.js';

export interface QuestionDraft {
  choices: string[];
  other: boolean;
  value: string;
  changing: boolean;
}

/** Controlled answer/conversation presentation. The admitted group owns drafts and all effects. */
export function QuestionCard({
  question,
  draft,
  onDraft,
  threads,
  discussion,
  ask,
  asks,
  title,
  blocked,
  connectionPresentation,
  readOnly = false,
  busy = false,
  recorded = false,
  error,
  body,
  sharedAsk,
  replyLabel,
  onOpenReply,
  onSend,
  onRetry,
  retryInput,
}: {
  question: Question;
  draft: QuestionDraft;
  onDraft(patch: Partial<QuestionDraft>): void;
  threads: readonly ThreadView[];
  discussion?: ThreadBinding;
  ask?: AskBinding;
  asks: readonly PageAsk[];
  title: string;
  blocked: boolean;
  connectionPresentation?: ComposerConnectionPresentation;
  readOnly?: boolean;
  busy?: boolean;
  recorded?: boolean;
  error?: string;
  body?: string;
  sharedAsk?: ReactNode;
  replyLabel?: string;
  onOpenReply?(): void;
  onSend?(): void;
  /** Sends the kept draft again after a failed send. */
  onRetry?(): void;
  retryInput?: AskRetryInput;
}) {
  const heading = useId();
  const { choices, other, value, changing } = draft;
  const section = useRef<HTMLElement>(null);
  useNotificationBlock(answerNotificationDestination(question.id), section, true);
  const thread = canonicalQuestionThread(question, threads);
  const answer = currentAnswer(question, thread?.comments ?? []);
  const answerText = answer?.body.slice(`Answer to “${question.title}”: `.length);
  const pageLine = answerText?.lastIndexOf('\nPage: ') ?? -1;
  const disabled = blocked || busy || recorded || !discussion;
  // From Send until the verified record arrives, the card shows the answer being sent where the
  // verified one will stand. It is display only: nothing here is a record, and a failure drops it.
  const echoing = !readOnly && (busy || recorded) && body !== undefined;
  const echo = echoing ? answerOf(question, body) : undefined;
  const pendingMark = useBusyMark(echoing);
  const compose = !echoing && (!answer || changing);
  const changeAction =
    !readOnly && (echoing || (answer && !changing)) ? (
      <BrowserAction
        label="Change answer"
        type="button"
        variant="text"
        disabled={disabled}
        onActivate={(event) => {
          if (event.isTrusted) {
            onDraft({ changing: true });
            queueMicrotask(() =>
              section.current?.querySelector<HTMLInputElement>('input,textarea')?.focus(),
            );
          }
        }}
      />
    ) : undefined;
  return (
    <section
      className="question-card"
      aria-label={question.title}
      ref={section}
      tabIndex={-1}
      onKeyDown={(event) => {
        if (
          !event.isTrusted ||
          event.defaultPrevented ||
          event.altKey ||
          event.ctrlKey ||
          event.metaKey ||
          event.shiftKey
        )
          return;
        const target = event.target as HTMLElement;
        if (
          onSend &&
          !event.nativeEvent.isComposing &&
          event.key === 'Enter' &&
          question.kind === 'short' &&
          target.id === `${heading}-answer`
        ) {
          event.preventDefault();
          onSend();
        }
      }}
    >
      <div className="question-controls">
        {(answer || echoing) && (
          <div className="question-answer" data-echo={echoing || undefined}>
            <p>{echo ?? (pageLine >= 0 ? answerText?.slice(0, pageLine) : answerText)}</p>
            {changeAction}
            <span className="question-pending" aria-hidden="true">
              {echoing && (
                // The same wordless mark as a sent message's echo, on the shared busy-mark delay.
                <span className="comment-echo-mark" data-shown={pendingMark || undefined}>
                  <LoaderCircle />
                </span>
              )}
            </span>
          </div>
        )}
        {readOnly && compose && body && <ArchivedDraft value={body} />}
        {!readOnly && compose && (
          <>
            {(question.kind === 'single' || question.kind === 'multiple') && (
              <fieldset disabled={disabled}>
                <legend>
                  {question.kind === 'single' ? 'Choose one' : 'Choose any that apply'}
                </legend>
                {question.choices.map((choice, index) => (
                  <label
                    key={choice}
                    className="question-option"
                    data-selected={choices.includes(choice)}
                    data-choice-index={index + 1}
                  >
                    <span className="question-option-key" aria-hidden="true">
                      {index + 1}
                    </span>
                    <input
                      type={question.kind === 'single' ? 'radio' : 'checkbox'}
                      name={heading}
                      aria-label={choice}
                      checked={choices.includes(choice)}
                      onChange={(event) => {
                        if (question.kind === 'single') {
                          onDraft({ choices: [choice], other: false });
                        } else
                          onDraft({
                            choices: event.target.checked
                              ? [...choices, choice]
                              : choices.filter((item) => item !== choice),
                          });
                      }}
                    />
                    {choice}
                  </label>
                ))}
                {question.other && (
                  <label
                    className="question-option"
                    data-selected={other}
                    data-choice-index={question.choices.length + 1}
                  >
                    <span className="question-option-key" aria-hidden="true">
                      {question.choices.length + 1}
                    </span>
                    <input
                      type={question.kind === 'single' ? 'radio' : 'checkbox'}
                      name={heading}
                      aria-label="Other"
                      checked={other}
                      onChange={(event) => {
                        onDraft({
                          other: event.target.checked,
                          ...(question.kind === 'single' ? { choices: [] } : {}),
                        });
                      }}
                    />
                    Other
                  </label>
                )}
              </fieldset>
            )}
            {(question.kind === 'short' || question.kind === 'long' || other) && (
              <div className="question-text">
                <label htmlFor={`${heading}-answer`}>
                  {other ? 'Other answer' : 'Your answer'}
                </label>
                {question.kind === 'short' ? (
                  <input
                    id={`${heading}-answer`}
                    value={value}
                    disabled={disabled}
                    onChange={(event) => onDraft({ value: event.target.value })}
                    maxLength={QUESTION_SHORT_POINTS * 2}
                  />
                ) : (
                  <textarea
                    id={`${heading}-answer`}
                    value={value}
                    disabled={disabled}
                    onChange={(event) => onDraft({ value: event.target.value })}
                    maxLength={QUESTION_LONG_BYTES}
                    rows={question.kind === 'long' ? 4 : 2}
                  />
                )}
              </div>
            )}
          </>
        )}
        {!readOnly && error && (!connectionPresentation || recorded) && (
          // Only a failure takes a line: the alert and its Retry stand on one row.
          <div className="question-status">
            <p role="alert">{error}</p>
            {onRetry && (
              <BrowserAction
                label="Retry"
                type="button"
                variant="text"
                disabled={disabled}
                onActivate={(event) => {
                  if (event.isTrusted) onRetry();
                }}
              />
            )}
          </div>
        )}
        {sharedAsk && <p className="question-shared-ask">{sharedAsk}</p>}
        {onOpenReply && (
          <BrowserAction
            label={`Open reply · ${replyLabel}`}
            visibleLabel="Open reply"
            type="button"
            variant="text"
            onActivate={(event) => {
              if (event.isTrusted) onOpenReply();
            }}
          />
        )}
        {threads.some((thread) => thread.comments.length) && (
          <div className="question-history">
            <h3>
              Conversation
              {asks.some(
                (record) =>
                  record.reply && threads.some((thread) => thread.threadId === record.thread),
              )
                ? ' · Agent replied'
                : ''}
            </h3>
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
                    retryInput={retryInput}
                  />
                ))}
              </div>
            ))}
          </div>
        )}
      </div>
    </section>
  );
}
