import { useEffect, useMemo, useRef, useState, type RefObject } from 'react';
import type { ComposerEdit } from './components/message-composer-edit.js';
import { AnnotationInput, type MessageSendFailure } from './annotation-input.js';
import { ProposalCard } from './components/proposal-card.js';
import { ProposalActions, type ProposalOutcome } from './proposal-actions.js';
import { conversationAsks } from './thread-store.js';
import { AskAgainAction } from './ask-again.js';
import { CommentExchange } from './thread-panel.js';
import { presentationOf } from './thread-status-presentation.js';
import type { ThreadView } from './thread-records.js';
import type { PageSnapshot, PageView, PageBinding } from './transport.js';
import type { RenderState, mountRenderer } from './renderer.js';
import { text } from './strings.js';

/** Parent-owned proposal presentation; the router wires admitted page and renderer ports. */
export function useProposalLayer({
  snapshot,
  view,
  binding,
  renderer,
  host,
  state,
  discussionBlocked,
  recoveryRequired,
  statusCoordinator,
  otherBusy,
  closeAnnotation,
  draft,
  keepDraft,
}: {
  snapshot: PageSnapshot;
  view: PageView;
  binding: RefObject<PageBinding | undefined>;
  renderer: RefObject<Awaited<ReturnType<typeof mountRenderer>> | null>;
  host: RefObject<HTMLDivElement | null>;
  state: RenderState | 'loading';
  discussionBlocked: boolean;
  recoveryRequired: boolean;
  statusCoordinator: PageBinding['status'];
  otherBusy(): boolean;
  closeAnnotation(): void;
  draft(key: string): ComposerEdit | undefined;
  keepDraft(key: string, edit: ComposerEdit | null): void;
}) {
  const [slotPositions, setSlotPositions] = useState<{ id: string; top: number }[]>([]);
  const [proposalStatusErrors, setProposalStatusErrors] = useState(new Set<string>());
  const [proposalOutcomes, setProposalOutcomes] = useState(new Map<string, ProposalOutcome>());
  const proposalContext = useRef({ discussionBlocked, snapshot, view, binding, renderer, host });
  const proposalActions = useMemo(() => {
    const pageId = snapshot.id;
    return new ProposalActions({
      thread: (id) =>
        proposalContext.current.view.threads?.find((t) => `${t.ref.writer}:${t.ref.id}` === id),
      discussion: () => proposalContext.current.binding.current?.discussion,
      ask: () => proposalContext.current.binding.current?.ask,
      asks: () => proposalContext.current.view.asks ?? [],
      current: (discussion, ask) =>
        proposalContext.current.snapshot.id === pageId &&
        !proposalContext.current.discussionBlocked &&
        proposalContext.current.binding.current?.discussion === discussion &&
        proposalContext.current.binding.current?.ask === ask,
      title: () => proposalContext.current.view.title || proposalContext.current.snapshot.title,
      url: () => location.href,
      changed: (id, outcome) =>
        setProposalOutcomes((previous) => new Map(previous).set(id, outcome)),
    });
  }, [snapshot.id]);
  useEffect(() => setProposalOutcomes(new Map()), [snapshot.id]);
  const [proposalComposer, setProposalComposer] = useState<{
    id: string;
    key: string;
    location: 'inline' | 'comments';
  }>();
  const [proposalSending, setProposalSending] = useState(false);
  const [proposalFailures, setProposalFailures] = useState(new Map<string, MessageSendFailure[]>());
  const proposalBusy = useRef(false);
  useEffect(() => {
    setProposalComposer(undefined);
    setProposalFailures(new Map());
    proposalBusy.current = false;
    setProposalSending(false);
  }, [snapshot.id]);
  function followUpProposal(thread: ThreadView, location: 'inline' | 'comments') {
    if (proposalBusy.current || otherBusy()) return;
    const id = `${thread.ref.writer}:${thread.ref.id}`;
    if (proposalComposer?.id === id && proposalComposer.location === location) return;
    closeAnnotation();
    binding.current?.markThreadStatusSeen?.(thread.ref);
    setProposalComposer({ id, key: crypto.randomUUID(), location });
  }
  function proposalCard(
    thread: ThreadView,
    detached = false,
    location: 'inline' | 'comments' = 'inline',
  ) {
    if (!thread.proposal) return;
    const id = `${thread.ref.writer}:${thread.ref.id}`;
    const localOutcome = proposalOutcomes.get(id);
    const records = conversationAsks(thread, view.asks ?? [], true);
    // Signed page outcomes own recovery and replace ephemeral adoption warnings.
    const recorded =
      localOutcome?.failure &&
      records.some((record) =>
        record.messageIds?.includes(localOutcome.failure!.input.context.message.id),
      );
    const outcome = recorded ? undefined : localOutcome;
    const status = presentationOf(view.threadPresentations, thread.ref)?.status;
    return (
      <ProposalCard
        thread={thread}
        outcome={outcome}
        statusError={proposalStatusErrors.has(id)}
        detached={detached}
        disabled={discussionBlocked || proposalSending || !snapshot.binding?.discussion}
        canDecide={!!status?.controllable && !!snapshot.binding?.discussion?.decideProposal}
        decide={(decision) => void proposalActions.decide(id, decision)}
        followUp={() => followUpProposal(thread, location)}
        resolve={
          status?.controllable && statusCoordinator
            ? () => {
                setProposalStatusErrors(
                  (previous) => new Set([...previous].filter((value) => value !== id)),
                );
                void statusCoordinator
                  .change(thread, !thread.resolved)
                  .catch(() => setProposalStatusErrors((previous) => new Set(previous).add(id)));
              }
            : undefined
        }
        composer={
          proposalComposer?.id === id &&
          proposalComposer.location === location && (
            <AnnotationInput
              key={proposalComposer.key}
              showCancel
              creationRecipient={thread.proposal.proposer}
              binding={snapshot.binding?.ask}
              discussion={snapshot.binding?.discussion}
              anchor={thread.anchor}
              thread={thread}
              asks={view.asks ?? []}
              title={view.title || snapshot.title}
              blocked={discussionBlocked}
              recoveryRequired={recoveryRequired}
              initialEdit={draft(id)}
              onDraft={(value, edit) => keepDraft(id, value.trim() || edit.edited ? edit : null)}
              onFailure={(failure) =>
                setProposalFailures((previous) =>
                  new Map(previous).set(id, [...(previous.get(id) ?? []), failure]),
                )
              }
              onBusy={(busy) => {
                proposalBusy.current = busy;
                setProposalSending(busy);
              }}
              cancel={() => {
                if (!proposalBusy.current) setProposalComposer(undefined);
              }}
              committed={() => {
                keepDraft(id, null);
                proposalBusy.current = false;
                setProposalSending(false);
                setProposalComposer(undefined);
              }}
            />
          )
        }
      >
        {thread.comments.map((comment) => (
          <CommentExchange
            key={`${comment.ref.writer}:${comment.messageId}`}
            comment={comment}
            thread={thread}
            binding={snapshot.binding?.discussion}
            ask={snapshot.binding?.ask}
            asks={view.asks ?? []}
            blocked={discussionBlocked}
            title={view.title || snapshot.title}
          />
        ))}
        {(proposalFailures.get(id) ?? [])
          .filter(
            (failure) =>
              !records.some(
                (record) =>
                  record.machine === failure.recipient.machine &&
                  record.agent === failure.recipient.agent &&
                  record.messageIds?.includes(failure.message),
              ),
          )
          .map((failure) => (
            <div
              key={`${failure.message}:${failure.recipient.agent}`}
              className="annotation-delivery-failure"
            >
              <p role="status">
                @{failure.agent} · {failure.uncertain ? text.askUnconfirmed : text.askNotDelivered}
              </p>
              {failure.uncertain ? (
                <p className="ask-supporting">{text.askUncertain}</p>
              ) : (
                <AskAgainAction
                  binding={snapshot.binding?.ask}
                  blocked={discussionBlocked || proposalSending}
                  input={failure.input}
                  recipient={failure.recipient}
                  retryOf={null}
                  settled={(result) =>
                    setProposalFailures((previous) =>
                      new Map(previous).set(
                        id,
                        (previous.get(id) ?? []).flatMap((value) =>
                          value !== failure
                            ? [value]
                            : result.adopted === true
                              ? []
                              : result.adopted === false
                                ? [value]
                                : [{ ...value, uncertain: true }],
                        ),
                      ),
                    )
                  }
                />
              )}
            </div>
          ))}
        {outcome?.failure && !outcome.failure.uncertain && (
          <AskAgainAction
            binding={snapshot.binding?.ask}
            blocked={discussionBlocked}
            input={outcome.failure.input}
            recipient={outcome.failure.recipient}
            retryOf={null}
          />
        )}
      </ProposalCard>
    );
  }
  const proposals = (view.threads ?? []).filter((thread) => thread.proposal && !thread.deleted);
  proposalContext.current = { discussionBlocked, snapshot, view, binding, renderer, host };
  const proposalIdentity = proposals.map((t) => `${t.ref.writer}:${t.ref.id}`).join(',');
  useEffect(() => {
    if (state !== 'ready') return;
    const nodes = [
      ...(proposalContext.current.host.current?.parentElement?.querySelectorAll<HTMLElement>(
        '[data-inline-proposal]',
      ) ?? []),
    ];
    let frame: number | undefined;
    let previous = '';
    const measure = () => {
      frame = undefined;
      const heights = nodes.map((node) => ({
        id: node.dataset.inlineProposal!,
        height: Math.min(2000, Math.ceil(node.getBoundingClientRect().height)),
      }));
      const next = JSON.stringify(heights);
      if (next !== previous) {
        previous = next;
        proposalContext.current.renderer.current?.slots(heights);
      }
    };
    const observer = new ResizeObserver(() => {
      if (frame === undefined) frame = requestAnimationFrame(measure);
    });
    nodes.forEach((node) => observer.observe(node));
    measure();
    return () => {
      observer.disconnect();
      if (frame !== undefined) cancelAnimationFrame(frame);
    };
  }, [state, view.source, proposalIdentity]);
  return {
    onSlots: setSlotPositions,
    inline: (
      <>
        {proposals
          .filter((thread) => proposals.filter((t) => t.threadId === thread.threadId).length === 1)
          .map((thread) => {
            const slot = slotPositions.find((slot) => slot.id === thread.threadId);
            return (
              <div
                key={`${thread.ref.writer}:${thread.ref.id}`}
                data-inline-proposal={thread.threadId}
                className="proposal-inline"
                data-detached={!slot || undefined}
                style={{ top: slot?.top ?? 0 }}
              >
                {proposalCard(thread)}
              </div>
            );
          })}
      </>
    ),
    renderProposal: (thread: ThreadView) =>
      proposalCard(thread, !slotPositions.some((slot) => slot.id === thread.threadId), 'comments'),
  };
}
