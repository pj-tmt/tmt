import { getRouteApi, Link, useRouter } from '@tanstack/react-router';
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent,
} from 'react';
import { holdSpace, openChapter, prepareToday, useOpenChapter } from './chapter.js';
import { ChapterSurfaceSlot, useChapterSurfaces, useChapterCounts } from './chapter-surfaces.js';
import { NeedsYouBar } from './components/needs-you-bar.js';
import { useHomeSpace, useSpaceTitle } from './home-space.js';
import { HomeTitleLink, type HomeTitleHints } from './home-titles.js';
import { TodayReadingLinks, useTodayReadings } from './today-readings.js';
import type { ReadingDetails } from './reading-details.js';
import { pageUpdate } from './page-index.js';
import { text } from './strings.js';
import { dateLine, greetingPart, todayModel } from './today-model.js';
import type { PageSummary, SpaceHome } from './transport.js';
import './today.css';

const t = text.today;
const todayRoute = getRouteApi('/');
const rootRoute = getRouteApi('__root__');

type Jump = 'waiting' | 'updated-today';
/** Waiting asked for from All pages: Today scrolls once it has rendered. */
let pendingJump: Jump | undefined;

/** Scrolls to the first chapter of a group and puts focus on its title link. The location hash
 * holds the mounted space's route, so a jump is a scroll, never a URL anchor. */
function jumpTo(jump: Jump): void {
  const link = document.querySelector<HTMLElement>(`[data-today-${jump}] a`);
  link?.scrollIntoView({ block: 'start' });
  link?.focus({ preventScroll: true });
}

/** Today · Waiting · All pages. The ⌘K search stays the header's existing control. */
export function HomeNav({ current, waiting }: { current: 'today' | 'all'; waiting?: number }) {
  const router = useRouter();
  return (
    <nav className="home-nav" aria-label={t.navLabel}>
      <Link
        to="/"
        className="today-control"
        activeOptions={{ exact: true }}
        data-current={current === 'today' || undefined}
        aria-current={current === 'today' ? 'page' : undefined}
      >
        {t.nav.today}
      </Link>
      <button
        type="button"
        className="today-control"
        disabled={current === 'today' && waiting === 0}
        onClick={(event) => {
          if (!event.isTrusted) return;
          if (current === 'today') return jumpTo('waiting');
          pendingJump = 'waiting';
          void router.navigate({ to: '/' });
        }}
      >
        {t.nav.waiting}
      </button>
      <Link
        to="/all"
        className="today-control"
        data-current={current === 'all' || undefined}
        aria-current={current === 'all' ? 'page' : undefined}
      >
        {t.nav.all}
      </Link>
    </nav>
  );
}

