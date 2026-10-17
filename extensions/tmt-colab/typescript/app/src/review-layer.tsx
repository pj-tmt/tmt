import { useCallback, useEffect, useMemo, useRef, useState, type RefObject } from 'react';
import { useAgentDirectory } from './agent-directory.js';
import { AskAgainAction } from './ask-again.js';
import { ReviewCard } from './components/review-card.js';
import { focusNeedsYouTarget, type NeedsYouCandidate } from './needs-you.js';
import type { Question, Review } from './questions.js';
import type { ReviewAnchor, ReviewRenderBlock } from './renderer.js';
import { AdmittedView, ReviewActions, type ReviewOutcome } from './review-actions.js';
import { canonicalReviewThread, repliedComments, reviewState, sectionAnswered } from './reviews.js';
import { text } from './strings.js';
import type { ThreadView } from './thread-records.js';
import type { CommentContext } from './thread-store.js';
import type { PageBinding, PageSnapshot, PageView } from './transport.js';

/** Parent-owned review sections. The frame supplies only each heading's quote and geometry;
 * state, controls and every action come from the admitted discussion and this layer. */
export function useReviewLayer({
  reviews,
  questions,
  snapshot,
  view,
  binding,
  discussionBlocked,
  statusCoordinator,
  slotPositions,
  questionThreads,
  onWorking,
  passage,
  pageUrl,
}: {
  onWorking?(id: string, working: boolean): void;
  passage?: { id?: string };
  pageUrl?: string;
  reviews: readonly Review[];
  questions: readonly Question[];
  snapshot: PageSnapshot;
  view: PageView;
  binding: RefObject<PageBinding | undefined>;
  discussionBlocked: boolean;
  statusCoordinator: PageBinding['status'];
  slotPositions: readonly { id: string; top: number }[];
  /** The threads the question layer associates with a question. */
  questionThreads: RefObject<(questionId: string) => readonly ThreadView[]>;
}) {
  const [anchors, setAnchors] = useState<ReviewAnchor[] | null>(null);
  const targets = useRef(new Map<string, HTMLDivElement>());
  const onAnchors = useCallback((next: ReviewAnchor[], checked = true) => {
    setAnchors(checked ? next : null);
  }, []);
  const [outcomes, setOutcomes] = useState(new Map<string, ReviewOutcome>());
  // A page with no section reads no directory. Without an Ask facade nobody can be asked, so
  // the comment stays on the page instead of waiting for a list that will never come.
  const { directory } = useAgentDirectory(reviews.length ? snapshot.binding?.ask : undefined);
  const resolving = !!snapshot.binding?.ask && directory.state === 'loading';
  const threads = useMemo(
    () =>
      (view.threads ?? []).filter((thread) => !thread.deleted && !thread.proposal && thread.anchor),
    [view.threads],
  );
  const rows = reviews.map((review) => {
    const anchor = anchors?.find((value) => value.id === review.id);
    const associated = threads.filter((thread) =>
      anchor?.threads.includes(`${thread.ref.writer}:${thread.ref.id}`),
    );
    const thread = canonicalReviewThread(review, associated);
    const state = reviewState(review, thread, repliedComments(thread, view.asks ?? []));
    // A unique directory match names the agent a comment tells; page HTML grants no routing.
    const matches =
      directory.state === 'ready'
        ? directory.agents.filter((agent) => agent.agentName === review.agent)
        : [];
    return { review, anchor, thread, state, recipients: matches.length === 1 ? matches : [] };
  });
  const ready = anchors !== null;
  const blocks: ReviewRenderBlock[] = rows.map(({ review, state }) => ({
    id: review.id,
    title: review.title,
    closed: state.kind === 'closed',
  }));

  const context = useRef({
    view,
    snapshot,
    binding,
    discussionBlocked,
    statusCoordinator,
    rows,
    resolving,
    questions,
    onWorking,
    pageUrl,
  });
  context.current = {
    view,
    snapshot,
    binding,
    discussionBlocked,
    statusCoordinator,
    rows,
    resolving,
    questions,
    onWorking,
    pageUrl,
  };
  const admitted = useRef(new AdmittedView());
  const actions = useMemo(() => {
    const pageId = snapshot.id;
    const row = (id: string) => context.current.rows.find((value) => value.review.id === id);
    return new ReviewActions({
      thread: (id) => row(id)?.thread,
      selector: (id) => row(id)?.anchor?.selector,
      recipients: (review) =>
        context.current.resolving ? undefined : (row(review.id)?.recipients ?? []),
      answered: (review) =>
        sectionAnswered(
          review,
          context.current.questions,
          (id) => questionThreads.current?.(id) ?? [],
        ),
      threadHolding: (thread, message) =>
        context.current.view.threads?.find(
          (value) =>
            value.ref.writer === thread.writer &&
            value.ref.id === thread.id &&
            value.comments.some(
              (comment) => comment.ref.writer === message.writer && comment.ref.id === message.id,
            ),
        ),
      discussion: () => context.current.binding.current?.discussion,
      ask: () => context.current.binding.current?.ask,
      asks: () => context.current.view.asks ?? [],
      status: () => context.current.statusCoordinator,
      current: (discussion, ask) =>
        context.current.snapshot.id === pageId &&
        !context.current.discussionBlocked &&
        context.current.binding.current?.discussion === discussion &&
        context.current.binding.current?.ask === ask,
      admitted: admitted.current,
      title: () => context.current.view.title || context.current.snapshot.title,
      url: () => context.current.pageUrl ?? location.href,
      working: (id, working) => context.current.onWorking?.(id, working),
      changed: (id, outcome) => {
        setOutcomes((previous) => new Map(previous).set(id, outcome));
      },
    });
  }, [snapshot.id, questionThreads]);
  useEffect(() => admitted.current.update(), [view.threads]);
  useEffect(() => {
    setOutcomes(new Map());
    setAnchors(null);
  }, [snapshot.id]);

  const writable = !view.archived && !!snapshot.binding?.discussion;
  const navigation: NeedsYouCandidate[] = rows.flatMap(({ review, anchor, state }) =>
    !anchor || !slotPositions.some((slot) => slot.id === review.id)
      ? []
      : [
          {
            kind: 'review',
            state: state.kind,
            id: review.id,
            title: review.title,
            agent: review.agent,
          },
        ],
  );
  return {
    onAnchors,
    blocks,
    navigation,
    ready,
    jump: (id: string) =>
      navigation.some((item) => item.id === id) && focusNeedsYouTarget(targets.current.get(id)),
    /** The trusted Send of answers recorded these comments: close any section they complete. */
    answered: (questionIds: readonly string[], contexts: readonly CommentContext[]) => {
      for (const review of reviews)
        if (review.questions.some((id) => questionIds.includes(id)))
          void actions.closeAnswered(review, contexts);
    },
    inline: (
      <>
        {rows.map(({ review, anchor, state, recipients }) => {
          const slot = slotPositions.find((value) => value.id === review.id);
          const outcome = outcomes.get(review.id);
          return (
            <div
              key={review.id}
              ref={(node) => {
                if (node) targets.current.set(review.id, node);
                else targets.current.delete(review.id);
              }}
              hidden={passage && passage.id !== review.id}
              data-inline-review={review.id}
              data-inline-slot={review.id}
              className="proposal-inline review-inline"
              data-detached={!slot || !anchor || undefined}
              style={{ top: slot?.top ?? 0 }}
            >
              <ReviewCard
                review={review}
                state={state}
                outcome={outcome}
                recipient={recipients[0]?.agentName}
                disabled={discussionBlocked || !writable || resolving || !statusCoordinator}
                readOnly={!!view.archived}
                confirm={(note) => void actions.confirm(review, note)}
                requestChange={(note) => void actions.requestChange(review, note)}
                reopen={() => void actions.reopen(review)}
              >
                {(outcome?.failures ?? []).map((failure) => (
                  <div
                    key={`${failure.message}:${failure.recipient.agent}`}
                    className="annotation-delivery-failure"
                  >
                    <p role="status">
                      @{failure.agent} ·{' '}
                      {failure.uncertain ? text.askUnconfirmed : text.askNotDelivered}
                    </p>
                    {failure.uncertain ? (
                      <p className="ask-supporting">{text.askUncertain}</p>
                    ) : (
                      <AskAgainAction
                        binding={snapshot.binding?.ask}
                        blocked={discussionBlocked}
                        input={failure.input}
                        recipient={failure.recipient}
                        retryOf={null}
                      />
                    )}
                  </div>
                ))}
              </ReviewCard>
            </div>
          );
        })}
      </>
    ),
  };
}
