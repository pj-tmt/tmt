import { getRouteApi, Link } from '@tanstack/react-router';
import { useEffect, useRef, useState } from 'react';
import { AgentChip } from './components/agent-chip.js';
import { compactRelativeTime } from './display-time.js';
import { HomeTitleLink } from './home-titles.js';
import { useHomeSpace } from './home-space.js';
import { pageLinkKey } from './page-link.js';
import { pageTitle } from './page-index.js';
import { ReadingDetailsQueue, type ReadingDetails } from './reading-details.js';
import type { Shell } from './shell.js';
import {
  pagesChangedSince,
  readingPageIds,
  readingView,
  shellByline,
  shellCells,
  type ReadingView,
  type ShellCell,
} from './shell-grid.js';
import { text } from './strings.js';
import type { SpaceHome } from './transport.js';
import './shell-page.css';

const rootRoute = getRouteApi('__root__');
const t = text.shellPage;

/** The shell page: the app owns the whole look, the document only supplies the data. Items are
 * links to their source pages and carry no content of their own. A page with no admitted value
 * says so instead of drawing nothing. */
export function ShellPage({ shell }: { shell: Shell | undefined }) {
  return shell ? (
    <ShellContent shell={shell} />
  ) : (
    <div className="shell-page">
      <p className="shell-state" role="status">
        {t.notShown}
      </p>
    </div>
  );
}

function ShellContent({ shell }: { shell: Shell }) {
  const { transport } = rootRoute.useRouteContext();
  const [home, setHome] = useState<SpaceHome | 'failed' | undefined>();
  useEffect(() => {
    let live = true;
    transport.spaceHome().then(
      (value) => live && setHome(value),
      () => live && setHome('failed'),
    );
    return () => {
      live = false;
    };
  }, [transport]);
  return (
    <div className="shell-page">
      <ShellSummary shell={shell} home={typeof home === 'object' ? home : undefined} />
      {home === undefined ? (
        <p className="shell-state" role="status">
          {t.loading}
        </p>
      ) : (
        <ShellGrid
          shell={shell}
          home={home === 'failed' ? { title: '', pages: [] } : home}
          failed={home === 'failed'}
        />
      )}
    </div>
  );
}

function useNow(): number {
  const [now, setNow] = useState(0);
  useEffect(() => setNow(Date.now()), []);
  return now;
}

function ShellSummary({ shell, home }: { shell: Shell; home: SpaceHome | undefined }) {
  const now = useNow();
  const summary = shell.summary;
  if (!summary) return null;
  const byline = shellByline(shell)!;
  const changed = home ? pagesChangedSince(shellCells(shell, home.pages), byline.timeMs) : 0;
  return (
    <section className="shell-summary" aria-label={t.summary}>
      <h2 className="shell-summary-line1">{summary.line1}</h2>
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
    </section>
  );
}

/** The owner's reading details of the pages the shell's reading items point at, read once per
 * opening (a return to the shell reads again, so a `Mark read` on the page shows). Optional and
 * owner-only: a transport without the port, a failed read or a pending page leaves no state. */
function useReadingDetails(pageIds: readonly string[]): ReadonlyMap<string, ReadingDetails> {
  const { transport } = rootRoute.useRouteContext();
  const [details, setDetails] = useState<ReadonlyMap<string, ReadingDetails>>(new Map());
  const key = pageIds.join(' ');
  useEffect(() => {
    const read = transport.readingDetails?.bind(transport);
    if (!read || !key) return;
    const queue = new ReadingDetailsQueue(read, (id, value) =>
      setDetails((previous) => new Map(previous).set(id, value)),
    );
    queue.enqueue(key.split(' '));
    return () => {
      void queue.close();
    };
  }, [transport, key]);
  return details;
}

