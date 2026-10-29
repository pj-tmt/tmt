import { useEffect, useMemo, useRef, useState } from 'react';
import type { PageProgress } from './bootstrap.js';
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
): {
  stageOf?: (pageId: string) => string | undefined;
  /** Whether the existing owner list has been observed, including an empty list. */
  observed: boolean;
  pages?: readonly PageSummary[];
  knownChild: boolean;
  parent?: WorkParent;
  subpages: readonly { id: string; title?: string; state?: PageSummary['state'] }[];
} {
  const key = followed(work);
  const [seen, setSeen] = useState(() => ({
    transport,
    space,
    pages: rememberedPages(transport, space),
  }));
  const pages = seen.transport === transport && seen.space === space ? seen.pages : undefined;
  useEffect(() => {
    const publish = (pages: readonly PageSummary[] | undefined) =>
      setSeen({ transport, space, pages });
    publish(rememberedPages(transport, space));
    return watchRememberedPages(transport, space, publish);
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
  const subpages = useMemo(
    () =>
      [...new Set(key.split(' ').filter(Boolean))].map((id) => {
        const row = pages?.find((page) => page.id === id);
        return { id, title: row?.title, state: row?.state };
      }),
    [key, pages],
  );
  const knownChild = pages?.find((page) => page.id === pageId)?.summary?.parent !== undefined;
  return { stageOf, parent, subpages, pages, knownChild, observed: pages !== undefined };
}

/** The header and work band consume this same observation, including the attach picker. */
export type LinkedPages = ReturnType<typeof useLinkedPages>;

/** One optional owner details read per page navigation; stale or cancelled results never render. */
export function useParentProgress(
  transport: PageTransport,
  pageId: string,
  parentId: string | undefined,
): PageProgress | undefined {
  const attempted = useRef({ pageId, parentId, transport, token: {}, done: false });
  if (attempted.current.pageId !== pageId)
    attempted.current = { pageId, parentId, transport, token: {}, done: false };
  else if (attempted.current.parentId !== parentId || attempted.current.transport !== transport)
    attempted.current = { ...attempted.current, parentId, transport, token: {} };
  const token = attempted.current.token;
  const [result, setResult] = useState<{ token: object; progress: PageProgress }>();
  const pending = useRef<
    | {
        token: object;
        abort: AbortController;
        readers: number;
        result: Promise<import('./reading-details.js').ReadingDetails>;
      }
    | undefined
  >(undefined);
  useEffect(() => {
    if (!parentId || !transport.readingDetails) return;
    let read = pending.current;
    if (read?.token !== token) {
      if (attempted.current.done) return;
      attempted.current.done = true;
      const abort = new AbortController();
      let result: Promise<import('./reading-details.js').ReadingDetails>;
      try {
        result = Promise.resolve(transport.readingDetails(parentId, abort.signal));
      } catch (error) {
        result = Promise.reject(error);
      }
      read = { token, abort, readers: 0, result };
      pending.current = read;
    }
    const current = read;
    current.readers++;
    let active = true;
    void current.result.then(
      (value) => {
        if (
          active &&
          !current.abort.signal.aborted &&
          value.pageId === parentId &&
          value.state === 'ready' &&
          value.progress
        )
          setResult({ token, progress: value.progress });
      },
      () => {},
    );
    return () => {
      active = false;
      current.readers--;
      // StrictMode reattaches the same effect before this microtask. A real departure has no reader
      // and cancels the request; the inactive callback cannot publish during either transition.
      queueMicrotask(() => {
        if (current.readers === 0) current.abort.abort();
      });
    };
  }, [transport, pageId, parentId, token]);
  return result?.token === token ? result.progress : undefined;
}
