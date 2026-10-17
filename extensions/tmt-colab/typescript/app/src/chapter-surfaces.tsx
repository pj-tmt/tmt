import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useSyncExternalStore,
  useState,
  type ReactNode,
} from 'react';
import { createPortal } from 'react-dom';
import { ChapterPassages } from './chapter-passages.js';
import { text } from './strings.js';
import { ChapterSurface } from './chapter-surface.js';
import type { PageTransport } from './transport.js';

interface Slot {
  node?: HTMLDivElement;
  container: HTMLDivElement;
  open(): void;
  url: string;
}
interface ChapterSlots {
  show(generation: object): () => void;
  order(order: readonly string[]): void;
  attach(id: string, node: HTMLDivElement | null, open: () => void, url: string): void;
}
const Context = createContext<ChapterSlots | undefined>(undefined);
const emptyCounts: ReadonlyMap<string, number> = new Map();
const Counts = createContext(emptyCounts);
export const useChapterCounts = () => useContext(Counts);

/** This owner sits above route replacement. Only the three admitted surfaces retain sources;
 * a hidden surface with an in-flight action parks its parent controls until that action joins. */
export function ChapterSurfaces({
  transport,
  children,
}: {
  transport: PageTransport;
  children: ReactNode;
}) {
  const parking = useRef<HTMLDivElement>(null);
  const generation = useRef<object | undefined>(undefined);
  const [counts, setCounts] = useState<ReadonlyMap<string, number>>(emptyCounts);
  const slots = useRef(new Map<string, Slot>());
  const order = useRef<readonly string[]>([]);
  const shown = useRef(false);
  const intersecting = useRef(new Set<string>());
  const observer = useRef<IntersectionObserver | null>(null);
  const passages = useMemo(
    () =>
      new ChapterPassages((id, signal) => {
        if (!transport.chapter) throw new Error('Chapter previews unavailable');
        return transport.chapter(id, signal);
      }),
    [transport],
  );
  const views = useSyncExternalStore(passages.subscribe, () => passages.snapshot);
  const refresh = useCallback(() => {
    passages.update(
      order.current,
      intersecting.current,
      shown.current && document.visibilityState === 'visible',
    );
  }, [passages]);
  const context = useMemo<ChapterSlots>(
    () => ({
      show(current) {
        if (generation.current !== current) {
          generation.current = current;
          setCounts(new Map());
        }
        shown.current = true;
        refresh();
        return () => {
          shown.current = false;
          refresh();
        };
      },
      order(next) {
        order.current = next;
        refresh();
      },
      attach(id, node, open, url) {
        let slot = slots.current.get(id);
        if (!slot) {
          slot = { container: document.createElement('div'), open, url };
          slots.current.set(id, slot);
        }
        if (slot.node !== node) {
          if (slot.node) observer.current?.unobserve(slot.node);
          // Retire the frame before moving its container: reparenting an iframe navigates it.
          intersecting.current.delete(id);
          refresh();
          if (!node) passages.detach(id);
          slot.node = node ?? undefined;
          if (node) {
            node.append(slot.container);
            observer.current?.observe(node);
          } else parking.current?.append(slot.container);
        }
        slot.open = open;
        slot.url = url;
      },
    }),
    [refresh, passages],
  );
  useEffect(() => {
    observer.current = new IntersectionObserver((entries) => {
      for (const entry of entries) {
        const id = (entry.target as HTMLElement).dataset.chapterSlot;
        if (!id) continue;
        if (entry.isIntersecting) intersecting.current.add(id);
        else intersecting.current.delete(id);
      }
      refresh();
    });
    for (const slot of slots.current.values()) if (slot.node) observer.current.observe(slot.node);
    document.addEventListener('visibilitychange', refresh);
    return () => {
      document.removeEventListener('visibilitychange', refresh);
      observer.current?.disconnect();
      observer.current = null;
      passages.update([], new Set());
    };
  }, [passages, refresh]);
  useEffect(() => {
    for (const [id, slot] of slots.current) {
      if (!slot.node && !views.some((view) => view.id === id)) {
        slot.container.remove();
        slots.current.delete(id);
      }
    }
  }, [views]);
  return (
    <Context.Provider value={transport.chapter ? context : undefined}>
      <Counts.Provider value={counts}>
        {children}
        <div ref={parking} hidden aria-hidden="true" />
        {views.map((view) => {
          const slot = slots.current.get(view.id);
          if (!slot) return null;
          return createPortal(
            view.state === 'ready' ? (
              <ChapterSurface
                snapshot={view.snapshot}
                transport={transport}
                passages={passages}
                visible={view.visible}
                busy={!!view.busy}
                detached={!!view.detached}
                open={slot.open}
                url={slot.url}
                onCount={(count) =>
                  setCounts((previous) =>
                    previous.get(view.id) === count
                      ? previous
                      : new Map(previous).set(view.id, count),
                  )
                }
              />
            ) : view.state === 'unavailable' && view.visible ? (
              <p className="chapter-passage-unavailable">
                {text.today.passage.unavailable} ·{' '}
                <button type="button" onClick={slot.open}>
                  {text.today.passage.open}
                </button>
              </p>
            ) : null,
            slot.container,
            view.id,
          );
        })}
      </Counts.Provider>
    </Context.Provider>
  );
}

/** Today supplies chapter order; IntersectionObserver, never authored HTML, admits a surface. */
export function useChapterSurfaces(order: readonly string[], generation: object, enabled = true) {
  const context = useContext(Context);
  const key = order.join(',');
  const ids = useMemo(() => (key ? key.split(',') : []), [key]);
  useEffect(() => {
    if (enabled) context?.order(ids);
  }, [context, ids, enabled]);
  useEffect(
    () => (enabled ? context?.show(generation) : undefined),
    [context, generation, enabled],
  );
}

export function ChapterSurfaceSlot({ id, open, url }: { id: string; open(): void; url: string }) {
  const context = useContext(Context);
  const ref = useCallback(
    (node: HTMLDivElement | null) => context?.attach(id, node, open, url),
    [context, id, open, url],
  );
  return context ? (
    <div data-chapter-slot={id} ref={ref} style={{ minHeight: 1, width: '100%' }} />
  ) : null;
}
