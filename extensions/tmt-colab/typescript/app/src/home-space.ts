import { getRouteApi } from '@tanstack/react-router';
import { useEffect, useState } from 'react';
import { HomeTitleHints } from './home-titles.js';
import type { PageSummary, SpaceHome } from './transport.js';

const rootRoute = getRouteApi('__root__');

/** The space's pages as the home list shows them: a missing title is filled by a bounded read
 * of the page itself (`hints`, enqueued by each title link), never by author-supplied text. */
export function useHomeSpace(space: SpaceHome): {
  pages: readonly PageSummary[];
  hints: HomeTitleHints | undefined;
} {
  const { transport } = rootRoute.useRouteContext();
  const [filled, setFilled] = useState<{ pages: typeof space.pages; titles: Map<string, string> }>(
    () => ({ pages: space.pages, titles: new Map() }),
  );
  const [hints, setHints] = useState<HomeTitleHints>();
  useEffect(() => {
    const read = transport.readTitle;
    if (!read) {
      setHints(undefined);
      return;
    }
    const next = new HomeTitleHints(read, (id, title) =>
      setFilled((previous) => ({
        pages: space.pages,
        titles: new Map(previous.pages === space.pages ? previous.titles : []).set(id, title),
      })),
    );
    setHints(next);
    return () => {
      void next.close();
    };
  }, [transport, space.pages]);
  const pages = space.pages.map((page) =>
    page.titleMissing && filled.pages === space.pages && filled.titles.has(page.id)
      ? { ...page, title: filled.titles.get(page.id)!, titleMissing: false }
      : page,
  );
  return { pages, hints };
}

/** The tab title both home views share. */
export function useSpaceTitle(title: string | undefined, product: string): void {
  useEffect(() => {
    if (title === undefined) return;
    document.title = title === product ? product : `${title} · ${product}`;
  }, [title, product]);
}
