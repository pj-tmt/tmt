import { useCallback, useEffect, useMemo, useRef, useState, type RefObject } from 'react';
import type { Reading } from './questions.js';
import { ReadingActions, type ReadingOutcome } from './reading-actions.js';
import { canonicalReadingThread, readingState } from './reading.js';
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
}: {
  readings: readonly Reading[];
  snapshot: PageSnapshot;
  view: PageView;
  binding: RefObject<PageBinding | undefined>;
  blocked: boolean;
  status: PageBinding['status'];
  renderer: RefObject<Awaited<ReturnType<typeof mountRenderer>> | null>;
  hint?: string;
}) {
  const [anchors, setAnchors] = useState<QuestionAnchor[] | null>(null);
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
  const jump = useCallback((id: string) => {
    if (context.current.rows.some((row) => row.reading.id === id && row.anchor))
      context.current.renderer.current?.scrollReading(id);
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
  return { onAnchors, blocks, rows, outcomes, actions, items, jump, next, ready: anchors !== null };
}
