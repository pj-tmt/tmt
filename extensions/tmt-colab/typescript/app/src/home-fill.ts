import { useEffect, useState } from 'react';

/** The owner folds page summaries in the background; until every page has one the list reads again. */
export const REREAD_MS = 2000;
/** At most this many re-reads while pages are still missing a summary; a reload starts again. */
export const REREADS = 15;

export type FillStep = 'schedule' | 'wait' | 'stop';
/**
 * What the list does about pages without a summary: with none left, or after `REREADS` tries, it
 * stops; a hidden tab waits (and never reads); a visible one schedules the next re-read.
 */
export function fillStep(missing: number, tries: number, visible: boolean): FillStep {
  if (missing === 0 || tries >= REREADS) return 'stop';
  return visible ? 'schedule' : 'wait';
}

/**
 * Reads the list again without moving the reader: the router scrolls the window to the top when any
 * load renders, and a re-read is not a navigation. Only that reset is undone, and to where the
 * reader last was, so one who scrolls while the read is in flight keeps their own position.
 */
export async function rereadInPlace(reload: () => Promise<unknown>) {
  let where = window.scrollY;
  const follow = () => {
    if (window.scrollY > 0) where = window.scrollY;
  };
  window.addEventListener('scroll', follow, { passive: true });
  try {
    await reload();
  } finally {
    window.removeEventListener('scroll', follow);
  }
  if (where > 0 && window.scrollY === 0) window.scrollTo({ top: where });
}

/**
 * Reads the list again every `REREAD_MS` while `missing` pages have no summary, only while the tab is
 * visible and at most `REREADS` times, and stops the moment the pages are all in or the list unmounts.
 * Returns whether that is still going on, so an empty tab can say it is reading rather than empty.
 */
export function useSummaryFill(missing: number, reread: () => unknown): boolean {
  const [tries, setTries] = useState(0);
  const [visible, setVisible] = useState(() => document.visibilityState === 'visible');
  useEffect(() => {
    const onChange = () => setVisible(document.visibilityState === 'visible');
    document.addEventListener('visibilitychange', onChange);
    return () => document.removeEventListener('visibilitychange', onChange);
  }, []);
  // A complete list starts over: a page edited later loses its summary and gets a fresh set of tries.
  useEffect(() => {
    if (missing === 0) setTries(0);
  }, [missing]);
  useEffect(() => {
    if (fillStep(missing, tries, visible) !== 'schedule') return;
    const timer = window.setTimeout(() => {
      setTries((count) => count + 1);
      void reread();
    }, REREAD_MS);
    return () => window.clearTimeout(timer);
    // `reread` is the router's invalidate: a new closure each render must not restart the timer.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [missing, tries, visible]);
  return missing > 0 && tries < REREADS;
}
