import { Link } from '@tanstack/react-router';
import type { ReactNode } from 'react';
import { AgentChip } from './components/agent-chip.js';
import { compactRelativeTime } from './display-time.js';
import { HomeTitleLink, type HomeTitleHints } from './home-titles.js';
import { pageLinkKey } from './page-link.js';
import type { Shell } from './shell.js';
import type { ShellCell } from './shell-grid.js';
import { pagesChangedSince, shellByline } from './shell-grid.js';
import type { Chapter, EarlierRow, ShellModel } from './shell-model.js';
import { spellSmallNumber } from './shell-model.js';
import { text } from './strings.js';
import type { PageSummary } from './transport.js';

const t = text.shellPage;

/** What a person sees first: how many things wait for them, then the maintainer agent's two lines
 * with who wrote them and when, or that nothing changed. The greeting names no one. */
export function ShellGreeting({
  model,
  shell,
  cells,
  now,
}: {
  model: ShellModel;
  shell: Shell;
  cells: readonly ShellCell[];
  now: number;
}) {
  const summary = shell.summary;
  const byline = shellByline(shell);
  const changed = byline ? pagesChangedSince(cells, byline.timeMs) : 0;
  return (
    <section className="shell-hero" aria-label={t.summary}>
      <h2 className="shell-greeting">
        {t.greeting(model.timeOfDay, model.waiting, spellSmallNumber(model.waiting))}
      </h2>
      {summary && byline ? (
        <>
          <p className="shell-summary-line1">{summary.line1}</p>
          <p className="shell-summary-line2">{summary.line2}</p>
          <p className="shell-summary-meta">
            {byline.who && (
              <>
                <AgentChip name={byline.who} /> ·{' '}
              </>
            )}
            <time dateTime={new Date(byline.timeMs).toISOString()}>
              {compactRelativeTime(byline.timeMs, now)}
            </time>
            {changed > 0 && <span className="shell-changed"> · {t.changedSince(changed)}</span>}
          </p>
        </>
      ) : (
        <p className="shell-summary-meta">{t.noSummary}</p>
      )}
    </section>
  );
}

type Jump = 'waiting' | 'in-progress' | 'today';

/** The three counts, each a link that jumps to the first row it counts. A count of zero is plain
 * text: there is nowhere to jump. */
export function ShellCounts({
  model,
  onJump,
}: {
  model: ShellModel;
  onJump: (jump: Jump) => void;
}) {
  const entries: { jump: Jump; value: number; label: string; mark?: boolean }[] = [
    { jump: 'waiting', value: model.counts.waiting, label: t.waitingCount, mark: true },
    { jump: 'in-progress', value: model.counts.inProgress, label: t.progressCount },
    { jump: 'today', value: model.counts.updatedToday, label: t.todayCount },
  ];
  return (
    <nav className="shell-counts" aria-label={t.counts}>
      <ul>
        {entries.map(({ jump, value, label, mark }) => {
          const inner = (
            <>
              <span className="shell-count-number">{value}</span>
              <span className="shell-count-label">
                {mark && value > 0 && (
                  <span className="shell-mark" aria-hidden="true">
                    ◆{' '}
                  </span>
                )}
                {label}
              </span>
            </>
          );
          return (
            <li key={jump} data-count={jump}>
              {value > 0 ? (
                <button type="button" onClick={() => onJump(jump)}>
                  {inner}
                </button>
              ) : (
                <span className="shell-count-plain">{inner}</span>
              )}
            </li>
          );
        })}
      </ul>
    </nav>
  );
}

/** Where the passage opens the page: at the reading it names, else at the element an anchor item
 * of the page names, else at the top. */
function pageSearch(chapter: Chapter): { read: string } | { anchor: string } | undefined {
  const passage = chapter.passage;
  if (passage.kind === 'need' && passage.need.kind === 'reading') return { read: passage.need.id };
  return chapter.anchor === undefined ? undefined : { anchor: chapter.anchor };
}

/** The right side of a chapter: the passage of the page that most needs the owner, as text. Opening
 * it to decide goes to the page; nothing here changes anything. */
function ChapterPassage({
  chapter,
  now,
  onOpen,
  onPanel,
}: {
  chapter: Chapter;
  now: number;
  onOpen: () => void;
  /** Where a wide window opens the chapter's panel; a narrow one has none and goes to the page. */
  onPanel?: (pageId: string) => void;
}) {
  const passage = chapter.passage;
  const search = pageSearch(chapter);
  const openPassage = search && (
    <Link
      className="shell-open"
      to="/pages/$pageId"
      params={{ pageId: chapter.pageId }}
      search={search}
      onClick={onOpen}
      onKeyDown={pageLinkKey}
    >
      {t.openPassage}
    </Link>
  );
  if (passage.kind === 'none')
    return (
      <div className="shell-passage" data-kind="none">
        {openPassage}
      </div>
    );
  if (passage.kind === 'pending')
    return (
      <div className="shell-passage" data-kind="pending" aria-busy="true">
        <p className="shell-passage-pending" role="status">
          {t.loadingPassage}
        </p>
      </div>
    );
  if (passage.kind === 'note')
    return (
      <div className="shell-passage" data-kind="note">
        <p className="shell-passage-note">{passage.note}</p>
        {openPassage}
      </div>
    );
  if (passage.kind === 'latest')
    return (
      <div className="shell-passage" data-kind="latest">
        <p className="shell-passage-lead">{t.latest}</p>
        <p className="shell-passage-text">{passage.latest.text}</p>
        <p className="shell-passage-meta">
          {passage.latest.who && <>{passage.latest.who} · </>}
          {compactRelativeTime(passage.latest.atMs, now)}
        </p>
        {openPassage}
      </div>
    );
  const { need, more } = passage;
  return (
    <div className="shell-passage" data-kind="need" data-need={need.kind}>
      <p className="shell-passage-lead">
        <span className="shell-mark" aria-hidden="true">
          ◆
        </span>{' '}
        {t.needKind[need.kind]}
        {need.agent && (
          <>
            {' · '}
            <AgentChip name={need.agent} />
          </>
        )}
      </p>
      <p className="shell-passage-title">{need.title}</p>
      {need.why && <p className="shell-passage-why">{need.why}</p>}
      {more > 0 && <p className="shell-passage-more">{t.moreWaiting(more)}</p>}
      {onPanel ? (
        <button
          type="button"
          className="shell-open"
          data-opens-panel
          aria-haspopup="dialog"
          onClick={() => onPanel(chapter.pageId)}
        >
          {t.openToDecide}
        </button>
      ) : (
        <Link
          className="shell-open"
          to="/pages/$pageId"
          params={{ pageId: chapter.pageId }}
          search={search}
          onClick={onOpen}
          onKeyDown={pageLinkKey}
        >
          {t.openToDecide}
        </Link>
      )}
    </div>
  );
}

