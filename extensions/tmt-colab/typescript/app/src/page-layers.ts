import { PassageDraftScope } from './passage-drafts.js';
import { createElement, useCallback, useMemo, useRef, type RefObject } from 'react';
import { useInlineSlots } from './inline-slots.js';
import { inspectQuestions } from './questions.js';
import { useQuestionLayer } from './question-layer.js';
import { useReviewLayer } from './review-layer.js';
import { useReadingLayer } from './reading-layer.js';
import type { PageBinding, PageSnapshot, PageView, PageTransport } from './transport.js';
import type { ThreadView } from './thread-records.js';
import type { mountRenderer, QuestionAnchor, ReviewAnchor, RenderState } from './renderer.js';
import type { ComposerConnectionPresentation } from './router.js';

/** One composition owns admitted state and actions for page and chapter surfaces alike. */
export function usePageLayers({
  transport,
  snapshot,
  view,
  binding,
  renderer,
  host,
  state,
  discussionBlocked,
  statusCoordinator,
  connectionPresentation,
  readingHint,
}: {
  transport: PageTransport;
  snapshot: PageSnapshot;
  view: PageView;
  binding: RefObject<PageBinding | undefined>;
  renderer: RefObject<Awaited<ReturnType<typeof mountRenderer>> | null>;
  host: RefObject<HTMLDivElement | null>;
  state: RenderState | 'loading';
  discussionBlocked: boolean;
  statusCoordinator: PageBinding['status'];
  connectionPresentation?: ComposerConnectionPresentation;
  readingHint?: string;
}) {
  const { questions, groups, reviews, readings } = useMemo(
    () => inspectQuestions(view.source),
    [view.source],
  );
  const inlineSlots = useInlineSlots({
    renderer,
    host,
    state,
    source: view.source,
    identity: `${(view.threads ?? [])
      .filter((thread) => thread.proposal && !thread.deleted)
      .map((thread) => thread.threadId)
      .join(
        ',',
      )}|${groups.map((group) => group.id).join(',')}|${reviews.map((review) => review.id).join(',')}|${readings.map((reading) => reading.id).join(',')}`,
  });
  const associatedQuestionThreads = useRef<(id: string) => readonly ThreadView[]>(() => []);
  const reviewLayer = useReviewLayer({
    reviews,
    questions,
    snapshot,
    view,
    binding,
    discussionBlocked,
    statusCoordinator,
    slotPositions: inlineSlots.positions,
    questionThreads: associatedQuestionThreads,
  });
  const readingLayer = useReadingLayer({
    readings,
    snapshot,
    view,
    binding,
    blocked: discussionBlocked,
    status: statusCoordinator,
    renderer,
    hint: readingHint,
    slotPositions: inlineSlots.positions,
    readingIds: inlineSlots.readingIds,
  });
  const questionLayer = useQuestionLayer({
    questions,
    groups,
    reviews: reviewLayer.blocks,
    readings: readingLayer.blocks,
    onRecorded: reviewLayer.answered,
    snapshot,
    view,
    renderer,
    state,
    slots: inlineSlots.positions,
    blocked: discussionBlocked,
    connectionPresentation,
  });
  associatedQuestionThreads.current = questionLayer.associated;
  const { onQuestions } = questionLayer;
  const { onAnchors: onReviewAnchors } = reviewLayer;
  const { onAnchors: onReadingAnchors } = readingLayer;
  const onQuestionAnchors = useCallback(
    (anchored: QuestionAnchor[], reviewed: ReviewAnchor[], checked = true) => {
      onQuestions(anchored, checked);
      onReviewAnchors(reviewed, checked);
    },
    [onQuestions, onReviewAnchors],
  );
  return {
    readings,
    inlineSlots,
    reviewLayer: {
      ...reviewLayer,
      inline: createElement(
        PassageDraftScope,
        { transport, pageId: snapshot.id },
        reviewLayer.inline,
      ),
    },
    readingLayer,
    questionLayer: {
      ...questionLayer,
      inline: createElement(
        PassageDraftScope,
        { transport, pageId: snapshot.id },
        questionLayer.inline,
      ),
    },
    onQuestionAnchors,
    onReadingAnchors,
  };
}
