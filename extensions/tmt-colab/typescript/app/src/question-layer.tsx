import type { ReplyNotification } from './reply-notifications.js';
import type { ComposerConnectionPresentation } from './router.js';
import {
  Fragment,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type RefObject,
} from 'react';
import { QuestionGroupCard } from './components/question-group-card.js';
import { focusNeedsYouTarget, unansweredQuestions, type NeedsYouCandidate } from './needs-you.js';
import type { Question, QuestionGroup } from './questions.js';
import type { PageSnapshot, PageView } from './transport.js';
import type {
  AnchorPosition,
  QuestionAnchor,
  RenderState,
  ReviewRenderBlock,
  ReadingRenderBlock,
  mountRenderer,
} from './renderer.js';
import { text } from './strings.js';
import { verifiedQuestionStates } from './question-registry.js';
import type { CommentContext } from './thread-store.js';

/** The parent owns logical groups and viewer disclosures. The frame supplies quotes/geometry. */
export function useQuestionLayer({
  snapshot,
  view,
  renderer,
  state,
  contentRevision,
  slots,
  blocked,
  connectionPresentation,
  questions,
  groups,
  reviews,
  readings = [],
  onRecorded,
}: {
  questions: readonly Question[];
  groups: readonly QuestionGroup[];
  /** Review sections ride the same handshake: their quotes come back with the questions'. */
  reviews: readonly ReviewRenderBlock[];
  readings?: readonly ReadingRenderBlock[];
  /** A trusted Send recorded these answers (question IDs and their comments). */
  onRecorded?(questionIds: readonly string[], contexts: readonly CommentContext[]): void;
  snapshot: PageSnapshot;
  view: PageView;
  renderer: RefObject<Awaited<ReturnType<typeof mountRenderer>> | null>;
  state: RenderState | 'loading';
  /** Counts sources the open frame applied in place (#5022); each one sends the blocks again. */
  contentRevision: number;
  slots: readonly AnchorPosition[];
  blocked: boolean;
  connectionPresentation?: ComposerConnectionPresentation;
}) {
  const [anchors, setAnchors] = useState<QuestionAnchor[] | null>(null);
  const onQuestions = useCallback((next: QuestionAnchor[], checked = true) => {
    setAnchors(checked ? next : null);
  }, []);
  const [active, setActive] = useState<Record<string, string>>({});
  // One active disclosure per logical group; all group drafts remain parent-owned.
  const [opened, setOpened] = useState<ReadonlySet<string>>(new Set());
  const panels = useRef(new Map<string, HTMLElement>());
  const focusPanel = useRef<string | undefined>(undefined);
  const navigationFocus = useRef<string | undefined>(undefined);
  const threads = useMemo(
    () => (view.threads ?? []).filter((thread) => !thread.deleted && !thread.proposal),
    [view.threads],
  );
  const associated = useCallback(
    (id: string) =>
      threads.filter(
        (thread) =>
          thread.comments.some(
            (comment) =>
              !comment.deleted && comment.deviceAuthor === true && comment.questionId === id,
          ) ||
          anchors
            ?.find((anchor) => anchor.id === id)
            ?.threads.includes(`${thread.ref.writer}:${thread.ref.id}`),
      ),
    [anchors, threads],
  );
  const selected = useCallback(
    (group: QuestionGroup) =>
      group.questions.includes(active[group.id]) ? active[group.id] : group.questions[0],
    [active],
  );
  const renderedReviews = JSON.stringify(reviews);
  const renderedReadings = JSON.stringify(readings);
  // Logical groups retain one action owner; the renderer only needs physical placements.
  const renderedGroups = JSON.stringify(
    groups.flatMap((group) =>
      group.split
        ? group.questions.map((id) => ({
            id,
            title: null,
            questions: [id],
            active: id,
            form: 'chip',
          }))
        : [
            {
              id: group.id,
              title: group.title,
              questions: group.questions,
              active: selected(group),
              form: 'chip',
            },
          ],
    ),
  );
  const identity = JSON.stringify(
    threads.flatMap((thread) =>
      thread.anchor
        ? [
            {
              id: `${thread.ref.writer}:${thread.ref.id}`,
              selector: thread.anchor,
            },
          ]
        : [],
    ),
  );
  useLayoutEffect(() => {
    if (state !== 'ready') return;
    renderer.current?.questions(
      questions.map(({ id, title }) => ({ id, title })),
      JSON.parse(identity),
      JSON.parse(renderedGroups),
      JSON.parse(renderedReviews),
      JSON.parse(renderedReadings),
    );
  }, [
    state,
    contentRevision,
    questions,
    renderedGroups,
    renderedReviews,
    renderedReadings,
    identity,
    renderer,
  ]);
  const notificationThreads = useMemo(
    () =>
      !questions.length
        ? new Map<string, string | null>()
        : anchors === null
          ? undefined
          : (() => {
              const destinations = new Map<string, string | null>();
              for (const question of questions)
                for (const thread of associated(question.id)) {
                  const key = JSON.stringify([thread.ref.writer, thread.ref.id]);
                  destinations.set(key, destinations.has(key) ? null : question.id);
                }
              return destinations;
            })(),
    [questions, anchors, associated],
  );
  const setOpen = useCallback((id: string, open: boolean) => {
    setOpened((previous) => {
      if (previous.has(id) === open) return previous;
      const next = new Set(previous);
      if (open) next.add(id);
      else next.delete(id);
      return next;
    });
  }, []);
  useEffect(() => {
    // Opening hands focus to the panel; the next render has it visible.
    const id = focusPanel.current;
    if (!id || !opened.has(id)) return;
    focusPanel.current = undefined;
    const group = groups.find((group) => group.id === id);
    const target = panels.current
      .get(id)
      ?.querySelector<HTMLElement>(`[id="question-panel-${group && selected(group)}"]`);
    if (navigationFocus.current === id) navigationFocus.current = undefined;
    if (!focusNeedsYouTarget(target ?? undefined)) target?.focus({ preventScroll: true });
  }, [opened, active, groups, selected]);
  /** Opens an admitted question's disclosure, including a group ID's current member. */
  const open = useCallback(
    (id: string) => {
      const group = groups.find((value) => value.id === id || value.questions.includes(id));
      if (!group) return false;
      const target = group.questions.includes(id) ? id : selected(group);
      const position = slots.find((slot) => slot.id === (group.split ? target : group.id));
      if (position?.hidden) return false;
      setActive((previous) => ({
        ...previous,
        [group.id]: group.questions.includes(id) ? id : (previous[group.id] ?? group.questions[0]),
      }));
      focusPanel.current = group.id;
      setOpen(group.id, true);
      return true;
    },
    [groups, selected, slots, setOpen],
  );
  const rows = groups.map((group) => {
    const members = group.questions.map((id) => questions.find((question) => question.id === id)!);
    const agents = [...new Set(members.map((member) => member.agent).filter(Boolean))];
    return { group, members, agents, pending: unansweredQuestions(members, associated) };
  });
  const navigation: NeedsYouCandidate[] =
    state !== 'ready' || anchors === null
      ? []
      : rows.flatMap(({ group, members, agents, pending }) => {
          const ids = group.split ? group.questions : [group.id];
          const attached = ids.some((id) => {
            const slot = slots.find((value) => value.id === id);
            return !!slot?.chip && !slot.hidden;
          });
          if (
            !attached ||
            !members.every((member) => anchors.some((anchor) => anchor.id === member.id))
          )
            return [];
          return [
            {
              kind: 'answer',
              version: JSON.stringify([group, members]),
              questionCount: { waiting: pending.length, total: members.length },
              state: pending.length ? 'unanswered' : 'answered',
              id: group.id,
              title: group.title ?? members[0].title,
              agents,
              targetId: (pending[0] ?? members[0]).id,
            },
          ];
        });
  return {
    onQuestions,
    associated,
    open,
    navigation,
    version: (id: string) =>
      JSON.stringify([
        questions.find((question) => question.id === id),
        associated(id).map((thread) => [
          thread.ref,
          thread.comments.map((comment) => [comment.ref, comment.revision, comment.deleted]),
        ]),
      ]),
    attached: (id: string) => {
      const group = groups.find((group) => group.questions.includes(id));
      const target = group && !group.split ? group.id : id;
      return (
        !!anchors?.some((anchor) => anchor.id === id) &&
        slots.some((slot) => slot.id === target && !!slot.chip && !slot.hidden)
      );
    },
    ready: state === 'ready' && anchors !== null,
    jump: (id: string) => {
      const item = navigation.find((value) => value.id === id || value.targetId === id);
      if (!item) return false;
      navigationFocus.current = item.id;
      return open(item.targetId ?? item.id);
    },
    notificationThreads,
    activateNotification: (event: ReplyNotification) => {
      const group = groups.find((group) => group.questions.includes(event.questionId!));
      if (
        !group ||
        !slots.some(
          (slot) =>
            slot.id === (group.split ? event.questionId : group.id) && slot.chip !== undefined,
        )
      )
        return false;
      return open(event.questionId!);
    },
    inline: (
      <>
        {rows.map(({ group, members }) => {
          const current = selected(group);
          const slot = slots.find((value) => value.id === (group.split ? current : group.id));
          const attached = !!slot?.chip;
          return (
            <div
              key={group.id}
              ref={(node) => {
                if (node) panels.current.set(group.id, node);
                else panels.current.delete(group.id);
              }}
            >
              <QuestionGroupCard
                group={group}
                registry={view.questions}
                states={verifiedQuestionStates(view)}
                creator={
                  view.creator?.agent ??
                  (view.creationRecipient
                    ? {
                        id: view.creationRecipient.agentId,
                        name: view.originalAuthor ?? text.unknownAuthor,
                      }
                    : undefined)
                }
                members={members}
                selected={current}
                onSelect={(id) => setActive((previous) => ({ ...previous, [group.id]: id }))}
                expanded={opened.has(group.id)}
                onToggle={(id) => {
                  if (opened.has(group.id) && id === current) setOpen(group.id, false);
                  else {
                    focusPanel.current = group.id;
                    setActive((previous) => ({ ...previous, [group.id]: id }));
                    setOpen(group.id, true);
                  }
                }}
                onCollapse={() => setOpen(group.id, false)}
                renderBlock={(id, content) => {
                  const position = slots.find((value) => value.id === id);
                  return (
                    <Fragment>
                      <div
                        data-inline-slot={id}
                        data-inline-question={current}
                        className="proposal-inline question-inline question-compact-inline"
                        data-detached={!position?.chip || undefined}
                        style={{
                          display: position?.hidden ? 'none' : undefined,
                          top: position?.top ?? 0,
                          left: position?.left ?? 0,
                          width: position?.width ?? '100%',
                        }}
                      >
                        <span
                          data-inline-chip={id}
                          className="question-placement-anchor"
                          aria-hidden="true"
                        />
                        {content}
                      </div>
                    </Fragment>
                  );
                }}
                associated={associated}
                selectors={new Map((anchors ?? []).map((anchor) => [anchor.id, anchor.selector]))}
                attached={attached}
                placement={slot}
                discussion={snapshot.binding?.discussion}
                ask={snapshot.binding?.ask}
                asks={view.asks ?? []}
                title={view.title || snapshot.title}
                blocked={blocked}
                readOnly={view.archived}
                connectionPresentation={connectionPresentation}
                onRecorded={onRecorded}
              />
            </div>
          );
        })}
      </>
    ),
  };
}
