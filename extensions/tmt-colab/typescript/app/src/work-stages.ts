import { useEffect, useMemo, useState } from 'react';
import { pageStage } from './list-stage.js';
import { rememberPages, rememberedPages, watchRememberedPages } from './quick-switcher-source.js';
import type { PageSummary, PageTransport } from './transport.js';
import type { Work } from './work.js';
import type { WorkParent } from './work-band.js';

/** The pages a work value's tasks follow, as one comparable key. */
function followed(work: Work | undefined): string {
  if (!work) return '';
  return work.order
    .map((id) => work.tasks[id]?.child)
    .filter((child): child is string => child !== undefined)
    .join(' ');
}

/**
 * What the band needs from the owner list the viewer already has: the stage each listed page's
 * summary says, for a task that follows a page, and the task of another page that follows this
 * one, named with that page's title as the list shows it. The list is the one the space home last
 * loaded, followed as other views refresh it (arriving from the list, or the read a link-opened
 * page already makes for its chips), so a page without work reads nothing extra. A page with a task
 * that follows a page also makes one fresh read. A page the list does not name, or whose summary
 * has not arrived, has no stage here, so its task keeps its own stored state, as the page list
 * shows it; a parent the list does not name is no breadcrumb, never a read of that page.
 */
export function useLinkedPages(
  transport: PageTransport,
  space: string | undefined,
  pageId: string,
  work: Work | undefined,
): { stageOf?: (pageId: string) => string | undefined; parent?: WorkParent } {
  const key = followed(work);
  const [pages, setPages] = useState<readonly PageSummary[] | undefined>(() =>
    rememberedPages(transport, space),
  );
  useEffect(() => {
    setPages(rememberedPages(transport, space));
    return watchRememberedPages(transport, space, setPages);
  }, [transport, space]);
  useEffect(() => {
    if (key === '') return;
    const abort = new AbortController();
    transport.spaceHome(abort.signal).then(
      (home) => {
        if (abort.signal.aborted) return;
        rememberPages(transport, space, home.pages);
      },
      () => {
        // The list is unavailable: the tasks keep their own states.
      },
    );
    return () => abort.abort();
  }, [transport, space, key]);
  const stageOf = useMemo(() => {
    if (key === '' || !pages) return undefined;
    const byId = new Map(pages.map((page) => [page.id, page]));
    return (id: string) => {
      const page = byId.get(id);
      return page ? pageStage(page) : undefined;
    };
  }, [key, pages]);
  const parent = useMemo(() => {
    const link = pages?.find((page) => page.id === pageId)?.summary?.parent;
    const held = link && pages?.find((page) => page.id === link.page);
    return link && held
      ? { pageId: link.page, taskId: link.task, title: held.title, taskTitle: link.title }
      : undefined;
  }, [pages, pageId]);
  return { stageOf, parent };
}
