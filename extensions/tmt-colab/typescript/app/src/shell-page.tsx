import { getRouteApi, Link } from '@tanstack/react-router';
import { useEffect, useRef, useState } from 'react';
import { compactRelativeTime } from './display-time.js';
import { HomeTitleLink } from './home-titles.js';
import { useHomeSpace } from './home-space.js';
import { pageLinkKey } from './page-link.js';
import { pageTitle } from './page-index.js';
import type { Shell } from './shell.js';
import { pagesChangedSince, shellByline, shellCells, type ShellCell } from './shell-grid.js';
import { text } from './strings.js';
import type { SpaceHome } from './transport.js';
import './shell-page.css';

const rootRoute = getRouteApi('__root__');
const t = text.shellPage;

/** The shell page: the app owns the whole look, the document only supplies the data. Items are
 * links to their source pages and carry no content of their own. */
export function ShellPage({ shell }: { shell: Shell }) {
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
        <time dateTime={new Date(byline.timeMs).toISOString()}>
          {t.summaryBy(byline.who, compactRelativeTime(byline.timeMs, now))}
        </time>
        {changed > 0 && <span className="shell-changed"> · {t.changedSince(changed)}</span>}
      </p>
    </section>
  );
}

function ShellGrid({ shell, home, failed }: { shell: Shell; home: SpaceHome; failed: boolean }) {
  const { pages, hints } = useHomeSpace(home);
  const cells = failed
    ? shellCells(shell, []).map((cell) => ({ ...cell, unavailable: false, titleMissing: true }))
    : shellCells(shell, pages);
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
          <ShellCellView key={cell.id} cell={cell} page={byId.get(cell.pageId)} hints={hints} />
        ))}
      </ul>
    </>
  );
}

function ShellCellView({
  cell,
  page,
  hints,
}: {
  cell: ShellCell;
  page: ReturnType<typeof useHomeSpace>['pages'][number] | undefined;
  hints: ReturnType<typeof useHomeSpace>['hints'];
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
            {page ? pageTitle(page) : text.unknownPageTitle}
          </Link>
        ) : (
          <HomeTitleLink page={page} hints={hints} onClick={remember} />
        )}
      </p>
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
