/** Isolated geometry producer for the real inline-slot hook; no renderer or publication authority. */
import { createRoot, type Root } from 'react-dom/client';
import { useEffect, useRef, useState, type RefObject } from 'react';
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
    contentRevision: 0,
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

let visibility: ((hidden: boolean) => void) | undefined;
function VisibilityProbe({ initiallyHidden }: { initiallyHidden: boolean }) {
  const host = useRef<HTMLDivElement>(null);
  const [hidden, setHidden] = useState(initiallyHidden);
  useEffect(() => {
    visibility = setHidden;
    return () => {
      visibility = undefined;
    };
  }, []);
  const renderer = useRef({ slots: (slots: InlineSlot[]) => batches.push(structuredClone(slots)) });
  const { readingIds } = useInlineSlots({
    host,
    renderer: renderer as unknown as RefObject<Awaited<ReturnType<typeof mountRenderer>> | null>,
    state: 'ready',
    contentRevision: 0,
    source: '',
    identity: 'visibility',
  });
  const style = { display: hidden ? 'none' : undefined };
  return (
    <div>
      <div ref={host} />
      <output data-selected-readings>{[...readingIds].join(',')}</output>
      <div data-inline-slot={id(1)} style={style}>
        <input aria-label="Ordinary draft" defaultValue="" />
      </div>
      <div data-inline-slot={id(2)} style={style}>
        <div style={{ height: 12 }} />
        <div className="question-description-space" style={{ height: 20 }} />
        <input aria-label="Answer draft" defaultValue="" />
      </div>
      <div data-inline-slot={id(3)} style={style}>
        <span data-inline-chip={id(3)} style={{ display: 'block', width: 80, height: 16 }} />
      </div>
      <div data-inline-slot={id(4)} data-inline-reading="" style={{ ...style, height: 32 }} />
    </div>
  );
}
export function mountVisibility(initiallyHidden = false) {
  root?.unmount();
  batches = [];
  const host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
  root.render(<VisibilityProbe initiallyHidden={initiallyHidden} />);
}
export function setVisibility(hidden: boolean) {
  visibility?.(hidden);
}
export function batchCount() {
  return batches.length;
}
