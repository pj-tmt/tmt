import { Link } from '@tanstack/react-router';
import { CalendarDays, Clock, TriangleAlert } from 'lucide-react';
import { useRef, type KeyboardEvent, type ReactNode } from 'react';
import { AgentChip } from './components/agent-chip.js';
import { compactRelativeTime } from './display-time.js';
import { useTitleHint, type HomeTitleHints } from './home-titles.js';
import { pageTitle } from './page-index.js';
import { pageLinkKey } from './page-link.js';
import type { Shell } from './shell.js';
import type { ShellCell } from './shell-grid.js';
import { pagesChangedSince, shellByline } from './shell-grid.js';
import type { Chapter, EarlierRow, ShellModel } from './shell-model.js';
import { spellSmallNumber } from './shell-model.js';
import { text } from './strings.js';
import type { PageSummary } from './transport.js';

const t = text.shellPage;

/** The page's own headline over its one-line greeting, which names the local time of day and the
 * number waiting. The greeting names no one. */
export function ShellHead({ title, model }: { title: string; model: ShellModel }) {
  return (
    <header className="tmt-ui-page-head">
      <h1 className="tmt-ui-page-title">{title || text.unknownPageTitle}</h1>
      <p className="tmt-ui-page-subtitle">
        {t.greeting(model.timeOfDay, model.waiting, spellSmallNumber(model.waiting))}
      </p>
    </header>
  );
}

/** The maintainer agent's two lines with who wrote them and when, or that nothing changed. */
export function ShellSummary({
  shell,
  cells,
  now,
}: {
  shell: Shell;
  cells: readonly ShellCell[];
  now: number;
}) {
  const summary = shell.summary;
  const byline = shellByline(shell);
  const changed = byline ? pagesChangedSince(cells, byline.timeMs) : 0;
  return (
    <div className="shell-summary" role="group" aria-label={t.summary}>
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
    </div>
  );
}

type Jump = 'waiting' | 'in-progress' | 'today';

/** The three counts as tiles, each a button that jumps to the first row it counts. A count of zero
 * is plain text: there is nowhere to jump. */
export function ShellCounts({
  model,
  onJump,
}: {
  model: ShellModel;
  onJump: (jump: Jump) => void;
}) {
  const entries: { jump: Jump; value: number; label: string; icon: ReactNode }[] = [
    {
      jump: 'waiting',
      value: model.counts.waiting,
      label: t.waitingCount,
      icon: <TriangleAlert />,
    },
    {
      jump: 'in-progress',
      value: model.counts.inProgress,
      label: t.progressCount,
      icon: <Clock />,
    },
    {
      jump: 'today',
      value: model.counts.updatedToday,
      label: t.todayCount,
      icon: <CalendarDays />,
    },
  ];
  return (
    <nav className="shell-counts" aria-label={t.counts}>
      <ul>
        {entries.map(({ jump, value, label, icon }) => {
          const inner = (
            <>
              <span className="shell-stat-icon" aria-hidden="true">
                {icon}
              </span>
              <span className="shell-stat-text">
                <span className="shell-count-label">{label}</span>
                <span className="shell-count-number">{value}</span>
              </span>
            </>
          );
          return (
            <li key={jump} data-count={jump}>
              {value > 0 ? (
                <button type="button" className="shell-stat" onClick={() => onJump(jump)}>
                  {inner}
                </button>
              ) : (
                <span className="shell-stat" data-plain>
                  {inner}
                </span>
              )}
            </li>
          );
        })}
      </ul>
    </nav>
  );
}

/** One line of the Pages list: a chapter, or an older page that has no passage. */
export interface ShellItem {
  pageId: string;
  number: number;
  title: string;
  author?: string;
  label?: string;
  lastUpdateAtMs: number | null;
  /** Set on a chapter; an older page has no stage and no passage. */
  chapter?: Chapter;
}

