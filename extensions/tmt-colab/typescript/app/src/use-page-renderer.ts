import { useEffect, useRef, useState, type RefObject } from 'react';
import { mountRenderer, sourceFrameKey } from './renderer.js';
import type { PageScheme, RenderState, SelectionRect } from './renderer.js';
import type { QuoteSelector } from './thread-records.js';
import { frameLead, toolbarInset } from './toolbar-inset.js';
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
  onOpenList(): void;
}

/** The page's inline layers, which the renderer reports its slots and anchors to. A new function
 * remounts the renderer, as it always has. */
export interface RendererLayers {
  onSlots: RendererOptions['onSlots'];
  onEditLocks?: RendererOptions['onEditLocks'];
  onQuestions: RendererOptions['onQuestions'];
  onReadings: RendererOptions['onReadings'];
}

/**
 * The page's frame: mounts the renderer for the page and layout, tears it down and mounts it again
 * when its frame key changes or the connection ends, and owns what the frame reports back (the page
 * scheme, which anchors resolved, whether they were checked) and the one-time reveal of the anchor
 * a shell item opened the page at. Any other source change is applied to the open frame (#5022):
 * on success `onContentApplied` runs, so the layers place their overlays again without a loading
 * state; when the frame refuses, does not answer or fails, the page mounts afresh as before. The
 * page owns `host`, `renderer` and `state`, because its layers read them too.
 */
export function usePageRenderer({
  snapshot,
  view,
  endedError,
  host,
  renderer,
  setState,
  toolbar,
  titleRow,
  anchor,
  layers,
  handlers,
  resetPageLinks,
  requestPageLink,
  onContentApplied,
}: {
  snapshot: PageSnapshot;
  view: PageView;
  endedError: unknown;
  host: RefObject<HTMLDivElement | null>;
  renderer: RefObject<Awaited<ReturnType<typeof mountRenderer>> | null>;
  setState(state: RenderState | 'loading'): void;
  toolbar: RefObject<HTMLElement | null>;
  /** The page's title row when it is content under the shared bar: it takes room from a frame that
   * fills the viewport, but not from where a revealed element lands. */
  titleRow: RefObject<HTMLElement | null>;
  /** The element a shell item opened the page at. */
  anchor: string | undefined;
  layers: RendererLayers;
  handlers: RendererHandlers;
  resetPageLinks(): void;
  requestPageLink: RendererOptions['onLink'];
  /** The open frame now shows a new source, applied in place. */
  onContentApplied(): void;
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
  // The frame is mounted again only when its key changes (#5022): the page, its layout and type,
  // and the part the source decides (whether it may hold scripts, whether it uses the design
  // system). `remounts` counts the in-place updates that fell back to a fresh frame.
  const frameKey = sourceFrameKey(view.source);
  const [remounts, setRemounts] = useState(0);
  const latestSource = useRef(view.source);
  latestSource.current = view.source;
  // The source the open frame shows, or is being mounted or updated with.
  const frameSource = useRef<string | undefined>(undefined);
  const contentApplied = useRef(onContentApplied);
  contentApplied.current = onContentApplied;

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

  const { onSlots, onQuestions, onReadings, onEditLocks } = layers;
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
    frameSource.current = latestSource.current;
    void mountRenderer(host.current!, latestSource.current, {
      signal: controller.signal,
      layout: view.layout,
      onState: (next) => {
        if (next !== 'ready') resetPageLinks();
        setState(next);
      },
      onScheme: setPageScheme,
      onLink: requestPageLink,
      onSlots,
      onEditLocks,
      onQuestions,
      onReadings,
      viewportInset: () => toolbarInset(toolbar.current),
      frameLead: () => frameLead(titleRow.current),
      onSelection: (_value, quote, rect) => current.current.onSelection(quote, rect),
      onAnchors: (ids, checked = false) => {
        setResolved(ids);
        setAnchorsChecked(checked);
      },
      onAnnotate: () => current.current.onAnnotate(),
      onToggleSelectionMode: () => current.current.onToggleSelectionMode(),
      onKey: (key) => current.current.onKey(key),
      onOpenThread: (id) => current.current.onOpenThread(id),
      onOpenList: () => current.current.onOpenList(),
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
      frameSource.current = undefined;
    };
  }, [
    snapshot.id,
    frameKey,
    remounts,
    view.layout,
    view.pageType,
    endedError,
    host,
    renderer,
    setState,
    toolbar,
    titleRow,
    onSlots,
    onEditLocks,
    onQuestions,
    onReadings,
    resetPageLinks,
    requestPageLink,
  ]);

  // Declared after the mount effect, so in a commit that also remounts, the frame being mounted
  // already holds this source and nothing is sent.
  useEffect(() => {
    const source = view.source;
    if (frameSource.current === undefined || frameSource.current === source) return;
    const handle = renderer.current;
    frameSource.current = source;
    // A frame still being created was given an older source: mount it again with this one.
    if (!handle) {
      setRemounts((count) => count + 1);
      return;
    }
    void handle.update(source).then((result) => {
      if (renderer.current !== handle || result === 'superseded') return;
      if (result === 'applied') contentApplied.current();
      else setRemounts((count) => count + 1);
    });
  }, [view.source, renderer]);

  return { resolved, anchorsChecked, pageScheme, missingAnchor };
}
