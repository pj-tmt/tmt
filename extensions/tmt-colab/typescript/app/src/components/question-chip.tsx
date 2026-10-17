import { forwardRef } from 'react';
import './question-chip.css';

/** The compact form of an answer block: one parent-drawn button inside the running text. `count`
 * is always the admitted group's question count, never author text. */
export const QuestionChip = forwardRef<
  HTMLButtonElement,
  {
    answered: boolean;
    count: number;
    open: boolean;
    panelId: string;
    onToggle(): void;
  }
>(function QuestionChip({ answered, count, open, panelId, onToggle }, ref) {
  return (
    <button
      ref={ref}
      type="button"
      className="question-chip"
      data-state={answered ? 'done' : 'needs'}
      aria-expanded={open}
      aria-controls={panelId}
      onClick={(event) => {
        if (event.isTrusted) onToggle();
      }}
    >
      <span className="question-chip-mark" aria-hidden="true">
        {answered ? '✓' : '◆'}
      </span>{' '}
      {answered ? 'Answered' : 'Please answer'} {count} {count === 1 ? 'question' : 'questions'}{' '}
      <span aria-hidden="true">{open ? '▴' : '▾'}</span>
    </button>
  );
});