export function shellItems(model: ShellModel): { chapters: ShellItem[]; earlier: ShellItem[] } {
  const chapters = model.chapters.map<ShellItem>((chapter) => ({
    pageId: chapter.pageId,
    number: chapter.number,
    title: chapter.title,
    ...(chapter.author === undefined ? {} : { author: chapter.author }),
    ...(chapter.label === undefined ? {} : { label: chapter.label }),
    lastUpdateAtMs: chapter.lastUpdateAtMs,
    chapter,
  }));
  const earlier = model.earlier.map<ShellItem>((row: EarlierRow, index) => ({
    pageId: row.pageId,
    number: chapters.length + index + 1,
    title: row.title,
    ...(row.author === undefined ? {} : { author: row.author }),
    lastUpdateAtMs: row.lastUpdateAtMs,
  }));
  return { chapters, earlier };
}

const two = (n: number) => String(n).padStart(2, '0');

/** The state of a chapter's page, drawn as the shared state chip. */
function StagePill({ stage }: { stage: Chapter['stage'] }) {
  const waiting = stage === 'waiting';
  return (
    <span className="tmt-ui-chip shell-row-status" data-tone={waiting ? 'waiting' : 'working'}>
      <span className="tmt-ui-state-mark" data-shape="dot" aria-hidden="true" />
      <span className="tmt-ui-chip-label">{waiting ? t.waitingCount : t.progressCount}</span>
    </span>
  );
}

function ShellRow({
  item,
  page,
  hints,
  now,
  today,
  selected,
  onSelect,
  onKeyDown,
}: {
  item: ShellItem;
  page: PageSummary | undefined;
  hints: HomeTitleHints | undefined;
  now: number;
  today: number;
  selected: boolean;
  onSelect: (pageId: string) => void;
  onKeyDown: (event: KeyboardEvent<HTMLButtonElement>) => void;
}) {
  const button = useRef<HTMLButtonElement>(null);
  useTitleHint(page ?? { id: item.pageId, titleMissing: false }, hints, button);
  const title = page ? pageTitle(page) : item.title || text.unknownPageTitle;
  const stage = item.chapter?.stage;
  return (
    <li
      className="shell-row-item"
      id={item.chapter ? `shell-chapter-${item.number}` : undefined}
      data-page-id={item.pageId}
      data-stage={stage}
      data-today={(item.lastUpdateAtMs ?? -1) >= today || undefined}
    >
      <button
        ref={button}
        type="button"
        className="shell-row"
        aria-current={selected ? 'true' : undefined}
        aria-controls="shell-aside"
        onClick={() => onSelect(item.pageId)}
        onKeyDown={onKeyDown}
      >
        <span className="shell-row-number">{two(item.number)}</span>
        <span className="shell-row-text">
          <span className="shell-row-title">{title}</span>
          <span className="shell-row-meta">
            {item.author && <AgentChip name={item.author} />}
            {item.lastUpdateAtMs !== null && (
              <time dateTime={new Date(item.lastUpdateAtMs).toISOString()}>
                {compactRelativeTime(item.lastUpdateAtMs, now)}
              </time>
            )}
            {item.label && <span className="shell-row-label">{item.label}</span>}
          </span>
        </span>
        {stage && <StagePill stage={stage} />}
      </button>
    </li>
  );
}

/** The Pages list: the chapters in the shell's order, then the older pages under their own
 * heading. Rows only select; the page opens from the selected page's summary. */
export function ShellPages({
  items,
  pages,
  hints,
  now,
  today,
  selected,
  onSelect,
}: {
  items: { chapters: readonly ShellItem[]; earlier: readonly ShellItem[] };
  pages: ReadonlyMap<string, PageSummary>;
  hints: HomeTitleHints | undefined;
  now: number;
  today: number;
  selected: string | undefined;
  onSelect: (pageId: string) => void;
}) {
  const list = useRef<HTMLElement>(null);
  // Arrow keys step between the rows; Enter or Space on a row selects it, as on any button.
  const step = (event: KeyboardEvent<HTMLButtonElement>) => {
    const move = event.key === 'ArrowDown' ? 1 : event.key === 'ArrowUp' ? -1 : 0;
    if (move === 0) return;
    const rows = [...list.current!.querySelectorAll<HTMLButtonElement>('button.shell-row')];
    const next = rows[rows.indexOf(event.currentTarget) + move];
    if (!next) return;
    event.preventDefault();
    next.focus();
  };
  const rows = (group: readonly ShellItem[]) =>
    group.map((item) => (
      <ShellRow
        key={item.pageId}
        item={item}
        page={pages.get(item.pageId)}
        hints={hints}
        now={now}
        today={today}
        selected={item.pageId === selected}
        onSelect={onSelect}
        onKeyDown={step}
      />
    ));
  return (
    <section className="tmt-ui-card shell-pages" data-flush ref={list}>
      <div className="tmt-ui-card-head">
        <h3 className="tmt-ui-card-title">{t.chapters}</h3>
      </div>
      <ol className="shell-rows" aria-label={t.chapters}>
        {rows(items.chapters)}
      </ol>
      {items.earlier.length > 0 && (
        <>
          <h4 className="shell-earlier-heading">{t.earlier(items.earlier.length)}</h4>
          <ol className="shell-rows" start={items.chapters.length + 1}>
            {rows(items.earlier)}
          </ol>
        </>
      )}
    </section>
  );
}