function Chapter({
  page,
  waiting,
  today,
  first,
  now,
  hints,
  details,
  onOpen,
  open,
}: {
  page: PageSummary;
  waiting: boolean;
  today: boolean;
  /** Which jump targets this chapter is the first of. */
  first: readonly Jump[];
  now: number;
  hints?: HomeTitleHints;
  details?: ReadingDetails;
  onOpen: (event: MouseEvent<HTMLAnchorElement>, page: PageSummary) => void;
  /** The chapter that is open beside Today. */
  open: boolean;
}) {
  const router = useRouter();
  const title = useRef<HTMLHeadingElement>(null);
  const openPassage = useCallback(() => {
    openChapter(router, page.id, {
      scroll: window.scrollY,
      link: title.current?.querySelector('a') ?? null,
      replace: false,
    });
  }, [router, page.id]);
  const url = new URL(router.history.createHref(`/pages/${page.id}`), location.href).href;
  const liveCounts = useChapterCounts();
  const count = liveCounts.get(page.id) ?? page.summary?.needsYou;
  const needsOwner = count !== undefined ? count > 0 : waiting;
  const updated = pageUpdate(page.lastUpdateAtMs, now);
  const when = updated.dateTime ? t.kickerUpdated(updated.label) : updated.label;
  const comments = page.summary?.comments;
  const author = page.summary?.author;
  return (
    <li
      className="today-chapter"
      data-page-id={page.id}
      data-today-waiting={first.includes('waiting') || undefined}
      data-today-updated-today={first.includes('updated-today') || undefined}
      data-updated-today={today || undefined}
      data-open={open || undefined}
    >
      <article>
        <div className="today-chapter-main">
          <p className="today-kicker">
            {needsOwner ? (
              <>
                <span className="today-mark" aria-hidden="true">
                  ◆
                </span>{' '}
                {t.kickerWaiting}
              </>
            ) : (
              when
            )}
          </p>
          <h2 className="today-title" ref={title}>
            <HomeTitleLink page={page} hints={hints} onClick={(event) => onOpen(event, page)} />
          </h2>
          <p className="today-meta">
            {author ? <span>{t.byAuthor(author)}</span> : null}
            {needsOwner && (updated.dateTime ? <span>{when}</span> : <span>{updated.label}</span>)}
            {page.labels?.length ? (
              <span className="today-labels">
                {page.labels.map((name) => (
                  <span key={name} className="today-label">
                    {name}
                  </span>
                ))}
              </span>
            ) : null}
          </p>
        </div>
        <div className="today-chapter-state">
          {count !== undefined && (waiting || liveCounts.has(page.id)) && (
            <NeedsYouBar
              tone={count > 0 ? 'needs' : 'done'}
              word={count > 0 ? t.pendingCount(count) : text.needsYouNothingWord}
            />
          )}
          <TodayReadingLinks pageId={page.id} details={details} />
          <ChapterSurfaceSlot id={page.id} open={openPassage} url={url} />
          {comments ? (
            <span className="today-comments">{text.pageList.commentCount(comments)}</span>
          ) : null}
        </div>
      </article>
    </li>
  );
}

export function Today() {
  const space = todayRoute.useLoaderData();
  return <TodayView space={space} />;
}

