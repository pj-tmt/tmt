import { useBusyMark } from '@tmt/browser-ui/react';
import { browserUiClasses as ui } from '@tmt/browser-ui/static';
import { ChevronDown, ChevronRight, LoaderCircle } from 'lucide-react';
import { forwardRef } from 'react';
import { questionText, type Question } from '../questions.js';
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
    expanded: boolean;
    panelId: string;
    onToggle(): void;
    onMove?(direction: -1 | 1): void;
  }
>(function QuestionLine(
  { question, answer, pending = false, expanded, panelId, onToggle, onMove },
  ref,
) {
  const state = answer !== undefined ? `Answered: ${questionText(answer)}` : 'Waiting for you';
  // The shared busy-mark delay: an answer verified within it never shows the mark.
  const marked = useBusyMark(pending);
  return (
    <button
      type="button"
      ref={ref}
      className="question-line"
      data-state={answer !== undefined ? 'answered' : 'waiting'}
      aria-label={`${question.title}, ${state}`}
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
      <span className="question-line-mark" aria-hidden="true">
        {pending && marked ? (
          <span className="comment-echo-mark" data-shown="true">
            <LoaderCircle className={ui.spin} />
          </span>
        ) : answer !== undefined ? (
          '✓'
        ) : (
          '◆'
        )}
      </span>
      <span className="question-line-title">{question.title}</span>
      <span className="question-line-state">{state}</span>
      <span className="question-line-chevron" aria-hidden="true">
        {expanded ? <ChevronDown size={16} /> : <ChevronRight size={16} />}
      </span>
    </button>
  );
});
