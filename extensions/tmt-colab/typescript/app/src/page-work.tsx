import type { PageWorkRow } from './bootstrap.js';
import { text } from './strings.js';
import './page-work.css';

/** A page's sub-pages in its list row: how many of its direct sub-pages are done, in words. */
export function PageWork({ work }: { work: PageWorkRow }) {
  if (work.total === 0) return null;
  return (
    <span className="page-work" data-stage={work.stage}>
      <span className="page-progress-count">
        {text.pageList.subpagesDone(work.done, work.total)}
      </span>
    </span>
  );
}
