import { useEffect, useMemo, useState } from 'react';
import { pageStage } from './list-stage.js';
import { rememberPages, rememberedPages } from './quick-switcher-source.js';
import type { PageSummary, PageTransport } from './transport.js';
import type { Work } from './work.js';

/** The pages a work value's tasks follow, as one comparable key. */
function followed(work: Work | undefined): string {
  if (!work) return '';
  return work.order
    .map((id) => work.tasks[id]?.child)
    .filter((child): child is string => child !== undefined)
    .join(' ');
}

/**
 * What the band needs to show a task's followed page at its stage: the stage each listed page's
 * summary says, from the owner list the viewer already reads (the list it last loaded at once,
 * then one fresh read). Nothing is read for a page with no task that follows another. A page the
 * list does not name, or whose summary has not arrived, has no stage here, so its task keeps its
 * own stored state, as the page list shows it.
 */
export function useLinkedStage(
  transport: PageTransport,
  space: string | undefined,
  work: Work | undefined,
): ((pageId: string) => string | undefined) | undefined {
  const key = followed(work);
  const [pages, setPages] = useState<readonly PageSummary[] | undefined>(() =>
    rememberedPages(transport, space),
  );
  useEffect(() => {
    if (key === '') return;
    const abort = new AbortController();
    transport.spaceHome(abort.signal).then(
      (home) => {
        if (abort.signal.aborted) return;
        rememberPages(transport, space, home.pages);
        setPages(home.pages);
      },
      () => {
        // The list is unavailable: the tasks keep their own states.
      },
    );
    return () => abort.abort();
  }, [transport, space, key]);
  return useMemo(() => {
    if (key === '' || !pages) return undefined;
    const byId = new Map(pages.map((page) => [page.id, page]));
    return (pageId: string) => {
      const page = byId.get(pageId);
      return page ? pageStage(page) : undefined;
    };
  }, [key, pages]);
}
