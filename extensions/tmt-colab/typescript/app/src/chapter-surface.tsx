import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { BrowserAction } from '@tmt/browser-ui/react';
import { chapterTargets, selectChapterTarget, type ChapterTargetRef } from './chapter-passage.js';
import type { ChapterPassages } from './chapter-passages.js';
import { ConversationTurn } from './components/conversation-turn.js';
import { ConversationPresentation } from './components/conversation-presentation.js';
import { MessageText } from './components/message-text.js';
import { compactRelativeTime } from './display-time.js';
import { PassageNavigation } from './passage-navigation.js';
import { pendingNeedsYou, proposalNeedsYou } from './needs-you.js';
import { usePageLayers } from './page-layers.js';
import { inspectQuestions } from './questions.js';
import { mountRenderer, type PassageBounds, type RenderState } from './renderer.js';
import type { PageSnapshot, PageTransport, PageView } from './transport.js';
import { text } from './strings.js';
import './chapter-surface.css';
const copy = text.today.passage;

/** The mounted owner keeps this component alive while an admitted action settles. Its frame
 * can retire independently; the ordinary page layers remain the sole action implementation. */
export function ChapterSurface({
  surfaceId,
  anchor,
  unavailableText,
  snapshot,
  transport,
  passages,
  visible,
  busy,
  detached,
  open,
  url,
  onCount,
}: {
  surfaceId: string;
  anchor?: string;
  unavailableText?: string;
  snapshot: PageSnapshot;
  transport: PageTransport;
  passages: ChapterPassages;
  visible: boolean;
  busy: boolean;
  detached: boolean;
  open(): void;
  url: string;
  onCount(count: number): void;
}) {
  const [view, setView] = useState<PageView>({ ...snapshot, ownData: snapshot.ownData ?? false });
  const [failed, setFailed] = useState(false);
  const binding = useRef(snapshot.binding);
  binding.current = snapshot.binding;
  useEffect(() => snapshot.binding?.subscribe(setView, () => setFailed(true)), [snapshot.binding]);
  const host = useRef<HTMLDivElement>(null);
  const clip = useRef<HTMLDivElement>(null);
  const controls = useRef<HTMLDivElement>(null);
  const replyBody = useRef<HTMLDivElement>(null);
  const renderer = useRef<Awaited<ReturnType<typeof mountRenderer>> | null>(null);
  const [state, setState] = useState<RenderState | 'loading'>('loading');
  const [bounds, setBounds] = useState<PassageBounds | null>(null);
  const [passageUnavailable, setPassageUnavailable] = useState(false);
  const onPassage = useCallback((value: PassageBounds | null, checked = false) => {
    setBounds(value);
    setPassageUnavailable(checked && value === null);
  }, []);
  const [clipped, setClipped] = useState(false);
  const [selected, setSelected] = useState<ChapterTargetRef>();
  const markup = useMemo(() => inspectQuestions(view.source), [view.source]);
  const answering = useRef(new Set<string>());
  const retiredAnswers = useRef(new Set<string>());
  const onWorking = useCallback(
    (kind: string, id: string, working: boolean) => {
      if (kind === 'answer') {
        if (working) {
          answering.current.add(id);
          retiredAnswers.current.delete(id);
        } else answering.current.delete(id);
      }
      passages.working(surfaceId, `${kind}:${id}`, working);
    },
    [passages, surfaceId],
  );
  const layers = usePageLayers({
    snapshot,
    view,
    transport,
    binding,
    host,
    renderer,
    state,
    // Visibility never invalidates an action which already published its ordinary record.
    discussionBlocked: failed || !!view.archived,
    statusCoordinator: snapshot.binding?.status,
    chapter: {
      selected,
      onWorking,
      controls,
      url,
      mayCloseAnswered: (ids) =>
        !markup.groups.some(
          (group) =>
            retiredAnswers.current.has(group.id) && group.questions.some((id) => ids.includes(id)),
        ),
    },
  });
  const candidates = [
    ...layers.reviewLayer.navigation,
    ...layers.questionLayer.navigation,
    ...layers.readingLayer.items.map((item) => ({
      ...item,
      kind: 'reading' as const,
      agent: markup.readings.find((reading) => reading.id === item.id)?.agent ?? '',
    })),
  ];
  const targets = chapterTargets(
    candidates,
    [...markup.reviews, ...markup.groups, ...markup.readings].map((item, order) => ({
      id: item.id,
      order,
    })),
  );
  // A reference can select only a target already admitted by the ordinary parent layers.
  // An arbitrary matching HTML id is only a cosmetic crop, with no parent controls.
  const referenced = anchor
    ? targets.find(
        (item) =>
          item.id === anchor ||
          (item.kind === 'answer' &&
            markup.groups.find((group) => group.id === item.id)?.questions.includes(anchor)),
      )
    : undefined;
  const target = anchor && !selected ? referenced : selectChapterTarget(targets, selected);
  const generic = anchor && !target ? anchor : undefined;
  const targetKind = target?.kind;
  const targetId = target?.id;
  useEffect(() => {
    setSelected((previous) =>
      previous?.id === targetId && previous?.kind === targetKind
        ? previous
        : targetId && targetKind
          ? { kind: targetKind, id: targetId }
          : undefined,
    );
  }, [targetId, targetKind]);
  const ready = layers.reviewLayer.ready && layers.questionLayer.ready && layers.readingLayer.ready;
  const pendingCount = ready
    ? pendingNeedsYou([...proposalNeedsYou(view.threads ?? []), ...candidates]).length
    : undefined;
  useEffect(() => {
    if (pendingCount !== undefined) onCount(pendingCount);
  }, [onCount, pendingCount]);
  const reply =
    ready && !target && !anchor
      ? [...(view.asks ?? [])]
          .filter((ask) => ask.reply !== undefined)
          .sort((a, b) => b.issuedAt - a.issuedAt || a.operationId.localeCompare(b.operationId))[0]
      : undefined;
  useEffect(() => {
    if (state === 'ready')
      renderer.current?.passage(
        targetKind && targetId
          ? { kind: targetKind, id: targetId }
          : generic
            ? { kind: 'section', id: generic }
            : { kind: 'opening' },
      );
  }, [targetKind, targetId, generic, state]);
  const imageInput = useRef({
    binding: snapshot.binding?.files?.attachments,
    descriptors: view.attachments ?? [],
  });
  imageInput.current = {
    binding: snapshot.binding?.files?.attachments,
    descriptors: view.attachments ?? [],
  };
  const imageDescriptors = JSON.stringify(view.attachments ?? []);
  useEffect(() => {
    renderer.current?.images(imageInput.current);
  }, [imageDescriptors]);
  const { onQuestionAnchors, onReadingAnchors } = layers;
  const onSlots = layers.inlineSlots.onSlots;
  const rendering = !detached && (visible || busy);
  useLayoutEffect(() => {
    if (!rendering || failed || passageUnavailable) return;
    const controller = new AbortController();
    const retire = (slotRemoved = false) => {
      if (slotRemoved) for (const id of answering.current) retiredAnswers.current.add(id);
      controller.abort();
      renderer.current?.destroy();
      renderer.current = null;
    };
    const unframe = passages.frame(surfaceId, retire);
    setBounds(null);
    setState('loading');
    void mountRenderer(host.current!, view.source, {
      signal: controller.signal,
      onState: setState,
      onSlots,
      onQuestions: onQuestionAnchors,
      onReadings: onReadingAnchors,
      onPassage,
    })
      .then((handle) => {
        if (controller.signal.aborted) handle.destroy();
        else {
          renderer.current = handle;
          handle.images(imageInput.current);
        }
      })
      .catch(() => {
        if (!controller.signal.aborted) setFailed(true);
      });
    return () => {
      retire();
      unframe();
    };
  }, [
    rendering,
    failed,
    passageUnavailable,
    onPassage,
    passages,
    surfaceId,
    snapshot.id,
    view.source,
    onSlots,
    onQuestionAnchors,
    onReadingAnchors,
  ]);
  useLayoutEffect(() => {
    const node = clip.current;
    if (!node || !bounds) {
      setClipped(false);
      return;
    }
    const measure = () =>
      setClipped(
        reply
          ? Math.ceil(replyBody.current?.getBoundingClientRect().height ?? 0) > node.clientHeight
          : node.clientHeight < bounds.height,
      );
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    if (replyBody.current) observer.observe(replyBody.current);
    measure();
    return () => observer.disconnect();
  }, [bounds, reply]);
  const unavailable = passageUnavailable || failed || state === 'failed' || state === 'navigation';
  const next = target && selectChapterTarget(targets, target, true);
  const pending =
    target &&
    ({ review: 'waiting', answer: 'unanswered', reading: 'unread' } as const)[target.kind] ===
      target.state;
  const [now, setNow] = useState(0);
  useEffect(() => setNow(Date.now()), [view.asks]);
  return (
    <section
      className="chapter-passage"
      hidden={!visible && (!busy || detached)}
      inert={!visible}
      aria-hidden={!visible || undefined}
      style={!visible ? { visibility: 'hidden' } : undefined}
      data-chapter-passage={snapshot.id}
    >
      {unavailable ? (
        <p className="chapter-passage-unavailable">
          {unavailableText ?? copy.unavailable} ·{' '}
          <BrowserAction type="button" variant="text" label={copy.open} onActivate={open} />
        </p>
      ) : (
        <>
          {!generic && (
            <p className="chapter-passage-kicker">
              {target
                ? copy.kinds[target.kind]
                : reply
                  ? copy.reply(reply.agentName, compactRelativeTime(reply.issuedAt, now))
                  : copy.opening}
            </p>
          )}
          <div
            ref={clip}
            className="chapter-passage-clip"
            style={{
              height: reply ? undefined : (bounds?.height ?? 1),
              visibility: !reply && !bounds ? 'hidden' : undefined,
            }}
          >
            <div
              ref={host}
              className="chapter-passage-source"
              style={{
                transform: `translateY(-${bounds?.top ?? 0}px)`,
                ...(reply ? { position: 'absolute', visibility: 'hidden', width: '100%' } : {}),
              }}
            />
            {reply && (
              <div ref={replyBody} className="chapter-passage-reply">
                <ConversationPresentation.Provider value="chat">
                  <ConversationTurn role="agent" author={reply.agentName} at={reply.issuedAt}>
                    <pre>
                      <MessageText value={reply.reply!} names={[reply.agentName]} />
                    </pre>
                  </ConversationTurn>
                </ConversationPresentation.Provider>
              </div>
            )}
          </div>
          <div ref={controls} className="chapter-passage-controls" hidden={!target || !bounds}>
            <PassageNavigation.Provider
              value={
                !pending &&
                next &&
                next.id !== target?.id && (
                  <BrowserAction
                    type="button"
                    variant="text"
                    label={copy.next}
                    onActivate={() => setSelected({ kind: next.kind, id: next.id })}
                  />
                )
              }
            >
              {layers.reviewLayer.inline}
              {layers.questionLayer.inline}
              {layers.readingLayer.inline}
            </PassageNavigation.Provider>
          </div>
          {clipped && (
            <div className="chapter-passage-continue">
              <BrowserAction type="button" variant="text" label={copy.continue} onActivate={open} />
            </div>
          )}
        </>
      )}
    </section>
  );
}
