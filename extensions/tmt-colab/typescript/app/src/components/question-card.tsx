import type { ComposerConnectionPresentation } from '../router.js';
import { BrowserAction, BrowserChoice, BrowserChoiceGroup } from '@tmt/browser-ui/react';
import { browserUiClasses as c } from '@tmt/browser-ui/static';
import { useEffect, useId, useRef, type ReactNode } from 'react';
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
import { attachmentInputHandlers } from '../attachment-drop.js';
import type { AttachmentDraft } from '../attachment-draft.js';
import { AttachButton, PendingAttachments } from '../attachment-tray.js';
import { MessageAttachments } from '../message-attachments.js';
import { text } from '../strings.js';
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
  files,
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
  /** The answer's attachment draft, owned by the group like the text draft. Absent where this
   * binding cannot upload files. */
  files?: AttachmentDraft;
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
  const attachable = !!files && !disabled;
  const compose = !answer || changing;
  // A failed send re-opens the card on its error row; the reader must act on it, so Retry has focus.
  const retryable = !!error && !!onRetry;
  useEffect(() => {
    if (!retryable || section.current?.closest('[hidden]')) return;
    const active = document.activeElement;
    if (active?.closest('input,textarea,[contenteditable]') && !section.current?.contains(active))
      return;
    section.current?.querySelector<HTMLElement>('.question-status button')?.focus();
  }, [retryable]);
  const changeAction =
    !readOnly && answer && !changing ? (
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
        {answer && (
          <div className="question-answer">
            <p>{pageLine >= 0 ? answerText?.slice(0, pageLine) : answerText}</p>
            <MessageAttachments comment={answer} binding={discussion?.attachments} />
            {changeAction}
          </div>
        )}
        {readOnly && compose && body && <ArchivedDraft value={body} />}
        {!readOnly && compose && (
          <>
            <p className="question-kind">{text.questionKind(question.kind, question.other)}</p>
            {(question.kind === 'single' || question.kind === 'multiple') && (
              <BrowserChoiceGroup
                layout="inline"
                legend={text.questionInstruction(question.kind, question.other)}
              >
                {question.choices.map((choice, index) => (
                  <BrowserChoice
                    key={choice}
                    type={question.kind === 'single' ? 'radio' : 'checkbox'}
                    name={heading}
                    label={choice}
                    data-choice-index={index + 1}
                    aria-keyshortcuts={String(index + 1)}
                    title={text.questionKey(index + 1)}
                    disabled={disabled}
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
                ))}
                {question.other && (
                  <BrowserChoice
                    type={question.kind === 'single' ? 'radio' : 'checkbox'}
                    name={heading}
                    label="Other"
                    data-choice-index={question.choices.length + 1}
                    aria-keyshortcuts={String(question.choices.length + 1)}
                    title={text.questionKey(question.choices.length + 1)}
                    disabled={disabled}
                    checked={other}
                    onChange={(event) => {
                      onDraft({
                        other: event.target.checked,
                        ...(question.kind === 'single' ? { choices: [] } : {}),
                      });
                    }}
                  />
                )}
              </BrowserChoiceGroup>
            )}
            {(question.kind === 'short' ||
              question.kind === 'long' ||
              (question.other && other)) && (
              // The answer field: the whole answer for a short or long question, and under the
              // choices once Other is picked.
              <div
                className="question-text"
                {...(files ? attachmentInputHandlers(files, attachable, false) : {})}
              >
                <label htmlFor={`${heading}-answer`}>
                  {question.kind === 'short' || question.kind === 'long'
                    ? 'Your answer'
                    : text.questionOtherLabel}
                </label>
                {question.kind === 'short' ? (
                  <input
                    id={`${heading}-answer`}
                    className={c.fieldControl}
                    value={value}
                    disabled={disabled}
                    onChange={(event) => onDraft({ value: event.target.value })}
                    maxLength={QUESTION_SHORT_POINTS * 2}
                  />
                ) : (
                  <textarea
                    id={`${heading}-answer`}
                    className={c.fieldControl}
                    placeholder={
                      question.kind === 'long' ? undefined : text.questionOtherPlaceholder
                    }
                    value={value}
                    disabled={disabled}
                    onChange={(event) => onDraft({ value: event.target.value })}
                    maxLength={QUESTION_LONG_BYTES}
                    rows={question.kind === 'long' ? 4 : 1}
                  />
                )}
                {files && (
                  <>
                    <PendingAttachments
                      draft={files}
                      disabled={!attachable}
                      onRemoved={() => {
                        section.current
                          ?.querySelector<HTMLElement>(`[id="${heading}-answer"]`)
                          ?.focus({ preventScroll: true });
                      }}
                    />
                    <div className="question-attach">
                      <AttachButton
                        draft={files}
                        disabled={!attachable}
                        labeled
                        label={text.messageAttach}
                      />
                    </div>
                  </>
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
                    // The answer's files show once, in the answer block above.
                    attachments={comment.messageId !== answer?.messageId}
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
