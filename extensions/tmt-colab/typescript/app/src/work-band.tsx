import { ChevronDown, ChevronRight, FileText, Plus } from 'lucide-react';
import { browserUiClasses as ui } from '@tmt/browser-ui/static';
import {
  useLayoutEffect,
  useId,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from 'react';
import { text } from './strings.js';
import type { PageState } from './bootstrap.js';
import type { SubpageCounts, SubpageRow } from './work-tree.js';
import './work-band.css';

type RowSlots = Pick<
  WorkBandProps,
  'childrenOf' | 'counts' | 'pageHref' | 'rowState' | 'rowActions'
>;
export interface WorkBandProps {
  onSize?(height: number): void;
  pageState?: PageState | null;
  /** This page's own state chip; the owner's is a menu of its own, which never toggles the band. */
  stateControl?: ReactNode;
  /** Direct sub-pages, in order. */
  subpages: readonly SubpageRow[];
  /** The next level under a sub-page, from what the viewer already has; empty when unknown. */
  childrenOf?(id: string): readonly SubpageRow[];
  /** Direct sub-page progress; absent where states are unknown (a share reader). */
  counts?(rows: readonly SubpageRow[]): SubpageCounts;
  /** The address of a page. */
  pageHref?(pageId: string): string;
  /** A sub-page's state chip; the owner's changes that page's state. */
  rowState?(row: SubpageRow): ReactNode;
  /** The ⋯ menu of a row (Open, Detach); absent for a viewer that cannot change links. */
  rowActions?(row: SubpageRow): ReactNode;
  /** Opens the one Add window; absent where this viewer cannot add. */
  onAdd?(): void;
}

function StateChip({ state }: { state?: PageState | null }) {
  const t = text.workBand;
  return (
    <span className={`work-page-state ${ui.countChip}`} data-state={state ?? 'unknown'}>
      {state ? t.taskState[state] : t.unknown}
    </span>
  );
}

/**
 * The page's work panel: one head row, pinned under the nav, that opens one list of sub-pages.
 * A sub-page with sub-pages of its own expands its next level in place. Progress counts only the
 * direct sub-pages, each by its own state.
 */
export function WorkBand({
  onSize,
  pageState,
  stateControl,
  subpages,
  onAdd,
  ...slots
}: WorkBandProps): ReactNode {
  const t = text.workBand;
  const [open, setOpen] = useState(false);
  const listId = useId();
  const section = useRef<HTMLElement>(null);
  useLayoutEffect(() => {
    const element = section.current;
    if (!element || !onSize) return;
    const measure = () => onSize(element.getBoundingClientRect().height);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [onSize]);
  const empty = subpages.length === 0;
  // With nothing attached the head invites adding instead of toggling an empty list.
  const activate = empty ? onAdd : () => setOpen((now) => !now);
  const progress = slots.counts?.(subpages);
  const buckets = progress
    ? ([
        ['done', progress.done],
        ['in-progress', progress.inProgress],
        ['todo', progress.todo],
        ['unknown', progress.unknown],
      ] as const)
    : [];
  return (
    <section ref={section} className="work-band" aria-label={t.label}>
      {/* The head is a plain row. Its toggle is one real button whose hit area stretches over the
          free row (::after); the chip and Add sit above it and keep their own roles and names. */}
      <div className="work-head" data-empty={empty}>
        <span className="work-head-chip">{stateControl ?? <StateChip state={pageState} />}</span>
        {empty ? (
          <>
            <span className="work-quiet">{t.noSubpages}</span>
            {onAdd && (
              <button type="button" className="work-add-text" onClick={onAdd}>
                <Plus aria-hidden="true" />
                {t.add}
              </button>
            )}
          </>
        ) : (
          <>
            {progress && (
              <>
                <span
                  className="work-segments"
                  role="img"
                  aria-label={buckets
                    .map(
                      ([state, count]) =>
                        `${state === 'unknown' ? t.unknown : t.taskState[state]} ${count}`,
                    )
                    .join(', ')}
                >
                  {buckets.map(
                    ([state, count]) =>
                      count > 0 && (
                        <span key={state} data-state={state} style={{ flexGrow: count }} />
                      ),
                  )}
                </span>
                <span className="work-progress">
                  <span className="work-wide">{t.progress(progress.done, progress.total)}</span>
                  <span className="work-narrow">
                    {t.progressShort(progress.done, progress.total)}
                  </span>
                </span>
              </>
            )}
          </>
        )}
        {activate && (
          <button
            type="button"
            className="work-toggle"
            aria-label={empty ? t.addSubpage : open ? t.collapsePanel : t.expandPanel}
            aria-expanded={empty ? undefined : open}
            aria-controls={empty ? undefined : listId}
            onClick={activate}
          >
            {!empty && <ChevronDown className="work-chevron" data-open={open} aria-hidden="true" />}
          </button>
        )}
      </div>
      {!empty && (
        <div className="work-body-transition" data-open={open} inert={!open}>
          <div className="work-body" id={listId}>
            <div className="work-body-content">
              <ul className="work-tree" aria-label={t.subpages}>
                <Rows rows={subpages} depth={0} trail={[]} {...slots} />
              </ul>
              {onAdd && (
                <div className="work-add-row">
                  <button type="button" className="work-add-text" onClick={onAdd}>
                    <Plus aria-hidden="true" />
                    {t.add}
                  </button>
                </div>
              )}
            </div>
          </div>
        </div>
      )}
    </section>
  );
}

function Rows({
  rows,
  depth,
  trail,
  ...slots
}: RowSlots & {
  rows: readonly SubpageRow[];
  depth: number;
  /** Pages already drawn above this level: a legacy loop is never drawn twice. */
  trail: readonly string[];
}) {
  return rows.map((row) => (
    <Row key={`${row.parent}/${row.id}`} row={row} depth={depth} trail={trail} {...slots} />
  ));
}

function Row({
  row,
  depth,
  trail,
  ...slots
}: RowSlots & { row: SubpageRow; depth: number; trail: readonly string[] }) {
  const t = text.workBand;
  const { childrenOf, counts, pageHref, rowState, rowActions } = slots;
  const [open, setOpen] = useState(false);
  const below = trail.includes(row.id) ? [] : (childrenOf?.(row.id) ?? []);
  const title = row.title ?? t.unknownPage;
  const own = below.length > 0 ? counts?.(below) : undefined;
  return (
    <li className="work-row">
      <div className="work-row-line">
        <span className="work-row-name" style={{ '--work-depth': depth } as CSSProperties}>
          {below.length > 0 ? (
            <button
              type="button"
              className="work-twisty"
              aria-expanded={open}
              aria-label={open ? t.collapseRow(title) : t.expandRow(title)}
              onClick={() => setOpen(!open)}
            >
              {open ? <ChevronDown aria-hidden="true" /> : <ChevronRight aria-hidden="true" />}
            </button>
          ) : (
            <span className="work-twisty" />
          )}
          <FileText className="work-row-icon" aria-hidden="true" />
          {pageHref ? (
            <a className="work-row-title" href={pageHref(row.id)}>
              {title}
            </a>
          ) : (
            <span className="work-row-title">{title}</span>
          )}
        </span>
        <span className="work-row-progress">
          {own ? t.rowProgress(own.done, own.total) : t.noProgress}
        </span>
        <span className="work-row-state">{rowState?.(row) ?? <StateChip state={row.state} />}</span>
        <span className="work-row-more">{rowActions?.(row)}</span>
      </div>
      {open && below.length > 0 && (
        <ul className="work-tree">
          <Rows rows={below} depth={depth + 1} trail={[...trail, row.id]} {...slots} />
        </ul>
      )}
    </li>
  );
}
