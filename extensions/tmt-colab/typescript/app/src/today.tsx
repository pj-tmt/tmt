import { getRouteApi, Link, useRouter } from '@tanstack/react-router';
import { useEffect, useState } from 'react';
import { NeedsYouBar } from './components/needs-you-bar.js';
import { useHomeSpace, useSpaceTitle } from './home-space.js';
import { HomeTitleLink, type HomeTitleHints } from './home-titles.js';
import { pageUpdate } from './page-index.js';
import { text } from './strings.js';
import { dateLine, greetingPart, todayModel } from './today-model.js';
import type { PageSummary } from './transport.js';
import './today.css';

const t = text.today;
const todayRoute = getRouteApi('/');

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
}: {
  page: PageSummary;
  waiting: boolean;
  today: boolean;
  /** Which jump targets this chapter is the first of. */
  first: readonly Jump[];
  now: number;
  hints?: HomeTitleHints;
}) {
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
    >
      <article>
        <div className="today-chapter-main">
          <p className="today-kicker">
            {waiting ? (
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
          <h2 className="today-title">
            <HomeTitleLink page={page} hints={hints} />
          </h2>
          <p className="today-meta">
            {author ? <span>{t.byAuthor(author)}</span> : null}
            {waiting && (updated.dateTime ? <span>{when}</span> : <span>{updated.label}</span>)}
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
          {waiting && <NeedsYouBar tone="needs" word={t.waitingForYou} />}
          {comments === undefined ? null : (
            <span className="today-comments">{text.pageList.commentCount(comments)}</span>
          )}
        </div>
      </article>
    </li>
  );
}

export function Today() {
  const space = todayRoute.useLoaderData();
  const { pages, hints } = useHomeSpace(space);
  useSpaceTitle(space.title, text.product);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => setNow(Date.now()), [space.pages]);
  useEffect(() => {
    const jump = pendingJump;
    pendingJump = undefined;
    if (jump) jumpTo(jump);
  }, []);
  const model = todayModel(pages, now);
  const { counts } = model;
  const startOfDay = new Date(now).setHours(0, 0, 0, 0);
  const chapters = [
    ...model.waiting.map((page) => ({ page, waiting: true })),
    ...model.recent.map((page) => ({ page, waiting: false })),
  ];
  const isToday = (page: PageSummary) =>
    page.lastUpdateAtMs != null && page.lastUpdateAtMs >= startOfDay;
  const firstToday = chapters.findIndex(({ page }) => isToday(page));
  return (
    <section className="today">
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
                {t.waitingCount(counts.waiting)}
              </>
            ) : (
              <>
                <span aria-hidden="true">✓</span> {t.nothingWaiting}
              </>
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
                <strong className="today-numeral">{counts.waiting}</strong>
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
                  ...(waiting && index === 0 ? (['waiting'] as const) : []),
                  ...(index === firstToday ? (['updated-today'] as const) : []),
                ]}
                now={now}
                hints={hints}
              />
            ))}
          </ul>
          {model.folded.length > 0 && (
            <ul className="today-fold" aria-label={t.olderLabel}>
              {model.folded.map((page) => {
                const updated = pageUpdate(page.lastUpdateAtMs, now);
                return (
                  <li key={page.id} data-page-id={page.id}>
                    <HomeTitleLink page={page} hints={hints} />
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
