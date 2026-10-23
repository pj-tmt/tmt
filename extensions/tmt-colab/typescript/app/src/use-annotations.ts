import { useCallback, useEffect, useRef, useState, type RefObject } from 'react';
import type { ComposerEdit } from './components/message-composer-edit.js';
import { DraftSession } from './draft-store.js';
import { usePageEnvironment } from './page-environment.js';
import type { mountRenderer } from './renderer.js';
import type { SelectionRect } from './renderer.js';
import type { DiscussionRef, QuoteSelector } from './thread-records.js';
import type { PageBinding, PageSnapshot, PageTransport } from './transport.js';

/** What the annotation window needs from the page's panels: it closes and selects threads. */
interface AnnotationPanels {
  clear(): void;
  showThread(id: string): void;
  setActiveThread(id: string | null): void;
}

/**
 * The page's selection and annotation: selection mode, the quote selected and where, the open
 * annotation window (a new one on a selection, or one on a thread), the unsent drafts kept per
 * quote or thread and their device-local session, and the actions that open, continue and close
 * them. Everything here clears when the page changes.
 */
export function useAnnotations({
  snapshot,
  transport,
  binding,
  renderer,
  host,
  panels,
}: {
  snapshot: PageSnapshot;
  transport: PageTransport;
  binding: RefObject<PageBinding | undefined>;
  renderer: RefObject<Awaited<ReturnType<typeof mountRenderer>> | null>;
  host: RefObject<HTMLDivElement | null>;
  panels: AnnotationPanels;
}) {
  const environment = usePageEnvironment();
  // The renderer's thread handler outlives a render, so it reads the environment and the panels
  // through refs.
  const environmentRef = useRef(environment);
  environmentRef.current = environment;
  const panelsRef = useRef(panels);
  panelsRef.current = panels;
  const [selectionMode, setSelectionMode] = useState(false);
  const [selector, setSelector] = useState<QuoteSelector | null>(null);
  const currentSelector = useRef<QuoteSelector | null>(null);
  const [rectangle, setRectangle] = useState<SelectionRect | null>(null);
  const currentRectangle = useRef<SelectionRect | null>(null);
  const [annotation, setAnnotation] = useState<{
    key: string;
    selector: QuoteSelector | null;
    rectangle: SelectionRect;
    thread?: DiscussionRef;
    /** Text kept from an earlier close of this selection or thread. */
    restored?: ComposerEdit;
  }>();
  const annotationRef = useRef(annotation);
  annotationRef.current = annotation;
  const popover = useRef<HTMLElement>(null);
  // An unsent draft lives in memory for this page only: never stored, never sent to the frame.
  const drafts = useRef(new Map<string, ComposerEdit>());
  // Device-local persistence behind `drafts`: it restores text only and never sends or opens.
  const session = useRef<DraftSession | undefined>(undefined);
  const [draftsReady, setDraftsReady] = useState(!transport.drafts);
  const [draftsUnsaved, setDraftsUnsaved] = useState(false);
  const [, setDraftKeys] = useState(0);
  function keepDraft(key: string, edit: ComposerEdit | null) {
    const had = drafts.current.has(key);
    if (edit) drafts.current.set(key, edit);
    else drafts.current.delete(key);
    session.current?.set(key, edit);
    if (had !== !!edit) setDraftKeys((n) => n + 1);
  }
  const annotationDraft = useRef<ComposerEdit>({ value: '' });
  const annotationBusy = useRef(false);
  const statusBusy = useRef(false);
  const [changingStatus, setChangingStatus] = useState(false);
  useEffect(() => {
    setAnnotation(undefined);
    drafts.current.clear();
    annotationDraft.current = { value: '' };
    annotationBusy.current = statusBusy.current = false;
    setChangingStatus(false);
    setSelectionMode(false);
    setSelector(null);
    currentSelector.current = null;
    currentRectangle.current = null;
    setRectangle(null);
  }, [snapshot.id]);
  useEffect(() => {
    const store = transport.drafts;
    setDraftsUnsaved(false);
    if (!store) {
      setDraftsReady(true);
      return;
    }
    setDraftsReady(false);
    let live = true;
    // Only the live session presents: a write that settles after leaving never sets the notice.
    const current = new DraftSession(store, snapshot.id, (unsaved) => {
      if (live) setDraftsUnsaved(unsaved);
    });
    session.current = current;
    void current.load().then((saved) => {
      if (!live) return;
      // Text typed before the read finished is newer than anything stored.
      for (const [key, edit] of saved ?? [])
        if (!drafts.current.has(key)) drafts.current.set(key, edit);
      setDraftKeys((n) => n + 1);
      setDraftsReady(true);
    });
    const leave = () => void current.flush();
    const hide = () => document.visibilityState === 'hidden' && leave();
    window.addEventListener('pagehide', leave);
    document.addEventListener('visibilitychange', hide);
    return () => {
      live = false;
      window.removeEventListener('pagehide', leave);
      document.removeEventListener('visibilitychange', hide);
      session.current = undefined;
      void current.close();
    };
  }, [snapshot.id, transport.drafts]);
  const annotate = useCallback(() => {
    if (
      annotationBusy.current ||
      statusBusy.current ||
      !currentSelector.current ||
      !currentRectangle.current
    )
      return;
    closeRef.current(false);
    setAnnotation({
      key: crypto.randomUUID(),
      selector: structuredClone(currentSelector.current),
      rectangle: { ...currentRectangle.current },
      restored: drafts.current.get(JSON.stringify(currentSelector.current)),
    });
    setRectangle(null);
    panelsRef.current.clear();
  }, []);
  const annotationKey = (value: NonNullable<typeof annotation>) =>
    value.thread ? `${value.thread.writer}:${value.thread.id}` : JSON.stringify(value.selector);
  /** Every nonblank message is a draft; recipient selection never replaces its bytes. */
  const typed = () =>
    annotationDraft.current.edited !== false && annotationDraft.current.value.trim() !== '';
  /** Closes without losing typed text: it comes back when the same selection is annotated again. */
  function closeAnnotation(focusPage: boolean) {
    // A send in flight is not interrupted by the ×, Escape, an outside press or a cleared selection.
    if (!annotation || annotationBusy.current || statusBusy.current) return;
    const key = annotationKey(annotation);
    keepDraft(key, typed() || annotationDraft.current.edited ? annotationDraft.current : null);
    annotationDraft.current = { value: '' };
    setAnnotation(undefined);
    setRectangle(currentRectangle.current);
    if (focusPage) queueMicrotask(() => host.current?.querySelector('iframe')?.focus());
  }
  const closeRef = useRef(closeAnnotation);
  closeRef.current = closeAnnotation;
  const selectionCleared = useRef<() => void>(() => {});
  selectionCleared.current = () => {
    // A new selection keeps typed text; a saved window collapses with its draft cached.
    if (annotation && (annotation.thread || !typed())) closeAnnotation(false);
  };
  useEffect(() => {
    if (!annotation) return;
    const outside = (event: PointerEvent) => {
      if (environment.modalOpen()) return;
      const target = event.target;
      if (
        target instanceof Element &&
        !popover.current?.contains(target) &&
        !target.closest('[role="listbox"]')
      )
        closeRef.current(false);
    };
    document.addEventListener('pointerdown', outside, true);
    return () => document.removeEventListener('pointerdown', outside, true);
  }, [annotation, environment]);
  function openThread(ref: DiscussionRef | null) {
    if (annotationBusy.current || statusBusy.current) return;
    if (!ref) {
      panelsRef.current.setActiveThread(null);
      return;
    }
    const id = `${ref.writer}:${ref.id}`;
    if (annotationRef.current) closeRef.current(false);
    setAnnotation(undefined);
    binding.current?.markThreadStatusSeen?.(ref);
    panelsRef.current.showThread(id);
    environmentRef.current.threadFragment()?.write(ref.id);
    renderer.current?.scrollAnchor(id);
  }
  function continueAnnotation(ref: DiscussionRef) {
    keepDraft(JSON.stringify(annotationRef.current?.selector), null);
    setAnnotation((previous) =>
      previous ? { ...previous, thread: ref, restored: undefined } : previous,
    );
  }
  /** The renderer reports the selection (or that it cleared). */
  function onSelection(
    quote: QuoteSelector | null | undefined,
    rect: SelectionRect | null | undefined,
  ) {
    setSelector(quote ?? null);
    currentSelector.current = quote ?? null;
    currentRectangle.current = rect ?? null;
    setRectangle(rect ?? null);
    if (!quote) selectionCleared.current();
  }

  return {
    selectionMode,
    setSelectionMode,
    selector,
    currentSelector,
    rectangle,
    currentRectangle,
    annotation,
    annotationRef,
    popover,
    drafts,
    draftsReady,
    draftsUnsaved,
    keepDraft,
    annotationDraft,
    annotationBusy,
    statusBusy,
    changingStatus,
    setChangingStatus,
    annotate,
    annotationKey,
    closeAnnotation,
    closeRef,
    onSelection,
    openThread,
    continueAnnotation,
  };
}
