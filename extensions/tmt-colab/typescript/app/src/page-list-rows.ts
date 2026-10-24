import { useEffect, useState, type RefObject } from 'react';

/** Rows the list draws at first and adds each time its end scrolls near. */
export const PAGE_LIST_STEP = 50;

/**
 * How many of `total` rows the list draws. It starts at one step whenever `key` (the view, filter
 * and sort) changes, and adds a step while the explicit count-line sentinel is within a
 * screen of its bottom edge. The observer is made anew after each step, so a short screen that
 * still shows the count line keeps adding until it scrolls or every row is drawn.
 */
export function useShownRows(
  scroller: RefObject<HTMLElement | null>,
  sentinel: RefObject<HTMLElement | null>,
  total: number,
  key: string,
): number {
  const [state, setState] = useState({ key, shown: PAGE_LIST_STEP });
  const shown = state.key === key ? state.shown : PAGE_LIST_STEP;
  if (state.key !== key) setState({ key, shown });
  useEffect(() => {
    scroller.current?.scrollTo({ top: 0 });
  }, [scroller, key]);
  useEffect(() => {
    const root = scroller.current;
    const end = sentinel.current;
    if (!root || !end || shown >= total) return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting))
          setState((previous) =>
            previous.key === key ? { key, shown: previous.shown + PAGE_LIST_STEP } : previous,
          );
      },
      { root, rootMargin: '0px 0px 100% 0px' },
    );
    observer.observe(end);
    return () => observer.disconnect();
  }, [scroller, sentinel, total, key, shown]);
  return shown;
}
