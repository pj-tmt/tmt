import { useEffect, type RefObject } from 'react';

/**
 * Marks the host `data-edge-start` / `data-edge-end` while the tab strip inside it scrolls on its own
 * and hides tabs before / after the visible part, so the page can fade that edge. `key` measures
 * again when the tabs change.
 */
export function useScrollEdges(host: RefObject<HTMLElement | null>, key: string) {
  useEffect(() => {
    const node = host.current;
    const strip = node?.querySelector<HTMLElement>('[role="tablist"]');
    if (!node || !strip) return;
    const measure = () => {
      const hidden = strip.scrollWidth - strip.clientWidth > 1;
      node.toggleAttribute('data-edge-start', hidden && strip.scrollLeft > 1);
      node.toggleAttribute(
        'data-edge-end',
        hidden && strip.scrollLeft + strip.clientWidth < strip.scrollWidth - 1,
      );
    };
    measure();
    strip.addEventListener('scroll', measure, { passive: true });
    // A tab that grows (its count changes) or a resize moves the edges without a scroll event.
    const observer = new ResizeObserver(measure);
    observer.observe(strip);
    for (const tab of Array.from(strip.children)) observer.observe(tab);
    return () => {
      strip.removeEventListener('scroll', measure);
      observer.disconnect();
      node.removeAttribute('data-edge-start');
      node.removeAttribute('data-edge-end');
    };
  }, [host, key]);
}
