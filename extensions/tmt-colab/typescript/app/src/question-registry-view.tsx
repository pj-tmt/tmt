import { useMemo, type ReactNode } from 'react';
import { inspectQuestions } from './questions.js';
import { verifiedQuestionStates, type QuestionState } from './question-registry.js';
import type { PageView } from './transport.js';
import { text } from './strings.js';
import './question-registry-view.css';
const t = text.metadata;

/** Shared registry presentation. Callers supply controls through their admitted write boundary. */
export function QuestionRegistryView({
  view,
  renderAction,
}: {
  view: PageView;
  renderAction?(question: QuestionState): ReactNode;
}) {
  const states = verifiedQuestionStates(view);
  const sourceIds = useMemo(
    () => new Set(inspectQuestions(view.source).questions.map((question) => question.id)),
    [view.source],
  );
  if (view.questions === undefined) return <p>{t.noRegistry}</p>;
  if (states === undefined) return <p role="status">{t.verifying}</p>;
  if (states.length === 0) return <p>{t.empty}</p>;
  const count = (state: QuestionState['state']) =>
    states.filter((question) => question.state === state).length;
  return (
    <>
      <p className="metadata-question-summary">
        {t.summary(count('open'), count('answered'), count('handled'))}
      </p>
      <ul className="metadata-questions">
        {states.map((question) => {
          const assignee = question.agent?.name ?? view.creator?.agent.name;
          return (
            <li key={question.id}>
              <div className="metadata-question-title">
                <strong>{question.title ?? question.id}</strong>
                <span className="metadata-question-state" data-state={question.state}>
                  {t.status[question.state]}
                </span>
              </div>
              <p>{assignee ? t.assignedTo(assignee) : t.onPage}</p>
              {!sourceIds.has(question.id) && <p>{t.removed}</p>}
              {question.answer !== undefined && <p>{question.answer}</p>}
              {renderAction && (
                <div className="metadata-question-action">{renderAction(question)}</div>
              )}
            </li>
          );
        })}
      </ul>
    </>
  );
}
