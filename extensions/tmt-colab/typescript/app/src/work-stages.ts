import { useEffect, useState } from 'react';
import { rememberPages, rememberedPages, watchRememberedPages } from './quick-switcher-source.js';
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
 * The owner list the viewer already has, which names each sub-page's title, state and parent: the
 * list the space home last loaded, followed as other views refresh it. A page without sub-pages
 * reads nothing extra; a page with some makes one fresh read so their titles and states are known.
 * Deeper levels come from the same list's parent links, never from a read of another page.
 */
export function useLinkedPages(
  transport: PageTransport,
  space: string | undefined,
  work: Work | undefined,
): {
  /** Whether the existing owner list has been observed, including an empty list. */
  observed: boolean;
  pages?: readonly PageSummary[];
} {
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
        // The list is unavailable: the sub-pages show without titles or states.
      },
    );
    return () => abort.abort();
  }, [transport, space, key]);
  return { pages, observed: pages !== undefined };
}
