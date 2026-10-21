import type { PageWorkRow } from './bootstrap.js';
import { AgentChip } from './components/agent-chip.js';
import { text } from './strings.js';
import './page-work.css';

/**
 * A page's tasks in its list row: a bar with the count beside it, then who works on it. The count
 * is words and digits, never colour alone. A page whose tasks were all removed has nothing to show.
 */
export function PageWork({ work }: { work: PageWorkRow }) {
  if (work.total === 0) return null;
  const t = text.pageList;
  return (
    <span className="page-work" data-stage={work.stage}>
      <progress
        className="page-progress"
        value={work.done}
        max={work.total}
        aria-label={t.tasksDone(work.done, work.total)}
      />
      <span className="page-progress-count" aria-hidden="true">
        {work.done}/{work.total}
      </span>
      {work.working.length > 0 && (
        <ul className="page-workers" aria-label={t.working}>
          {work.working.map((who, index) => (
            <li key={`${index}:${who.kind}:${who.name}`}>
              {who.kind === 'agent' ? (
                <AgentChip name={who.name} />
              ) : (
                <span className="page-worker">{who.name}</span>
              )}
            </li>
          ))}
          {work.more ? <li className="page-workers-more">{t.moreWorking(work.more)}</li> : null}
        </ul>
      )}
    </span>
  );
}
