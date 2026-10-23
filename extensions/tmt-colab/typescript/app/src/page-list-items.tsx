import type { ReactNode } from 'react';
import { FileText, Lock, Pin } from 'lucide-react';
import { HomeTitleLink, type HomeTitleHints } from './home-titles.js';
import { pageStage } from './list-stage.js';
import { pageUpdate } from './page-index.js';
import { PageWork } from './page-work.js';
import type { PageSummary } from './transport.js';
import { text } from './strings.js';

const t = text.pageList;
function Stage({ page }: { page: PageSummary }) {
  if (page.archived)
    return (
      <span className="page-stage" data-page-stage="archived">
        {text.archived}
      </span>
    );
  const stage = pageStage(page);
  if (!stage) return null;
  return (
    <span className="page-stage" data-page-stage={stage}>
      {t.stageNames[stage]}
    </span>
  );
}

/** Read-only shortcuts to the owner's actual pins, kept in the supplied pin order. */
export function PinnedPages({
  pages,
  hints,
  scope,
}: {
  pages: readonly PageSummary[];
  hints?: HomeTitleHints;
  scope?: string;
}) {
  if (!pages.length) return null;
  return (
    <ul className="page-pins" aria-label={t.pinnedPages}>
      {pages.map((page) => (
        <li key={page.id}>
          <Pin className="page-pin-mark" size={18} aria-hidden="true" />
          <FileText size={28} aria-hidden="true" />
          <div>
            <HomeTitleLink page={page} hints={hints} scope={scope} />
            <div className="page-pin-meta">
              {(page.projects ?? []).map((name) => (
                <span key={name}>{name}</span>
              ))}
              <Stage page={page} />
            </div>
          </div>
        </li>
      ))}
    </ul>
  );
}

/** List presentation only. Home retains selection, filtering and every write's lifetime. */
export function PageListItem({
  page,
  hints,
  scope,
  now,
  leading,
  projects,
  actions,
  selected,
}: {
  page: PageSummary;
  hints?: HomeTitleHints;
  scope?: string;
  now: number;
  leading?: ReactNode;
  projects: ReactNode;
  actions?: ReactNode;
  selected: boolean;
}) {
  const updated = pageUpdate(page.lastUpdateAtMs, now);
  return (
    <li className="page-row" data-page-id={page.id} data-selected={selected ? '' : undefined}>
      <span className="page-row-leading">{leading}</span>
      <div className="page-row-title tmt-ui-list-title">
        <span className="page-row-icon" aria-hidden="true">
          {page.pinned && !page.archived ? (
            <Pin className="page-pin-mark" size={20} />
          ) : (
            <FileText size={22} />
          )}
        </span>
        <HomeTitleLink page={page} hints={hints} scope={scope} />
        {page.summary?.work ? <PageWork work={page.summary.work} /> : null}
      </div>
      <div className="page-row-projects">{projects}</div>
      <div className="page-row-stage">
        {page.summary?.waiting ? (
          <span className="page-waiting" data-waiting>
            <span className="page-waiting-mark" aria-hidden="true">
              ◆
            </span>{' '}
            {t.waitingForYou}
            {page.summary.needsYou > 0 ? ` · ${page.summary.needsYou}` : ''}
          </span>
        ) : (
          <Stage page={page} />
        )}
      </div>
      <span className="page-metadata">
        {page.summary?.author ? <span className="page-author">{page.summary.author}</span> : null}
        {updated.dateTime ? (
          <time
            className="page-updated"
            dateTime={updated.dateTime}
            title={updated.absolute}
            aria-label={`${text.lastUpdated}: ${updated.label}`}
          >
            {updated.label}
          </time>
        ) : null}
        {page.summary?.comments !== undefined ? (
          <span className="page-comments">{t.commentCount(page.summary.comments)}</span>
        ) : null}
      </span>
      <span className="page-row-access">
        {page.sharing === 'private' ? <Lock size={16} aria-hidden="true" /> : null}
        {page.sharing === 'private' ? text.private : text.shared}
      </span>
      {actions ? <span className="tmt-ui-list-actions">{actions}</span> : null}
    </li>
  );
}
