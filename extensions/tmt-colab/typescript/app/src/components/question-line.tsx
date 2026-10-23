import { BusyMark, useBusyMark } from '@tmt/browser-ui/react';
import { Check, ChevronDown, ChevronUp } from 'lucide-react';
import { forwardRef } from 'react';
import { questionText, type Question } from '../questions.js';
import { text } from '../strings.js';
import './question-line.css';

/** Viewer-only disclosure of an admitted question; opening never records an answer. */
export const QuestionLine = forwardRef<
  HTMLButtonElement,
  {
    question: Question;
    /** The answer the line shows: the verified one, or the one being sent while `pending`. */
    answer?: string;
    /** The answer is sent but no verified record backs it yet: the line is folded and inert. */
    pending?: boolean;
    /** Names the asking agent above the title; a group's heading already names it. */
    eyebrow?: boolean;
    expanded: boolean;
    panelId: string;
    onToggle(): void;
    onMove?(direction: -1 | 1): void;
  }
>(function QuestionLine(
  { question, answer, pending = false, eyebrow = true, expanded, panelId, onToggle, onMove },
  ref,
) {
  const summary = answer === undefined ? undefined : questionText(answer);
  const state = summary === undefined ? text.questionLine.waiting : text.questionLine.answered;
  // The shared busy-mark delay: an answer verified within it never shows the mark.
  const marked = useBusyMark(pending);
  return (
    <button
      type="button"
      ref={ref}
      className="question-line"
      data-state={summary !== undefined ? 'answered' : 'waiting'}
      aria-label={`${question.title}, ${summary === undefined ? state : `${state}: ${summary}`}`}
      aria-expanded={expanded}
      aria-controls={panelId}
      aria-disabled={pending || undefined}
      onClick={(event) => {
        if (event.isTrusted && !pending) onToggle();
      }}
      onKeyDown={(event) => {
        if (
          !event.isTrusted ||
          event.defaultPrevented ||
          event.ctrlKey ||
          event.metaKey ||
          event.altKey ||
          event.shiftKey
        )
          return;
        if (onMove && (event.key === 'ArrowUp' || event.key === 'ArrowDown')) {
          event.preventDefault();
          onMove(event.key === 'ArrowUp' ? -1 : 1);
        }
      }}
    >
      <span className="question-line-icon" aria-hidden="true">
        ?
      </span>
      <span className="question-line-heading">
        {eyebrow && (
          <span className="question-line-eyebrow">{text.questionLine.eyebrow(question.agent)}</span>
        )}
        <span className="question-line-title">{question.title}</span>
      </span>
      <span className="question-line-state">
        <span className="question-line-mark" aria-hidden="true">
          {pending && marked ? (
            <span className="comment-echo-mark" data-shown="true">
              <BusyMark />
            </span>
          ) : summary !== undefined ? (
            <Check size={13} />
          ) : (
            '◆'
          )}
        </span>
        <span>{state}</span>
      </span>
      <span className="question-line-chevron" aria-hidden="true">
        {expanded ? <ChevronUp size={14} /> : <ChevronDown size={14} />}
      </span>
      {summary !== undefined && <span className="question-line-answer">{summary}</span>}
    </button>
  );
});
