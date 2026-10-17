/** Isolated geometry producer for the real inline-slot hook; no renderer or publication authority. */
import { createRoot, type Root } from 'react-dom/client';
import { useRef, type RefObject } from 'react';
import { useInlineSlots } from '../src/inline-slots.js';
import type { InlineSlot, mountRenderer } from '../src/renderer.js';
let root: Root | undefined;
let batches: InlineSlot[][] = [];
const id = (index: number) => `10000000-0000-4000-8000-${String(index).padStart(12, '0')}`;
function Probe({
  occupied,
  readings,
  collide = false,
}: {
  occupied: number;
  readings: number;
  collide?: boolean;
}) {
  const host = useRef<HTMLDivElement>(null);
  const renderer = useRef({ slots: (slots: InlineSlot[]) => batches.push(structuredClone(slots)) });
  const { readingIds } = useInlineSlots({
    host,
    renderer: renderer as unknown as RefObject<Awaited<ReturnType<typeof mountRenderer>> | null>,
    state: 'ready',
    source: '',
    identity: `${occupied}:${readings}`,
  });
  return (
    <div>
      <div ref={host} />
      <output data-selected-readings>{[...readingIds].join(',')}</output>
      {Array.from({ length: occupied }, (_, index) => (
        <div key={id(index)} data-inline-slot={id(index)} style={{ height: 2 }} />
      ))}
      {Array.from({ length: readings }, (_, index) => (
        <div
          key={`reading-${index}`}
          data-inline-slot={id(collide ? index : 1000 + index)}
          data-inline-reading=""
          style={{ height: 2 }}
        />
      ))}
    </div>
  );
}
export function mount(options: { occupied: number; readings: number; collide?: boolean }) {
  root?.unmount();
  batches = [];
  const host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
  root.render(<Probe {...options} />);
}
export function proof() {
  return batches.at(-1);
}
