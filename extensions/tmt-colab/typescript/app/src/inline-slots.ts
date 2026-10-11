import { useEffect, useRef, useState, type RefObject } from 'react';
import type { AnchorPosition, RenderState, mountRenderer } from './renderer.js';

/** One measurement batch owns proposal and question spacers; layers cannot overwrite siblings. */
export function useInlineSlots({
  renderer,
  host,
  state,
  source,
  identity,
}: {
  renderer: RefObject<Awaited<ReturnType<typeof mountRenderer>> | null>;
  host: RefObject<HTMLDivElement | null>;
  state: RenderState | 'loading';
  source: string;
  identity: string;
}) {
  const [positions, onSlots] = useState<AnchorPosition[]>([]);
  const current = useRef({ renderer, host });
  current.current = { renderer, host };
  useEffect(() => {
    if (state !== 'ready') return;
    const nodes = [
      ...(current.current.host.current?.parentElement?.querySelectorAll<HTMLElement>(
        '[data-inline-slot]',
      ) ?? []),
    ];
    let frame: number | undefined;
    let previous = '';
    const measure = () => {
      frame = undefined;
      const heights = nodes
        .filter(
          (node, index) =>
            nodes.findIndex((other) => other.dataset.inlineSlot === node.dataset.inlineSlot) ===
            index,
        )
        .map((node) => {
          const box = node.getBoundingClientRect();
          const gap = [...node.querySelectorAll<HTMLElement>('.question-description-space')].find(
            (gap) => !gap.closest('[hidden]'),
          );
          if (!gap)
            return { id: node.dataset.inlineSlot!, height: Math.min(2000, Math.ceil(box.height)) };
          const description = gap.getBoundingClientRect();
          return {
            id: node.dataset.inlineSlot!,
            before: Math.min(2000, Math.max(0, Math.ceil(description.top - box.top))),
            height: Math.min(2000, Math.max(0, Math.ceil(box.bottom - description.bottom))),
          };
        });
      const next = JSON.stringify(heights);
      if (next !== previous) {
        previous = next;
        current.current.renderer.current?.slots(heights);
      }
    };
    const observer = new ResizeObserver(() => {
      if (frame === undefined) frame = requestAnimationFrame(measure);
    });
    nodes.forEach((node) => observer.observe(node));
    measure();
    return () => {
      observer.disconnect();
      if (frame !== undefined) cancelAnimationFrame(frame);
    };
  }, [state, source, identity]);
  return { positions, onSlots };
}
