import { useCallback, useEffect, useRef, useState, type RefObject } from 'react';
import type { ComposerEdit } from './components/message-composer-edit.js';
import { DraftSession } from './draft-store.js';
import { usePageEnvironment } from './page-environment.js';
import type { mountRenderer } from './renderer.js';
import type { SelectionRect } from './renderer.js';
import type { DiscussionRef, QuoteSelector } from './thread-records.js';
import type { PageBinding, PageSnapshot, PageTransport } from './transport.js';

/** One annotation window: a new one on a selection, or one on a thread. */
export interface Annotation {
  key: string;
  selector: QuoteSelector | null;
  rectangle: SelectionRect;
  thread?: DiscussionRef;
  /** Text kept from an earlier close of this selection or thread. */
  restored?: ComposerEdit;
}

/** What the annotation window needs from the page's panels: it closes and selects threads. */
interface AnnotationPanels {
  clear(): void;
  showThread(id: string): void;
}

/**
 * The page's selection and annotation: selection mode, the quote selected and where, the open
 * annotation window (a new one on a selection, or one on a thread), the unsent drafts kept per
 * quote or thread and their device-local session, and the actions that open, continue and close
 * them. Everything here clears when the page changes.
 *
 * Each annotation keeps its own composer, and so its own send, for as long as that send runs
 * (#5816): closing a window whose send is in flight only hides it, another annotation opens beside
 * it, and nothing waits for another annotation's send. A hidden annotation goes when its send
 * settles with a record (its thread carries the status and recovery from then on); one whose send
 * failed before a record stays hidden with its failed row, draft and files, is listed in
 * `unsentAnnotations` so the page says so, and opens again from there or on the same quote.
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
  const [annotations, setAnnotations] = useState<Annotation[]>([]);
  // The one window shown; every other annotation is hidden while its send runs or holds a failure.
  const [visibleKey, setVisibleKey] = useState<string>();
  const annotation = annotations.find((value) => value.key === visibleKey);
  // The annotations whose send is in flight; each one reports its own.
  const sending = useRef(new Set<string>());
  // The annotations whose send failed before a record: their composer holds the failed row.
  const unsent = useRef(new Set<string>());
  const [, setUnsentVersion] = useState(0);
  // The annotations whose last send recorded its message; recovery is on its thread from then on.
  const recorded = useRef(new Set<string>());
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
  const statusBusy = useRef(false);
  const [changingStatus, setChangingStatus] = useState(false);
  useEffect(() => {
    setAnnotations([]);
    setVisibleKey(undefined);
    sending.current.clear();
    unsent.current.clear();
    recorded.current.clear();
    drafts.current.clear();
    annotationDraft.current = { value: '' };
    statusBusy.current = false;
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
  const annotationsRef = useRef(annotations);
  annotationsRef.current = annotations;
  const annotate = useCallback(() => {
    if (statusBusy.current || !currentSelector.current || !currentRectangle.current) return;
    const quote = JSON.stringify(currentSelector.current);
    closeRef.current(false);
    setSelectionMode(false);
    // The same quote again shows the hidden annotation whose send failed before it was recorded.
    const held = annotationsRef.current.find(
      (value) => !value.thread && JSON.stringify(value.selector) === quote,
    );
    const rectangle = { ...currentRectangle.current };
    if (held) {
      setAnnotations((all) =>
        all.map((value) => (value.key === held.key ? { ...value, rectangle } : value)),
      );
      setVisibleKey(held.key);
    } else {
      const key = crypto.randomUUID();
      setAnnotations((all) => [
        ...all,
        {
          key,
          selector: structuredClone(currentSelector.current),
          rectangle,
          restored: drafts.current.get(quote),
        },
      ]);
      setVisibleKey(key);
    }
    setRectangle(null);
    panelsRef.current.clear();
  }, []);
  const annotationKey = (value: NonNullable<typeof annotation>) =>
    value.thread ? `${value.thread.writer}:${value.thread.id}` : JSON.stringify(value.selector);
  /** Every nonblank message is a draft; recipient selection never replaces its bytes. */
  const typed = () =>
    annotationDraft.current.edited !== false && annotationDraft.current.value.trim() !== '';
  /** Closes without losing typed text: it comes back when the same selection is annotated again.
   * A window whose send is in flight (or whose send failed before a record) is hidden instead, so
   * its composer keeps the send, the failed row and the files. */
  function closeAnnotation(focusPage: boolean) {
    if (!annotation || statusBusy.current) return;
    if (!retained(annotation.key)) {
      const key = annotationKey(annotation);
      keepDraft(key, typed() || annotationDraft.current.edited ? annotationDraft.current : null);
      setAnnotations((all) => all.filter((value) => value.key !== annotation.key));
    }
    annotationDraft.current = { value: '' };
    setVisibleKey(undefined);
    setRectangle(currentRectangle.current);
    if (focusPage) queueMicrotask(() => host.current?.querySelector('iframe')?.focus());
  }
  const retained = (key: string) => sending.current.has(key) || unsent.current.has(key);
  /** An annotation's composer reports its own send; a hidden one goes once nothing needs it. */
  function annotationSending(key: string, busy: boolean) {
    if (busy) {
      sending.current.add(key);
      if (unsent.current.delete(key)) setUnsentVersion((n) => n + 1);
      recorded.current.delete(key);
      return;
    }
    if (!sending.current.delete(key)) return;
    // A new annotation whose send failed before a record keeps its failed row here; a reply's
    // draft goes back to its thread's composer as before.
    const value = annotationsRef.current.find((item) => item.key === key);
    if (!recorded.current.has(key) && value && !value.thread) {
      unsent.current.add(key);
      setUnsentVersion((n) => n + 1);
    }
    if (key !== annotationRef.current?.key && !unsent.current.has(key))
      setAnnotations((all) => all.filter((item) => item.key !== key));
  }
  /** Discarding a kept draft also releases the hidden window of that quote whose first comment
   * was not sent: its failed row, alert row and draft go together. A send still running stays. */
  function discardDraft(key: string) {
    keepDraft(key, null);
    const held = annotationsRef.current.find(
      (value) =>
        !value.thread &&
        value.key !== annotationRef.current?.key &&
        unsent.current.has(value.key) &&
        annotationKey(value) === key,
    );
    if (!held) return;
    unsent.current.delete(held.key);
    setUnsentVersion((n) => n + 1);
    setAnnotations((all) => all.filter((value) => value.key !== held.key));
  }
  /** Shows a hidden annotation whose send failed before a record: its failed row, draft and Retry. */
  function openUnsent(key: string) {
    if (statusBusy.current || !unsent.current.has(key)) return;
    if (annotationRef.current) closeRef.current(false);
    setSelectionMode(false);
    setVisibleKey(key);
    setRectangle(null);
    panelsRef.current.clear();
  }
  /** An annotation's send recorded its message; its delivery may still be running. */
  function annotationRecorded(key: string) {
    recorded.current.add(key);
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
  function openThread(ref: DiscussionRef, { revealInPage = true } = {}) {
    if (statusBusy.current) return;
    const id = `${ref.writer}:${ref.id}`;
    if (annotationRef.current) closeRef.current(false);
    setVisibleKey(undefined);
    binding.current?.markThreadStatusSeen?.(ref);
    panelsRef.current.showThread(id);
    environmentRef.current.threadFragment()?.write(ref.id);
    if (revealInPage) renderer.current?.scrollAnchor(id);
  }
  /** An annotation's send recorded its thread: that window now shows the thread, shown or not. */
  function continueAnnotation(key: string, ref: DiscussionRef) {
    const value = annotationsRef.current.find((item) => item.key === key);
    if (!value) return;
    if (unsent.current.delete(key)) setUnsentVersion((n) => n + 1);
    keepDraft(JSON.stringify(value.selector), null);
    setAnnotations((all) =>
      all.map((item) => (item.key === key ? { ...item, thread: ref, restored: undefined } : item)),
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
    annotations,
    // The closed windows whose first comment was not sent; the page names each with an Open action.
    unsentAnnotations: annotations.filter(
      (value) => value.key !== visibleKey && unsent.current.has(value.key),
    ),
    openUnsent,
    discardDraft,
    annotationRef,
    popover,
    drafts,
    draftsReady,
    draftsUnsaved,
    keepDraft,
    annotationDraft,
    annotationSending,
    annotationRecorded,
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