const two = (n: number) => String(n).padStart(2, '0');

/** One chapter: the page's large title with who, when and label on the left, its passage on the
 * right. Chapters join by hairlines; there are no cards. */
export function ShellChapter({
  chapter,
  page,
  hints,
  now,
  today,
  onOpen,
  onPanel,
}: {
  chapter: Chapter;
  page: PageSummary | undefined;
  hints: HomeTitleHints | undefined;
  now: number;
  today: number;
  onOpen: () => void;
  onPanel?: (pageId: string) => void;
}) {
  const isToday = (chapter.lastUpdateAtMs ?? -1) >= today;
  return (
    <li
      className="shell-chapter"
      id={`shell-chapter-${chapter.number}`}
      data-page-id={chapter.pageId}
      data-stage={chapter.stage}
      data-today={isToday || undefined}
    >
      <div className="shell-chapter-head">
        <p className="shell-chapter-number">
          {two(chapter.number)} ·{' '}
          {chapter.stage === 'waiting' ? (
            <span className="shell-mark">
              <span aria-hidden="true">◆</span> {t.stage.waiting}
            </span>
          ) : (
            t.stage['in-progress']
          )}
        </p>
        <h3 className="shell-chapter-title">
          {page ? (
            <HomeTitleLink page={page} hints={hints} onClick={onOpen} />
          ) : (
            <Link to="/pages/$pageId" params={{ pageId: chapter.pageId }} onClick={onOpen}>
              {chapter.title || text.unknownPageTitle}
            </Link>
          )}
        </h3>
        <p className="shell-chapter-meta">
          {chapter.author && <AgentChip name={chapter.author} />}
          {chapter.lastUpdateAtMs !== null && (
            <time dateTime={new Date(chapter.lastUpdateAtMs).toISOString()}>
              {compactRelativeTime(chapter.lastUpdateAtMs, now)}
            </time>
          )}
          {chapter.label && <span className="shell-tag">{chapter.label}</span>}
        </p>
        <Link
          className="shell-open"
          to="/pages/$pageId"
          params={{ pageId: chapter.pageId }}
          onClick={onOpen}
          onKeyDown={pageLinkKey}
        >
          {t.openFullPage}
        </Link>
      </div>
      <ChapterPassage chapter={chapter} now={now} onOpen={onOpen} onPanel={onPanel} />
    </li>
  );
}

/** Older pages, one large-type row each. They keep their place in the numbering after the chapters. */
export function ShellEarlier({
  rows,
  start,
  pages,
  hints,
  now,
  today,
  onOpen,
}: {
  rows: readonly EarlierRow[];
  start: number;
  pages: ReadonlyMap<string, PageSummary>;
  hints: HomeTitleHints | undefined;
  now: number;
  today: number;
  onOpen: () => void;
}): ReactNode {
  if (rows.length === 0) return null;
  return (
    <section className="shell-earlier" aria-labelledby="shell-earlier-heading">
      <h3 id="shell-earlier-heading" className="shell-earlier-heading">
        {t.earlier(rows.length)}
      </h3>
      <ol start={start}>
        {rows.map((row) => {
          const page = pages.get(row.pageId);
          return (
            <li
              key={row.pageId}
              className="shell-earlier-row"
              data-page-id={row.pageId}
              data-today={(row.lastUpdateAtMs ?? -1) >= today || undefined}
            >
              <span className="shell-earlier-title">
                {page ? (
                  <HomeTitleLink page={page} hints={hints} onClick={onOpen} />
                ) : (
                  <Link to="/pages/$pageId" params={{ pageId: row.pageId }} onClick={onOpen}>
                    {row.title || text.unknownPageTitle}
                  </Link>
                )}
              </span>
              <span className="shell-earlier-meta">
                {row.author && <>{row.author} · </>}
                {row.lastUpdateAtMs !== null && compactRelativeTime(row.lastUpdateAtMs, now)}
              </span>
            </li>
          );
        })}
      </ol>
    </section>
  );
}

/** References to pages the space does not show: they stay listed and open nothing. */
export function ShellUnavailable({ count }: { count: number }): ReactNode {
  if (count === 0) return null;
  return (
    <p className="shell-state" role="status" data-unavailable={count}>
      {t.cannotShow(count)}
    </p>
  );
}

export type { Jump };
