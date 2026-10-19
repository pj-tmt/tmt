import { ChevronDown, ChevronRight } from 'lucide-react';
import { forwardRef } from 'react';
import { answerOf, questionText, type Question } from '../questions.js';
import type { CommentView } from '../thread-records.js';
import './question-line.css';

/** Viewer-only disclosure of an admitted question; opening never records an answer. */
export const QuestionLine = forwardRef<
  HTMLButtonElement,
  {
    question: Question;
    answer?: CommentView;
    expanded: boolean;
    panelId: string;
    onToggle(): void;
    onMove?(direction: -1 | 1): void;
  }
>(function QuestionLine({ question, answer, expanded, panelId, onToggle, onMove }, ref) {
  const state = answer
    ? `Answered: ${questionText(answerOf(question, answer.body) ?? '')}`
    : 'Waiting for you';
  return (
    <button
      type="button"
      ref={ref}
      className="question-line"
      data-state={answer ? 'answered' : 'waiting'}
      aria-label={`${question.title}, ${state}`}
      aria-expanded={expanded}
      aria-controls={panelId}
      onClick={(event) => {
        if (event.isTrusted) onToggle();
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
        {answer ? '✓' : '◆'}
      </span>
      <span className="question-line-title">{question.title}</span>
      <span className="question-line-state">{state}</span>
      <span className="question-line-chevron" aria-hidden="true">
        {expanded ? <ChevronDown size={16} /> : <ChevronRight size={16} />}
      </span>
    </button>
  );
});
