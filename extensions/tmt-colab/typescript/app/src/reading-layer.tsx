import { BrowserAction, BusyMark } from '@tmt/browser-ui/react';
import { useCallback, useEffect, useId, useMemo, useRef, useState, type RefObject } from 'react';
import { AgentChip, agentSentence } from './components/agent-chip.js';
import { NeedsYouBar } from './components/needs-you-bar.js';
import { ReadingConnecting } from './reading-connecting.js';
import { focusNeedsYouTarget } from './needs-you.js';
import { compactRelativeTime } from './display-time.js';
import { text } from './strings.js';
import type { Reading } from './questions.js';
import { ReadingActions, type ReadingOutcome } from './reading-actions.js';
import { canonicalReadingThread, readingState, readingNotification } from './reading.js';
import { AdmittedView } from './review-actions.js';
import type { QuestionAnchor, ReadingRenderBlock, mountRenderer } from './renderer.js';
import type { PageBinding, PageSnapshot, PageView } from './transport.js';

/** A presentation-only entry; neighboring question/review projections can use the same shape. */
export interface ReadingNavigationItem {
  id: string;
  title: string;
  state: 'unread' | 'read';
}

/** The frame resolves only quote/geometry. Admitted ordinary records own each reading state. */
export function useReadingLayer({
  readings,
  snapshot,
  view,
  binding,
  blocked,
  status,
  renderer,
  hint,
  slotPositions,
  readingIds,
}: {
  readings: readonly Reading[];
  snapshot: PageSnapshot;
  view: PageView;
  binding: RefObject<PageBinding | undefined>;
  blocked: boolean;
  status: PageBinding['status'];
  renderer: RefObject<Awaited<ReturnType<typeof mountRenderer>> | null>;
  hint?: string;
  slotPositions: readonly { id: string; top: number }[];
  readingIds: ReadonlySet<string>;
}) {
  const detailPrefix = useId();
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set());
  const [now, setNow] = useState(0);
  useEffect(() => setNow(Date.now()), [view.threads]);
  const [anchors, setAnchors] = useState<QuestionAnchor[] | null>(null);
  const targets = useRef(new Map<string, HTMLElement>());
  const [outcomes, setOutcomes] = useState(new Map<string, ReadingOutcome>());
  const onAnchors = useCallback(
    (next: QuestionAnchor[], checked = true) => setAnchors(checked ? next : null),
    [],
  );
  const threads = (view.threads ?? []).filter(
    (thread) => !thread.deleted && !thread.proposal && thread.anchor,
  );
  const rows = readings.map((reading) => {
    const anchor = anchors?.find((anchor) => anchor.id === reading.id);
    const thread = canonicalReadingThread(
      reading,
      threads.filter((thread) => anchor?.threads.includes(`${thread.ref.writer}:${thread.ref.id}`)),
    );
    return { reading, anchor, thread, state: readingState(reading, thread) };
  });
  const context = useRef({ snapshot, view, binding, blocked, status, renderer, rows });
  context.current = { snapshot, view, binding, blocked, status, renderer, rows };
  const jump = useCallback((id: string, focus = false) => {
    if (context.current.rows.some((row) => row.reading.id === id && row.anchor)) {
      context.current.renderer.current?.scrollReading(id);
      if (focus) focusNeedsYouTarget(targets.current.get(id));
    }
  }, []);
  const next = useCallback(
    (id: string) => {
      const rows = context.current.rows;
      const index = rows.findIndex((row) => row.reading.id === id);
      const remaining = [...rows.slice(index + 1), ...rows.slice(0, Math.max(0, index))];
      const target = remaining.find((row) => row.state.kind === 'unread' && row.anchor);
      if (target) jump(target.reading.id);
    },
    [jump],
  );
  const admitted = useRef(new AdmittedView());
  const followed = useRef('');
  const actions = useMemo(() => {
    const page = snapshot.id;
    return new ReadingActions({
      thread: (id) => context.current.rows.find((row) => row.reading.id === id)?.thread,
      selector: (id) => context.current.rows.find((row) => row.reading.id === id)?.anchor?.selector,
      threadHolding: (thread, message) =>
        context.current.view.threads?.find(
          (row) =>
            row.ref.writer === thread.writer &&
            row.ref.id === thread.id &&
            row.comments.some(
              (comment) => comment.ref.writer === message.writer && comment.ref.id === message.id,
            ),
        ),
      discussion: () => context.current.binding.current?.discussion,
      status: () => context.current.status,
      current: (discussion) =>
        context.current.snapshot.id === page &&
        !context.current.blocked &&
        !context.current.view.archived &&
        context.current.binding.current?.discussion === discussion,
      title: () => context.current.view.title || context.current.snapshot.title,
      admitted: admitted.current,
      changed: (id, outcome) => {
        setOutcomes((previous) => new Map(previous).set(id, outcome));
        if (outcome.state === 'settled') next(id);
      },
    });
  }, [snapshot.id, next]);
  useEffect(() => admitted.current.update(), [view.threads]);
  useEffect(() => {
    setAnchors(null);
    setOutcomes(new Map());
    setExpanded(new Set());
    followed.current = '';
  }, [snapshot.id]);
  useEffect(() => {
    if (!hint) {
      followed.current = '';
      return;
    }
    if (anchors === null) return;
    const key = `${snapshot.id}:${hint}`;
    if (followed.current === key) return;
    followed.current = key;
    jump(hint);
  }, [hint, anchors, snapshot.id, jump]);
  const blocks: ReadingRenderBlock[] = rows.map(({ reading, state }) => ({
    id: reading.id,
    title: reading.title,
    closed: state.kind === 'read',
  }));
  const items: ReadingNavigationItem[] =
    anchors === null
      ? []
      : rows
          .filter((row) => row.anchor)
          .map(({ reading, state }) => ({
            id: reading.id,
            title: reading.title,
            state: state.kind,
          }));
  const writable = !blocked && !view.archived && !!snapshot.binding?.discussion && !!status;
  const inline = rows.map(({ reading, anchor, thread, state }) => {
    const slot = slotPositions.find((value) => value.id === reading.id);
    const attached = !!slot && !!anchor && readingIds.has(reading.id);
    const outcome = outcomes.get(reading.id);
    const notification = readingNotification(thread, view.asks ?? []);
    const read = state.kind === 'read';
    const notifiedAgent = notification?.recipient.agentName ?? reading.agent;
    const notified = notification?.state === 'accepted';
    const notNotified =
      !!outcome?.warnings?.length ||
      notification?.state === 'unavailable' ||
      notification?.state === 'held' ||
      notification?.state === 'refused';
    const warning =
      read && !notified
        ? agentSentence(
            notNotified ? text.readingNotNotified : text.readingUnconfirmed,
            notifiedAgent,
          )
        : undefined;
    const reason =
      notification?.state === 'held'
        ? text.readingHeld
        : notification?.state === 'refused'
          ? text.readingRefused
          : notNotified
            ? text.readingUnavailable
            : text.readingNoticeUnconfirmed;
    const detailId = `${detailPrefix}-${reading.id}`;
    const readOnly = !!view.archived || !snapshot.binding?.discussion;
    const at = read && state.at ? Number(state.at) : NaN;
    const time = Number.isFinite(at) ? compactRelativeTime(at, now) : text.readingTimeUnknown;
    const working = outcome?.state === 'working';
    const unavailable = !writable || !attached;
    // Mark read acts only when the discussion, status and passage are all live; until then it is
    // disabled and says so, so a click is never ignored.
    const markReady = actions.ready(reading);
    // Work in flight fences every action without greying it; only Mark read carries the mark, in a
    // slot every action reserves.
    const activate = (
      label: string,
      action: () => void,
      disabled = unavailable,
      marked = false,
    ) => (
      <BrowserAction
        key={label}
        type="button"
        variant="text"
        label={label}
        busy={working}
        busyMark={marked ? <BusyMark /> : <span />}
        disabled={disabled}
        onActivate={(event) => {
          if (event.isTrusted) action();
        }}
      />
    );
    const hasNext = rows.some(
      (row) => row.reading.id !== reading.id && row.anchor && row.state.kind === 'unread',
    );
    return (
      <section
        key={reading.id}
        ref={(node) => {
          if (node) targets.current.set(reading.id, node);
          else targets.current.delete(reading.id);
        }}
        data-inline-reading={reading.id}
        data-inline-slot={reading.id}
        className="proposal-inline reading-inline"
        data-detached={!attached || undefined}
        data-reading-state={state.kind}
        data-reason-expanded={(!!warning && (readOnly || expanded.has(reading.id))) || undefined}
        aria-label={text.readingSection(reading.title)}
        style={{ top: slot?.top ?? 0 }}
      >
        <NeedsYouBar
          tone={read ? 'done' : 'needs'}
          word={read ? text.readingWord : text.readingPleaseRead}
          who={read ? time : <AgentChip name={reading.agent} />}
          why={
            read
              ? notified
                ? agentSentence(text.readingNotified, notifiedAgent)
                : warning
              : reading.why
          }
          role={read ? 'status' : undefined}
          actions={
            readOnly ? undefined : (
              <>
                {!read &&
                  activate(
                    text.readingMark,
                    () => void actions.mark(reading),
                    unavailable || !markReady,
                    true,
                  )}
                {!read && <ReadingConnecting waiting={writable && attached && !markReady} />}
                {activate(text.readingNext, () => next(reading.id), !attached || !hasNext)}
                {warning && (
                  <button
                    type="button"
                    className="tmt-ui-action reading-why"
                    data-variant="text"
                    aria-expanded={expanded.has(reading.id)}
                    aria-controls={detailId}
                    onClick={() =>
                      setExpanded((previous) => {
                        const next = new Set(previous);
                        if (next.has(reading.id)) next.delete(reading.id);
                        else next.add(reading.id);
                        return next;
                      })
                    }
                  >
                    {text.readingWhy}
                  </button>
                )}
              </>
            )
          }
        />
        {warning && (
          <p
            id={detailId}
            className="reading-reason"
            hidden={!readOnly && !expanded.has(reading.id)}
          >
            {reason}
          </p>
        )}
        {!read && outcome?.state === 'failed' && (
          <p role="alert">
            {outcome.step === 'closing' ? text.readingCommentKept : text.readingFailed}
          </p>
        )}
        {!read && outcome?.state === 'uncertain' && <p role="status">{text.readingUncertain}</p>}
      </section>
    );
  });
  return {
    onAnchors,
    blocks,
    rows,
    outcomes,
    actions,
    items,
    jump,
    next,
    inline,
    ready: anchors !== null,
  };
}
