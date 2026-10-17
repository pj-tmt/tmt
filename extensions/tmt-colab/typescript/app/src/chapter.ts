import type { AnyRouter, HistoryState } from '@tanstack/react-router';
import { useSyncExternalStore } from 'react';
import type { PageTransport, SpaceHome } from './transport.js';

/** A chapter opened from Today. The page route stays `/pages/<id>`; this history state is only how
 * it is presented: a right panel over a held Today, or the full page. A direct visit has none. */
export interface ChapterState {
  /** Today's window scroll offset when the chapter was opened. */
  scroll: number;
  /** The reader chose, or the viewport left no room for, the full page. */
  full?: true;
}
/** What a history entry says about its chapter, read defensively: the state is the browser's. */
export function chapterOf(state: object): ChapterState | undefined {
  const value = (state as { chapter?: unknown }).chapter;
  if (typeof value !== 'object' || value === null) return undefined;
  const { scroll, full } = value as Partial<ChapterState>;
  return typeof scroll === 'number' && Number.isFinite(scroll) && scroll >= 0
    ? { scroll, ...(full === true ? { full } : {}) }
    : undefined;
}
const entry = (chapter: ChapterState) => ({ chapter }) as HistoryState;

const WIDE = '(min-width: 900px)';
const REDUCED = '(prefers-reduced-motion: reduce)';
/** The one shared title: the opened chapter's title and the page's, in both directions. */
const TITLE = 'chapter-title';

export type ChapterMode = 'panel' | 'full';

function subscribeWide(notify: () => void): () => void {
  const query = matchMedia(WIDE);
  query.addEventListener('change', notify);
  return () => query.removeEventListener('change', notify);
}
/** Whether the viewport leaves room for the panel beside Today. */
export function useWide(): boolean {
  return useSyncExternalStore(
    subscribeWide,
    () => matchMedia(WIDE).matches,
    () => false,
  );
}
export function chapterMode(
  state: ChapterState | undefined,
  wide: boolean,
): ChapterMode | undefined {
  return state ? (wide && !state.full ? 'panel' : 'full') : undefined;
}

/** Per router: Today's offset and chapter, kept for the way back (Back, Close and ← Today all use
 * it), and whether this document opened the chapter, so Close can step back instead of pushing. */
const trips = new WeakMap<
  object,
  {
    left?: { id: string; scroll: number };
    opened: boolean;
    today?: HTMLElement;
    named?: HTMLElement;
  }
>();
function trip(router: object) {
  let value = trips.get(router);
  if (!value) trips.set(router, (value = { opened: false }));
  return value;
}
const heldSpaces = new WeakMap<PageTransport, SpaceHome>();

/** Today shares the list it shows, so the panel's held copy renders at once. */
export function holdSpace(transport: PageTransport, space: SpaceHome): void {
  heldSpaces.set(transport, space);
}
export function heldSpace(transport: PageTransport): SpaceHome | undefined {
  return heldSpaces.get(transport);
}

function animated(): boolean {
  return typeof document.startViewTransition === 'function' && !matchMedia(REDUCED).matches;
}
/** Names one element the transition's shared group follows. A router names one at a time, so the
 * name is unique within it; another router's tree is never touched. */
function nameTitle(router: AnyRouter, element: HTMLElement | null | undefined): void {
  const state = trip(router);
  state.named?.style.removeProperty('view-transition-name');
  state.named = element ?? undefined;
  if (element) element.style.viewTransitionName = TITLE;
}
/** Today as it comes back (mounted, before the router's own reset runs): the offset it was left
 * at, and the chapter's title named for the shared transition. */
export function prepareToday(router: AnyRouter, section: HTMLElement): void {
  const state = trip(router);
  state.today = section;
  const left = state.left;
  if (!left) return;
  window.scrollTo(0, left.scroll);
  nameTitle(router, section.querySelector<HTMLElement>(`[data-page-id="${left.id}"] a`));
}
/** The router has rendered Today: its own scroll reset has run, so the offset is set once more and
 * the chapter that was left takes focus. */
export function finishReturn(router: AnyRouter): void {
  const state = trip(router);
  const { left, today } = state;
  if (!left || !today?.isConnected || router.state.location.pathname !== '/') return;
  state.left = undefined;
  window.scrollTo(0, left.scroll);
  today.querySelector<HTMLElement>(`[data-page-id="${left.id}"] a`)?.focus({ preventScroll: true });
}
/** The page's title takes the shared name while a chapter is open. */
export function nameOpenedTitle(router: AnyRouter, element: HTMLElement | null): void {
  nameTitle(router, element);
}

export function openChapter(
  router: AnyRouter,
  pageId: string,
  options: { scroll: number; link: HTMLElement | null; replace: boolean; full?: true },
): void {
  Object.assign(trip(router), { left: { id: pageId, scroll: options.scroll }, opened: true });
  nameTitle(router, options.link);
  void router.navigate({
    to: '/pages/$pageId',
    params: { pageId },
    state: entry({
      scroll: options.scroll,
      ...(options.full || !matchMedia(WIDE).matches ? { full: true as const } : {}),
    }),
    replace: options.replace,
    viewTransition: animated(),
  });
}
export function expandChapter(router: AnyRouter, pageId: string, scroll: number): void {
  void router.navigate({
    to: '/pages/$pageId',
    params: { pageId },
    state: entry({ scroll, full: true }),
    replace: true,
    viewTransition: animated(),
  });
}
/** Back to Today at the same chapter and offset. */
export function closeChapter(router: AnyRouter, pageId: string, scroll: number): void {
  const state = trip(router);
  state.left = { id: pageId, scroll };
  if (animated()) router.shouldViewTransition = true;
  if (state.opened) {
    state.opened = false;
    router.history.back();
  } else void router.navigate({ to: '/', viewTransition: animated() });
}
/** Escape with nothing left to escape inside the page: an open panel closes, the full page stays. */
export function escapeChapter(router: AnyRouter): void {
  const { pathname, state } = router.state.location;
  const chapter = chapterOf(state);
  if (chapterMode(chapter, matchMedia(WIDE).matches) !== 'panel' || !chapter) return;
  if (pathname.startsWith('/pages/'))
    closeChapter(router, pathname.slice('/pages/'.length), chapter.scroll);
}
