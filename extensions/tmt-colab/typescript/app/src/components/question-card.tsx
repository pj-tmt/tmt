import type { ComposerConnectionPresentation } from '../router.js';
import { BrowserAction } from '@tmt/browser-ui/react';
import { useEffect, useId, useRef, useState } from 'react';
import {
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
import { compactRelativeTime } from '../display-time.js';
import { NeedsYouBar } from './needs-you-bar.js';

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
  descriptionHeight = 0,
  busy = false,
  recorded = false,
  error,
  body,
  sharedAsk,
  replyLabel,
  onOpenReply,
  onSend,
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
  descriptionHeight?: number;
  busy?: boolean;
  recorded?: boolean;
  error?: string;
  body?: string;
  sharedAsk?: string;
  replyLabel?: string;
  onOpenReply?(): void;
  onSend?(): void;
  retryInput?: AskRetryInput;
}) {
  const heading = useId();
  const { choices, other, value, changing } = draft;
  const section = useRef<HTMLElement>(null);
  useNotificationBlock(answerNotificationDestination(question.id), section, true);
  const thread = canonicalQuestionThread(question, threads);
  const answer = currentAnswer(question, thread?.comments ?? []);
  const [now, setNow] = useState(0);
  useEffect(() => setNow(Date.now()), [answer]);
  const answerText = answer?.body.slice(`Answer to “${question.title}”: `.length);
  const pageLine = answerText?.lastIndexOf('\nPage: ') ?? -1;
  const disabled = blocked || busy || recorded || !discussion;
  const compose = !answer || changing;
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
      <NeedsYouBar
        role="status"
        tone={answer ? 'done' : 'needs'}
        word={answer ? 'Answered' : 'Please answer'}
        who={answer ? answer.deviceName : question.agent}
        why={
          answer
            ? Number(answer.at) > 0 && Number.isFinite(Number(answer.at))
              ? compactRelativeTime(Number(answer.at), now)
              : 'time unknown'
            : undefined
        }
        detail={
          answer ? (
            <span className="question-answer">
              {pageLine >= 0 ? answerText?.slice(0, pageLine) : answerText}
            </span>
          ) : undefined
        }
        actions={
          answer && !readOnly && !changing ? (
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
          ) : undefined
        }
      />
      <div
        className="question-description-space"
        style={{ height: descriptionHeight }}
        aria-hidden="true"
      />
      <div className="question-controls">
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
        {busy && <p role="status">Sending…</p>}
        {!busy && recorded && <p role="status">Answer recorded; waiting for the verified view.</p>}
        {error && (!connectionPresentation || recorded) && <p role="alert">{error}</p>}
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
