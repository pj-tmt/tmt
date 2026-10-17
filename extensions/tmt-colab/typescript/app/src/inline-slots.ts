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
    const scope = current.current.host.current?.parentElement;
    const nodes = [...(scope?.querySelectorAll<HTMLElement>('[data-inline-slot]') ?? [])];
    // A chip-form group is two nodes: the chip the frame leaves a placeholder for, and the panel.
    const chips = new Map(
      [...(scope?.querySelectorAll<HTMLElement>('[data-inline-chip]') ?? [])].map((node) => [
        node.dataset.inlineChip!,
        node,
      ]),
    );
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
          const chip = chips.get(node.dataset.inlineSlot!)?.getBoundingClientRect();
          if (chip)
            return {
              id: node.dataset.inlineSlot!,
              height: Math.min(2000, Math.ceil(box.height)),
              chip: { width: Math.ceil(chip.width), height: Math.ceil(chip.height) },
            };
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
    chips.forEach((node) => observer.observe(node));
    measure();
    return () => {
      observer.disconnect();
      if (frame !== undefined) cancelAnimationFrame(frame);
    };
  }, [state, source, identity]);
  return { positions, onSlots };
}