function ShellGrid({ shell, home, failed }: { shell: Shell; home: SpaceHome; failed: boolean }) {
  const { pages, hints } = useHomeSpace(home);
  const cells = failed
    ? shellCells(shell, []).map((cell) => ({ ...cell, unavailable: false, titleMissing: true }))
    : shellCells(shell, pages);
  const details = useReadingDetails(failed ? [] : readingPageIds(cells));
  const byId = new Map(pages.map((page) => [page.id, page]));
  const list = useRef<HTMLUListElement>(null);
  // Coming back from an item restores focus to it; the browser restores the scroll.
  useEffect(() => {
    const id = (window.history.state as { shellFocus?: string } | null)?.shellFocus;
    if (!id) return;
    const link = list.current?.querySelector<HTMLElement>(`[data-item-id="${id}"] a`);
    link?.focus({ preventScroll: true });
    link?.scrollIntoView({ block: 'nearest' });
  }, []);
  if (cells.length === 0)
    return (
      <p className="shell-state" role="status">
        {t.empty}
      </p>
    );
  return (
    <>
      {failed && (
        <p className="shell-state" role="status">
          {t.pagesUnavailable}
        </p>
      )}
      <ul className="shell-grid" aria-label={t.items} ref={list}>
        {cells.map((cell) => (
          <ShellCellView
            key={cell.id}
            cell={cell}
            page={byId.get(cell.pageId)}
            hints={hints}
            reading={readingView(cell, details.get(cell.pageId))}
          />
        ))}
      </ul>
    </>
  );
}

function ShellCellView({
  cell,
  page,
  hints,
  reading,
}: {
  cell: ShellCell;
  page: ReturnType<typeof useHomeSpace>['pages'][number] | undefined;
  hints: ReturnType<typeof useHomeSpace>['hints'];
  reading: ReadingView | undefined;
}) {
  const remember = () => {
    window.history.replaceState({ ...window.history.state, shellFocus: cell.id }, '');
  };
  const search =
    cell.kind === 'reading' && cell.readingId
      ? { read: cell.readingId }
      : cell.kind === 'anchor' && cell.anchor
        ? { anchor: cell.anchor }
        : undefined;
  return (
    <li
      className="shell-cell"
      data-item-id={cell.id}
      data-kind={cell.kind}
      data-unavailable={cell.unavailable || undefined}
    >
      <p className="shell-cell-kind">
        {t.kind[cell.kind]}
        {cell.kind === 'anchor' && cell.anchor ? ` · ${cell.anchor}` : ''}
        {cell.archived ? ` · ${t.archived}` : ''}
        {reading?.kind === 'listed' && (
          <>
            {' · '}
            <span className="shell-cell-state" data-state={reading.state}>
              <span className="shell-cell-mark" aria-hidden="true">
                {reading.state === 'unread' ? '•' : '✓'}
              </span>{' '}
              <span className="shell-cell-word">
                {reading.state === 'unread' ? t.unread : t.read}
              </span>
            </span>
          </>
        )}
        {reading?.kind === 'missing' && ` · ${t.readingMissing}`}
      </p>
      <p className="shell-cell-title">
        {cell.unavailable ? (
          <span className="shell-cell-unavailable">{t.unavailable}</span>
        ) : search || !page ? (
          <Link
            to="/pages/$pageId"
            params={{ pageId: cell.pageId }}
            search={search}
            onClick={remember}
            onKeyDown={pageLinkKey}
          >
            {reading?.kind === 'listed'
              ? reading.title
              : page
                ? pageTitle(page)
                : text.unknownPageTitle}
          </Link>
        ) : (
          <HomeTitleLink page={page} hints={hints} onClick={remember} />
        )}
      </p>
      {reading?.kind === 'listed' && page && !page.titleMissing && (
        <p className="shell-cell-source">{t.inPage(pageTitle(page))}</p>
      )}
      {reading?.kind === 'listed' && reading.state === 'unread' && (
        <p className="shell-cell-why">
          <AgentChip name={reading.agent} />: {reading.why}
        </p>
      )}
      {cell.note && (
        <p className="shell-cell-note" title={cell.note}>
          {cell.note}
        </p>
      )}
      {cell.audience && <p className="shell-cell-audience">{t.audience(cell.audience)}</p>}
      {cell.needsYou > 0 && (
        <p className="shell-cell-needs">
          <span aria-hidden="true">◆</span> {t.needsYou(cell.needsYou)}
        </p>
      )}
    </li>
  );
}