/** Where the passage opens the page: at the reading it names, else at the element an anchor item
 * of the page names, else at the top. */
function pageSearch(chapter: Chapter): { read: string } | { anchor: string } | undefined {
  const passage = chapter.passage;
  if (passage.kind === 'need' && passage.need.kind === 'reading') return { read: passage.need.id };
  return chapter.anchor === undefined ? undefined : { anchor: chapter.anchor };
}

/** What the selected page says: the passage of the page that most needs the owner, as text. Nothing
 * here changes anything; the actions below it open the page. */
function Passage({ chapter, now }: { chapter: Chapter; now: number }) {
  const passage = chapter.passage;
  if (passage.kind === 'none') return null;
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
    </div>
  );
}

/** The selected page beside the list: who and when, its passage, then the ways to open it. The
 * first action is the primary one: deciding when something waits, else the page itself. */
export function ShellAside({
  item,
  page,
  now,
  onOpen,
  onPanel,
}: {
  item: ShellItem;
  page: PageSummary | undefined;
  now: number;
  onOpen: () => void;
  /** Where a wide window opens the chapter's panel; a narrow one has none and goes to the page. */
  onPanel?: (pageId: string) => void;
}) {
  const chapter = item.chapter;
  const search = chapter ? pageSearch(chapter) : undefined;
  const need = chapter?.passage.kind === 'need';
  const pageLink = (label: string, primary: boolean, withSearch: boolean) => (
    <Link
      key={label}
      className="tmt-ui-action shell-open"
      data-variant={primary ? 'primary' : 'text'}
      to="/pages/$pageId"
      params={{ pageId: item.pageId }}
      {...(withSearch && search ? { search } : {})}
      onClick={onOpen}
      onKeyDown={pageLinkKey}
    >
      {label}
    </Link>
  );
  const actions: ReactNode[] = [];
  if (need && onPanel)
    actions.push(
      <button
        key="decide"
        type="button"
        className="tmt-ui-action shell-open"
        data-variant="primary"
        data-opens-panel
        aria-haspopup="dialog"
        onClick={() => onPanel(item.pageId)}
      >
        {t.openToDecide}
      </button>,
    );
  else if (need) actions.push(pageLink(t.openToDecide, true, true));
  else if (search) actions.push(pageLink(t.openPassage, true, true));
  actions.push(pageLink(t.openFullPage, actions.length === 0, false));
  return (
    <aside
      className="tmt-ui-card shell-aside"
      id="shell-aside"
      aria-label={t.selectedPage}
      data-page-id={item.pageId}
    >
      <h3 className="shell-aside-title">
        {page ? pageTitle(page) : item.title || text.unknownPageTitle}
      </h3>
      <p className="shell-aside-meta">
        {item.author && <AgentChip name={item.author} />}
        {item.lastUpdateAtMs !== null && (
          <time dateTime={new Date(item.lastUpdateAtMs).toISOString()}>
            {compactRelativeTime(item.lastUpdateAtMs, now)}
          </time>
        )}
      </p>
      {chapter && <Passage chapter={chapter} now={now} />}
      <div className="shell-aside-actions">{actions}</div>
    </aside>
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
