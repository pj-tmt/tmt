// The gallery as a route module of the one TMT web app (dev preview only; the product has no Design
// route). It mounts into the shell's main area and gives the sections to the module menu, so the
// gallery looks and moves like Colab or Remote inside the same shell, with no reload.
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import uiCss from '../generated/ui.css?inline';
import type { ShellMenu } from '../src/shell.js';
import type { ShellSlot } from '../src/shell-host.js';
import galleryCss from './gallery.css?inline';
import { BrowserHeader } from '../src/header.js';
import { sections, type SectionId } from './covered.js';
import { refreshForcedStates } from './force-states.js';
import { GallerySections } from './sections.js';

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
  container.append(main);
  slot.main.replaceChildren(container);
  slot.setTitle('Design gallery');

  const href = (id: string) => `${slot.basePath}#${id}`;
  let current: SectionId =
    sections.find(([id]) => id === location.hash.slice(1))?.[0] ?? sections[0][0];
  const menu = (): ShellMenu => ({
    title: 'Design',
    groups: [
      {
        items: sections.map(([id, label]) => ({
          id,
          label,
          href: href(id),
          current: id === current,
        })),
      },
    ],
  });
  const show = (id: SectionId) => {
    if (id === current) return;
    current = id;
    slot.setMenu(menu(), select);
    paintBar();
  };
  const select = (id: string): boolean => {
    const known = sections.find(([key]) => key === id);
    if (!known) return false;
    const target = main.querySelector<HTMLElement>(`#${CSS.escape(id)}`);
    if (!target) return false;
    history.replaceState(null, '', href(id));
    target.scrollIntoView({ block: 'start' });
    show(known[0]);
    return true;
  };
  slot.setMenu(menu(), select);

  // The module's one header in the shell bar: the shell owns the TMT mark, so the label is the
  // module (TMT · Design) and the title is the section in view.
  const bar = createRoot(slot.bar);
  const paintBar = () =>
    bar.render(
      <BrowserHeader
        productLabel="Design"
        headingPrefix={
          <span className="gallery-title-divider" aria-hidden="true">
            /
          </span>
        }
        title={sections.find(([id]) => id === current)![1]}
      />,
    );
  paintBar();
  const root = createRoot(main);
  root.render(
    <StrictMode>
      <GallerySections />
    </StrictMode>,
  );
  // The sections exist only after React's first commit: pin the forced states, honour the address
  // and start following the scroll then. The section nearest the top is the current menu item.
  const visible = new Set<string>();
  const observer = new IntersectionObserver(
    (entries) => {
      for (const entry of entries)
        if (entry.isIntersecting) visible.add(entry.target.id);
        else visible.delete(entry.target.id);
      const top = sections.find(([id]) => visible.has(id));
      if (top) show(top[0]);
    },
    { rootMargin: '-10% 0px -80% 0px' },
  );
  const frame = requestAnimationFrame(() => {
    refreshForcedStates(document);
    for (const [id] of sections) {
      const target = main.querySelector(`#${CSS.escape(id)}`);
      if (target) observer.observe(target);
    }
    if (location.hash.length > 1) select(location.hash.slice(1));
  });

  let mounted = true;
  if (import.meta.hot)
    import.meta.hot.on('vite:afterUpdate', () => mounted && refreshForcedStates(document));

  return () => {
    mounted = false;
    cancelAnimationFrame(frame);
    observer.disconnect();
    root.unmount();
    bar.unmount();
    style.remove();
  };
}