/** Today's page, or its held copy behind an open chapter (`held`: the offset it keeps). */
export function TodayView({
  space,
  held,
}: {
  space: SpaceHome;
  /** The held copy behind an open chapter: the offset it keeps and the chapter that is open. */
  held?: { scroll: number; open: string };
}) {
  const router = useRouter();
  const openedChapter = useOpenChapter();
  const { transport } = rootRoute.useRouteContext();
  const { pages, hints } = useHomeSpace(space);
  // The held copy hides the column the reading links sit in, so it reads none.
  const noPages = useMemo(() => ({ ...space, pages: [] }), [space]);
  const readingDetails = useTodayReadings(held ? noPages : space);
  // The panel over Today shows this same list, so it paints at once.
  useLayoutEffect(() => {
    if (!held) holdSpace(transport, space);
  }, [held, transport, space]);
  useSpaceTitle(held ? undefined : space.title, text.product);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => setNow(Date.now()), [space.pages]);
  useEffect(() => {
    if (held) return;
    const jump = pendingJump;
    pendingJump = undefined;
    if (jump) jumpTo(jump);
  }, [held]);
  // Coming back from a chapter: same offset, same chapter focused.
  const section = useRef<HTMLElement>(null);
  useLayoutEffect(() => {
    if (!held && section.current) prepareToday(router, section.current);
  }, [held, router]);
  /** A plain click opens the chapter beside Today (wide) or as the full page; a modified click
   * keeps the link's own behaviour. Another chapter, while one is open, replaces it. */
  function onOpen(event: MouseEvent<HTMLAnchorElement>, page: PageSummary) {
    if (
      !event.isTrusted ||
      event.defaultPrevented ||
      event.button !== 0 ||
      event.metaKey ||
      event.ctrlKey ||
      event.shiftKey ||
      event.altKey
    )
      return;
    event.preventDefault();
    openChapter(router, page.id, {
      scroll: held ? held.scroll : window.scrollY,
      link: event.currentTarget,
      replace: !!held,
    });
  }
  const model = useMemo(() => todayModel(pages, now), [pages, now]);
  const liveCounts = useChapterCounts();
  const counts = useMemo(() => todayModel(pages, now, liveCounts).counts, [pages, now, liveCounts]);
  const startOfDay = new Date(now).setHours(0, 0, 0, 0);
  const chapters = useMemo(
    () => [
      ...model.waiting.map((page) => ({ page, waiting: true })),
      ...model.recent.map((page) => ({ page, waiting: false })),
    ],
    [model],
  );
  const chapterOrder = useMemo(() => chapters.map(({ page }) => page.id), [chapters]);
  useChapterSurfaces(chapterOrder, space.pages, !held && !openedChapter);
  const isToday = (page: PageSummary) =>
    page.lastUpdateAtMs != null && page.lastUpdateAtMs >= startOfDay;
  const firstToday = chapters.findIndex(({ page }) => isToday(page));
  const firstWaiting = chapters.findIndex(
    ({ page }) => (liveCounts.get(page.id) ?? page.summary?.needsYou ?? 0) > 0,
  );
  return (
    <section ref={section} className="today">
      <HomeNav current="today" waiting={counts.waiting} />
      <header className="today-hero">
        <p className="today-date">{dateLine(now)}</p>
        <h1>{t.greeting[greetingPart(now)]}</h1>
        {counts.pages > 0 && (
          <p className="today-count" data-waiting={counts.waiting > 0 || undefined}>
            {counts.waiting > 0 ? (
              <>
                <span className="today-mark" aria-hidden="true">
                  ◆
                </span>{' '}
                {t.waitingCount(counts.waiting, counts.complete)}
              </>
            ) : counts.complete ? (
              <>
                <span aria-hidden="true">✓</span> {t.nothingWaiting}
              </>
            ) : (
              t.checking
            )}
          </p>
        )}
      </header>
      {counts.pages === 0 ? (
        <div className="today-empty">
          <p>
            {text.empty}
            <code className="tmt-ui-code">tmt colab page create --title "Notes"</code>.
          </p>
        </div>
      ) : (
        <>
          <ul className="today-metrics" aria-label={t.metricsLabel}>
            <li>
              <button
                type="button"
                className="today-control"
                disabled={counts.waiting === 0}
                onClick={(event) => {
                  if (event.isTrusted) jumpTo('waiting');
                }}
              >
                <strong className="today-numeral">
                  {counts.complete
                    ? counts.waiting
                    : counts.waiting > 0
                      ? `${counts.waiting}+`
                      : '—'}
                </strong>
                <span>{t.metrics.waiting}</span>
              </button>
            </li>
            <li>
              <button
                type="button"
                className="today-control"
                disabled={counts.updatedToday === 0}
                onClick={(event) => {
                  if (event.isTrusted) jumpTo('updated-today');
                }}
              >
                <strong className="today-numeral">{counts.updatedToday}</strong>
                <span>{t.metrics.updatedToday}</span>
              </button>
            </li>
            <li>
              <Link to="/all" className="today-control">
                <strong className="today-numeral">{counts.pages}</strong>
                <span>{t.metrics.pages}</span>
              </Link>
            </li>
          </ul>
          <ul className="today-chapters" aria-label={t.chaptersLabel}>
            {chapters.map(({ page, waiting }, index) => (
              <Chapter
                key={page.id}
                page={page}
                waiting={waiting}
                today={isToday(page)}
                first={[
                  ...(index === firstWaiting ? (['waiting'] as const) : []),
                  ...(index === firstToday ? (['updated-today'] as const) : []),
                ]}
                now={now}
                hints={hints}
                details={readingDetails.get(page.id)}
                onOpen={onOpen}
                open={held?.open === page.id}
              />
            ))}
          </ul>
          {model.folded.length > 0 && (
            <ul className="today-fold" aria-label={t.olderLabel}>
              {model.folded.map((page) => {
                const updated = pageUpdate(page.lastUpdateAtMs, now);
                return (
                  <li
                    key={page.id}
                    data-page-id={page.id}
                    data-open={held?.open === page.id || undefined}
                  >
                    <HomeTitleLink
                      page={page}
                      hints={hints}
                      onClick={(event) => onOpen(event, page)}
                    />
                    <span className="today-fold-time">{updated.label}</span>
                  </li>
                );
              })}
            </ul>
          )}
        </>
      )}
    </section>
  );
}
