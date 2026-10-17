import { getRouteApi, useMatch, useRouter } from '@tanstack/react-router';
import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from 'react';
import {
  closeChapter,
  escapeChapter,
  expandChapter,
  heldSpace,
  nameOpenedTitle,
  type ChapterMode,
  type ChapterState,
} from './chapter.js';
import { pageTitle } from './page-index.js';
import { text } from './strings.js';
import { TodayView } from './today.js';
import type { SpaceHome } from './transport.js';
import './chapter.css';

const t = text.chapter;
const COVERING = '[role="dialog"],dialog[open],[aria-modal="true"],[role="menu"],[role="listbox"]';
const rootRoute = getRouteApi('__root__');

/** Today, held in place under an open chapter: its list, offset and width unchanged, clipped by the
 * panel. A click on a chapter switches the panel to it; a click anywhere else on Today closes. */
export function HeldToday({ pageId, chapter }: { pageId: string; chapter: ChapterState }) {
  const router = useRouter();
  const { transport } = rootRoute.useRouteContext();
  const [space, setSpace] = useState<SpaceHome | undefined>(() => heldSpace(transport));
  useEffect(() => {
    if (space) return;
    let current = true;
    void transport.spaceHome().then(
      (home) => current && setSpace(home),
      () => undefined,
    );
    return () => {
      current = false;
    };
  }, [transport, space]);
  return (
    <div
      className="chapter-held"
      style={{ '--chapter-scroll': `${chapter.scroll}px` } as CSSProperties}
      onClick={(event) => {
        if (!event.isTrusted || (event.target as Element).closest('a')) return;
        closeChapter(router, pageId, chapter.scroll);
      }}
    >
      <div className="chapter-held-inner">
        {space && <TodayView space={space} held={{ scroll: chapter.scroll }} />}
      </div>
    </div>
  );
}

/** The panel's header (title, Close, Open full page) or, on the full page, the way back to Today.
 * Esc closes unless something inside the page, a popup or a drawer took it first. */
export function ChapterBar({
  mode,
  pageId,
  chapter,
}: {
  mode: ChapterMode;
  pageId: string;
  chapter: ChapterState;
}) {
  const router = useRouter();
  const { transport } = rootRoute.useRouteContext();
  const opened = useMatch({ from: '/pages/$pageId', shouldThrow: false });
  const known = heldSpace(transport)?.pages.find((page) => page.id === pageId);
  const title = opened?.loaderData?.title || (known ? pageTitle(known) : text.unknownPageTitle);
  const bar = useRef<HTMLElement>(null);
  const heading = useRef<HTMLHeadingElement>(null);
  useLayoutEffect(() => {
    document.documentElement.dataset.chapter = mode;
    nameOpenedTitle(
      router,
      mode === 'panel'
        ? heading.current
        : document.querySelector<HTMLElement>('.tmt-ui-header .tmt-ui-title'),
    );
    return () => {
      delete document.documentElement.dataset.chapter;
    };
  }, [mode, pageId, router]);
  useEffect(() => {
    if (mode === 'panel') bar.current?.focus({ preventScroll: true });
  }, [mode, pageId]);
  useEffect(() => {
    if (mode !== 'panel') return;
    const covered = new WeakSet<Event>();
    // Before anything reacts: a dialog, drawer, menu or list inside the page closes itself first.
    function seen(event: KeyboardEvent) {
      if ([...document.querySelectorAll(COVERING)].some((node) => node.checkVisibility()))
        covered.add(event);
    }
    function onKey(event: KeyboardEvent) {
      const target = event.target as Element | null;
      if (
        event.key !== 'Escape' ||
        !event.isTrusted ||
        event.defaultPrevented ||
        covered.has(event) ||
        target?.closest('input,textarea,select,[contenteditable],[role="textbox"]')
      )
        return;
      // The page's own Escape (annotation, chat, selection) is never swallowed: it runs in this
      // same dispatch, so look once it has finished.
      setTimeout(() => {
        if (!event.defaultPrevented) escapeChapter(router);
      }, 0);
    }
    document.addEventListener('keydown', seen, true);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('keydown', seen, true);
      document.removeEventListener('keydown', onKey);
    };
  }, [mode, pageId, chapter.scroll, router]);
  if (mode === 'full')
    return (
      <nav className="chapter-back" aria-label={t.navLabel}>
        <button
          type="button"
          className="chapter-control"
          onClick={(event) => {
            if (event.isTrusted) closeChapter(router, pageId, chapter.scroll);
          }}
        >
          {t.back}
        </button>
      </nav>
    );
  return (
    <section ref={bar} className="chapter-bar" aria-label={t.panelLabel} tabIndex={-1}>
      <h2 ref={heading} className="chapter-title">
        {title}
      </h2>
      <div className="chapter-actions">
        <button
          type="button"
          className="chapter-control"
          onClick={(event) => {
            if (event.isTrusted) closeChapter(router, pageId, chapter.scroll);
          }}
        >
          {t.close}
        </button>
        <button
          type="button"
          className="chapter-control"
          onClick={(event) => {
            if (event.isTrusted) expandChapter(router, pageId, chapter.scroll);
          }}
        >
          {t.openFull}
        </button>
      </div>
    </section>
  );
}
