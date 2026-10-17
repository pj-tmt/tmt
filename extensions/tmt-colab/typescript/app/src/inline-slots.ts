import { useEffect, useRef, useState, type RefObject } from 'react';
import {
  INLINE_SLOT_LIMIT,
  type AnchorPosition,
  type RenderState,
  type mountRenderer,
} from './renderer.js';

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
  const [readingIds, setReadingIds] = useState<ReadonlySet<string>>(new Set());
  const current = useRef({ renderer, host });
  current.current = { renderer, host };
  useEffect(() => {
    if (state !== 'ready') return;
    const scope = current.current.host.current?.parentElement;
    const nodes = [...(scope?.querySelectorAll<HTMLElement>('[data-inline-slot]') ?? [])];
    // A chip-form group keeps its separate measured chip and panel.
    const chips = new Map(
      [...(scope?.querySelectorAll<HTMLElement>('[data-inline-chip]') ?? [])].map((node) => [
        node.dataset.inlineChip!,
        node,
      ]),
    );
    // Preserve every existing control and its order. New reading bands can use only the
    // remaining budget, never a colliding ID or a slot belonging to a sibling layer.
    const ordinary = nodes.filter(
      (node, index) =>
        !node.hasAttribute('data-inline-reading') &&
        nodes.findIndex(
          (other) =>
            !other.hasAttribute('data-inline-reading') &&
            other.dataset.inlineSlot === node.dataset.inlineSlot,
        ) === index,
    );
    const occupied = new Set(ordinary.map((node) => node.dataset.inlineSlot));
    const reading = nodes
      .filter(
        (node, index) =>
          node.hasAttribute('data-inline-reading') &&
          !occupied.has(node.dataset.inlineSlot) &&
          nodes.findIndex(
            (other) =>
              other.hasAttribute('data-inline-reading') &&
              other.dataset.inlineSlot === node.dataset.inlineSlot,
          ) === index,
      )
      .slice(0, Math.max(0, INLINE_SLOT_LIMIT - ordinary.length));
    const admitted = [...ordinary, ...reading];
    const ids = reading.map((node) => node.dataset.inlineSlot!);
    setReadingIds((previous) =>
      previous.size === ids.length && ids.every((id) => previous.has(id)) ? previous : new Set(ids),
    );
    let frame: number | undefined;
    let previous = '';
    const measure = () => {
      frame = undefined;
      const heights = admitted.map((node) => {
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
  return { positions, onSlots, readingIds };
}
