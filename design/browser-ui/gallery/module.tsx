// The gallery as a route module of the one TMT web app (dev preview only; the product has no Design
// route). It mounts into the shell's main area and gives the sections to the module menu, so the
// gallery looks and moves like Colab or Remote inside the same shell, with no reload.
import { StrictMode } from 'react';
import { flushSync } from 'react-dom';
import { createRoot } from 'react-dom/client';
import uiCss from '../generated/ui.css?inline';
import type { ShellMenu } from '../src/shell.js';
import type { ShellSlot } from '../src/shell-host.js';
import galleryCss from './gallery.css?inline';
import { BrowserHeader } from '../src/header.js';
import { sections, type SectionId } from './covered.js';
import { refreshForcedStates } from './force-states.js';
import { GallerySections } from './sections.js';
import { GalleryOutline, OutlineMenu, readOutline, type OutlineEntry } from './outline.js';

/** The outline sits beside the page from this width; below it, it is one dropdown in the header. */
const WIDE = '(min-width: 1280px)';
/** A heading this far below the sticky header already counts as the one in view. */
const SPY_OFFSET = 24;

export function mount(slot: ShellSlot): () => void {
  // The gallery's own rules (host metrics, demo chrome) and the UI layer's utilities live only
  // while this route is mounted.
  const style = document.createElement('style');
  style.dataset.galleryModule = '';
  style.textContent = `${uiCss}\n${galleryCss}`;
  document.head.append(style);

  const container = document.createElement('div');
  container.className = 'gallery gallery-embedded';
  const main = document.createElement('main');
  main.className = 'gallery-main';
  const aside = document.createElement('div');
  container.append(main, aside);
  slot.main.replaceChildren(container);
  slot.setTitle('Design system');

  const href = (id: string) => `${slot.basePath}#${id}`;
  const wide = matchMedia(WIDE);
  const calm = matchMedia('(prefers-reduced-motion: reduce)');
  let entries: OutlineEntry[] = [];
  let currentId: string =
    sections.find(([id]) => id === location.hash.slice(1))?.[0] ?? sections[0][0];
  const sectionOf = (id: string): SectionId =>
    (entries.find((entry) => entry.id === id)?.section ??
      sections.find(([key]) => key === id)?.[0] ??
      sections[0][0]) as SectionId;
  const menu = (): ShellMenu => ({
    title: 'Design',
    groups: [
      {
        items: sections.map(([id, label]) => ({
          id,
          label,
          href: href(id),
          current: id === sectionOf(currentId),
        })),
      },
    ],
  });
  const show = (id: string) => {
    if (id === currentId) return;
    const before = sectionOf(currentId);
    currentId = id;
    if (sectionOf(id) !== before) slot.setMenu(menu(), select);
    paint();
  };
  // A click on the menu or the outline moves to the entry, keeps the address and follows it.
  const go = (id: string, smooth: boolean): boolean => {
    const target = main.querySelector<HTMLElement>(`#${CSS.escape(id)}`);
    if (!target || !(sections.some(([key]) => key === id) || entries.some((e) => e.id === id)))
      return false;
    history.replaceState(null, '', href(id));
    // The shell repaints its menu when the section changes; scroll after that, so a repaint that
    // resets the page cannot undo the move.
    show(id);
    target.scrollIntoView({
      block: 'start',
      behavior: smooth && !calm.matches ? 'smooth' : 'auto',
    });
    return true;
  };
  // The menu passes its href as a second argument; only the entry matters here.
  const select = (id: string) => go(id, true);
  slot.setMenu(menu(), select);

  // The module's one header in the shell bar: the shell owns the TMT mark, so the label is the
  // module (TMT · Design), the title is the page, and the section in view follows it.
  const bar = createRoot(slot.bar);
  const outline = createRoot(aside);
  const paint = () => {
    const section = sections.find(([id]) => id === sectionOf(currentId))![1];
    bar.render(
      <BrowserHeader
        productLabel="Design"
        headingPrefix={
          <span className="gallery-title-divider" aria-hidden="true">
            /
          </span>
        }
        title="Design system"
        headingSuffix={<span>· {section}</span>}
        actions={
          !wide.matches && entries.length ? (
            <OutlineMenu entries={entries} current={currentId} onSelect={select} />
          ) : undefined
        }
      />,
    );
    outline.render(
      wide.matches && entries.length ? (
        <GalleryOutline entries={entries} current={currentId} onSelect={select} />
      ) : null,
    );
  };
  wide.addEventListener('change', paint);
  paint();
  const root = createRoot(main);
  // The outline reads the rendered sections, so they are committed before anything follows them.
  flushSync(() =>
    root.render(
      <StrictMode>
        <GallerySections />
      </StrictMode>,
    ),
  );
  // Read the outline, pin the forced states and honour the address once the sections exist. The
  // current entry is the last one whose top has reached the line just under the sticky header (a
  // heading a little below that line already counts), so the highlight and the heading in view
  // agree.
  const elements = new Map<string, HTMLElement>();
  let queued = false;
  const follow = () => {
    queued = false;
    const line = slot.bar.getBoundingClientRect().bottom + SPY_OFFSET;
    let found = entries[0];
    for (const entry of entries) {
      if (elements.get(entry.id)!.getBoundingClientRect().top > line) break;
      found = entry;
    }
    if (found) show(found.id);
  };
  const onScroll = () => {
    if (queued) return;
    queued = true;
    requestAnimationFrame(follow);
  };
  document.addEventListener('scroll', onScroll, { capture: true, passive: true });
  const frame = requestAnimationFrame(() => {
    refreshForcedStates(document);
    entries = readOutline(main);
    for (const entry of entries)
      elements.set(entry.id, main.querySelector<HTMLElement>(`#${CSS.escape(entry.id)}`)!);
    slot.setMenu(menu(), select);
    paint();
    if (location.hash.length > 1) go(location.hash.slice(1), false);
    else follow();
  });

  let mounted = true;
  if (import.meta.hot)
    import.meta.hot.on('vite:afterUpdate', () => mounted && refreshForcedStates(document));

  return () => {
    mounted = false;
    cancelAnimationFrame(frame);
    document.removeEventListener('scroll', onScroll, { capture: true });
    wide.removeEventListener('change', paint);
    root.unmount();
    outline.unmount();
    bar.unmount();
    style.remove();
  };
}
