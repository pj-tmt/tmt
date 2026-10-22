import { useEffect, useRef, useState, type RefObject } from 'react';
import { mountRenderer } from './renderer.js';
import type { PageScheme, RenderState, SelectionRect } from './renderer.js';
import type { QuoteSelector } from './thread-records.js';
import { toolbarInset } from './toolbar-inset.js';
import type { PageSnapshot, PageView } from './transport.js';

type RendererOptions = Parameters<typeof mountRenderer>[2];

/** What the page does when its renderer reports something. The page passes the newest handlers on
 * every render; the renderer, mounted once per source, calls whichever are current. */
export interface RendererHandlers {
  onSelection(
    quote: QuoteSelector | null | undefined,
    rect: SelectionRect | null | undefined,
  ): void;
  onAnnotate(): void;
  onToggleSelectionMode(): void;
  onKey(key: Parameters<NonNullable<RendererOptions['onKey']>>[0]): void;
  onOpenThread(id: string): void;
}

/** The page's inline layers, which the renderer reports its slots and anchors to. A new function
 * remounts the renderer, as it always has. */
export interface RendererLayers {
  onSlots: RendererOptions['onSlots'];
  onQuestions: RendererOptions['onQuestions'];
  onReadings: RendererOptions['onReadings'];
}

/**
 * The page's frame: mounts the renderer for the page source and layout, tears it down and mounts
 * it again when they change or the connection ends, and owns what the frame reports back (the page
 * scheme, which anchors resolved, whether they were checked) and the one-time reveal of the anchor
 * a shell item opened the page at. The page owns `host`, `renderer` and `state`, because its layers
 * read them too.
 */
export function usePageRenderer({
  snapshot,
  view,
  endedError,
  host,
  renderer,
  setState,
  toolbar,
  anchor,
  layers,
  handlers,
  resetPageLinks,
  requestPageLink,
}: {
  snapshot: PageSnapshot;
  view: PageView;
  endedError: unknown;
  host: RefObject<HTMLDivElement | null>;
  renderer: RefObject<Awaited<ReturnType<typeof mountRenderer>> | null>;
  setState(state: RenderState | 'loading'): void;
  toolbar: RefObject<HTMLElement | null>;
  /** The element a shell item opened the page at. */
  anchor: string | undefined;
  layers: RendererLayers;
  handlers: RendererHandlers;
  resetPageLinks(): void;
  requestPageLink: RendererOptions['onLink'];
}) {
  const [resolved, setResolved] = useState<string[]>([]);
  const [anchorsChecked, setAnchorsChecked] = useState(false);
  // Until the frame reports what its page paints, the cards keep the app theme.
  const [pageScheme, setPageScheme] = useState<PageScheme>();
  // A shell item can open the page at one exact element; when it is not there, say so once.
  const [missingAnchor, setMissingAnchor] = useState<string>();
  const revealed = useRef('');
  const [rendererMounts, setRendererMounts] = useState(0);
  const current = useRef(handlers);
  current.current = handlers;

  const imageBinding = snapshot.binding?.files?.attachments;
  const imageScope = imageBinding?.scope?.();
  const imageDescriptors = JSON.stringify(view.attachments ?? []);
  const imageInput = useRef({ binding: imageBinding, descriptors: view.attachments ?? [] });
  imageInput.current = { binding: imageBinding, descriptors: view.attachments ?? [] };
  useEffect(() => {
    renderer.current?.images(imageInput.current);
  }, [renderer, imageBinding, imageScope, imageDescriptors]);

  useEffect(() => {
    setResolved([]);
    setAnchorsChecked(false);
  }, [snapshot.id]);

  useEffect(() => {
    const key = anchor ? `${snapshot.id}:${anchor}` : '';
    if (!anchor || !renderer.current || revealed.current === key) return;
    // Once per opening: a later live edit of the page never pulls the reader back to it.
    revealed.current = key;
    setMissingAnchor(undefined);
    void renderer.current.reveal(anchor).then((found) => {
      if (found === false) setMissingAnchor(anchor);
    });
  }, [anchor, renderer, rendererMounts, snapshot.id]);
  useEffect(() => {
    if (!anchor) setMissingAnchor(undefined);
  }, [anchor]);

  const { onSlots, onQuestions, onReadings } = layers;
  useEffect(() => {
    const controller = new AbortController();
    if (view.pageType === 'shell') {
      host.current?.replaceChildren();
      setState('ready');
      return () => controller.abort();
    }
    if (endedError) {
      host.current?.replaceChildren();
      setState('failed');
      return () => controller.abort();
    }
    setState('loading');
    resetPageLinks();
    // Source revisions replace only author content. Parent selection and composer
    // state keep the original quote and rectangle even if that quote is now stale.
    setResolved([]);
    setAnchorsChecked(false);
    setPageScheme(undefined);
    void mountRenderer(host.current!, view.source, {
      signal: controller.signal,
      layout: view.layout,
      onState: (next) => {
        if (next !== 'ready') resetPageLinks();
        setState(next);
      },
      onScheme: setPageScheme,
      onLink: requestPageLink,
      onSlots,
      onQuestions,
      onReadings,
      viewportInset: () => toolbarInset(toolbar.current),
      onSelection: (_value, quote, rect) => current.current.onSelection(quote, rect),
      onAnchors: (ids, checked = false) => {
        setResolved(ids);
        setAnchorsChecked(checked);
      },
      onAnnotate: () => current.current.onAnnotate(),
      onToggleSelectionMode: () => current.current.onToggleSelectionMode(),
      onKey: (key) => current.current.onKey(key),
      onOpenThread: (id) => current.current.onOpenThread(id),
    })
      .then((handle) => {
        if (controller.signal.aborted) handle.destroy();
        else {
          renderer.current = handle;
          handle.images(imageInput.current);
          setRendererMounts((count) => count + 1);
        }
      })
      .catch(() => {
        if (!controller.signal.aborted) {
          host.current?.replaceChildren();
          setState('failed');
        }
      });
    return () => {
      // Retire messages before replacing the frame, retaining its layout and the
      // parent's frozen selection. Unmount removes the host itself.
      renderer.current?.release();
      resetPageLinks();
      controller.abort();
      renderer.current = null;
    };
  }, [
    snapshot.id,
    view.source,
    view.layout,
    view.pageType,
    endedError,
    host,
    renderer,
    setState,
    toolbar,
    onSlots,
    onQuestions,
    onReadings,
    resetPageLinks,
    requestPageLink,
  ]);

  return { resolved, anchorsChecked, pageScheme, missingAnchor };
}
