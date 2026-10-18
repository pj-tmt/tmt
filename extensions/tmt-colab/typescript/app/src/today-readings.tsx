import { pageLinkKey } from './page-link.js';
import { getRouteApi, Link } from '@tanstack/react-router';
import { useEffect, useState } from 'react';
import { HomeReadingDetails } from './reading-details.js';
import { todayModel } from './today-model.js';
import type { SpaceHome } from './transport.js';
import type { ReadingDetails } from './reading-details.js';
import { text } from './strings.js';

/** Cosmetic owner navigation only; no discussion, admission or notification controls. */
export function TodayReadingLinks({
  pageId,
  details,
}: {
  pageId: string;
  details?: ReadingDetails;
}) {
  if (details?.state !== 'ready' || details.pageId !== pageId) return null;
  const unread = details.readings.filter((reading) => reading.state === 'unread');
  if (!unread.length) return null;
  return (
    <div className="today-readings">
      <p className="today-readings-kicker">{text.readingPleaseRead}</p>
      <ul>
        {unread.slice(0, 3).map((reading) => (
          <li key={reading.id}>
            <Link
              to="/pages/$pageId"
              params={{ pageId }}
              search={{ read: reading.id }}
              onKeyDown={pageLinkKey}
            >
              {reading.title}
            </Link>
            <span className="today-reading-reason">
              {reading.agent} · {reading.why}
            </span>
          </li>
        ))}
      </ul>
      {unread.length > 3 && (
        <Link
          className="today-readings-more"
          onKeyDown={pageLinkKey}
          to="/pages/$pageId"
          params={{ pageId }}
          search={{ read: unread[0].id }}
        >
          {text.today.readingMore(unread.length - 3)}
        </Link>
      )}
    </div>
  );
}

const rootRoute = getRouteApi('__root__');

/** The loader's pages array is the generation, independent of missing-title fills and paints. */
export function useTodayReadings(space: SpaceHome): ReadonlyMap<string, ReadingDetails> {
  const { transport } = rootRoute.useRouteContext();
  const [held, setHeld] = useState<{
    generation: SpaceHome['pages'];
    values: Map<string, ReadingDetails>;
  }>();
  useEffect(() => {
    const read = transport.readingDetails;
    if (!read) return;
    const model = todayModel(space.pages, Date.now());
    const chapters = [...model.waiting, ...model.recent];
    const summaries = new Map(chapters.map((page) => [page.id, page.summary]));
    const queue = new HomeReadingDetails(read, (id, details) => {
      const summary = summaries.get(id);
      // A different fold may already have landed. Keep the count; never mix its details in.
      if (
        details.state === 'ready' &&
        (!summary ||
          details.summary.needsYou !== summary.needsYou ||
          details.summary.comments !== summary.comments ||
          details.summary.author !== summary.author)
      )
        return;
      setHeld((previous) => ({
        generation: space.pages,
        values: new Map(previous?.generation === space.pages ? previous.values : []).set(
          id,
          details,
        ),
      }));
    });
    queue.enqueue(chapters);
    return () => {
      void queue.close();
    };
  }, [transport, space.pages]);
  return held?.generation === space.pages ? held.values : new Map();
}
