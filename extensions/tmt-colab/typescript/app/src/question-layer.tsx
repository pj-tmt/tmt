import type { ReplyNotification } from './reply-notifications.js';
import type { ComposerConnectionPresentation } from './router.js';
import { useCallback, useLayoutEffect, useMemo, useState, type RefObject } from 'react';
import { QuestionGroupCard } from './components/question-group-card.js';
import { type Question, type QuestionGroup } from './questions.js';
import type { PageSnapshot, PageView } from './transport.js';
import type {
  AnchorPosition,
  QuestionAnchor,
  RenderState,
  ReviewRenderBlock,
  mountRenderer,
} from './renderer.js';
import type { CommentContext } from './thread-store.js';

/** The parent owns admitted groups, verified answers and active tabs. The frame supplies quotes/geometry. */
export function useQuestionLayer({
  snapshot,
  view,
  renderer,
  state,
  slots,
  blocked,
  connectionPresentation,
  questions,
  groups,
  reviews,
  onRecorded,
}: {
  questions: readonly Question[];
  groups: readonly QuestionGroup[];
  /** Review sections ride the same handshake: their quotes come back with the questions'. */
  reviews: readonly ReviewRenderBlock[];
  /** A trusted Send recorded these answers (question IDs and their comments). */
  onRecorded?(questionIds: readonly string[], contexts: readonly CommentContext[]): void;
  snapshot: PageSnapshot;
  view: PageView;
  renderer: RefObject<Awaited<ReturnType<typeof mountRenderer>> | null>;
  state: RenderState | 'loading';
  slots: readonly AnchorPosition[];
  blocked: boolean;
  connectionPresentation?: ComposerConnectionPresentation;
}) {
  const [anchors, setAnchors] = useState<QuestionAnchor[] | null>(null);
  const onQuestions = useCallback((next: QuestionAnchor[], checked = true) => {
    setAnchors(checked ? next : null);
  }, []);
  const [active, setActive] = useState<Record<string, string>>({});
  const threads = useMemo(
    () =>
      (view.threads ?? []).filter((thread) => !thread.deleted && !thread.proposal && thread.anchor),
    [view.threads],
  );
  const associated = useCallback(
    (id: string) =>
      threads.filter((thread) =>
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
  const renderedGroups = JSON.stringify(
    groups.map((group) => ({ ...group, active: selected(group) })),
  );
  const identity = JSON.stringify(
    threads.map((thread) => ({
      id: `${thread.ref.writer}:${thread.ref.id}`,
      selector: thread.anchor!,
    })),
  );
  useLayoutEffect(() => {
    if (state !== 'ready') return;
    renderer.current?.questions(
      questions.map(({ id, title }) => ({ id, title })),
      JSON.parse(identity),
      JSON.parse(renderedGroups),
      JSON.parse(renderedReviews),
    );
  }, [state, questions, renderedGroups, renderedReviews, identity, renderer]);
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
  return {
    onQuestions,
    associated,
    notificationThreads,
    activateNotification: (event: ReplyNotification) => {
      const group = groups.find((group) => group.questions.includes(event.questionId!));
      if (
        !group ||
        !slots.some((slot) => slot.id === group.id && slot.descriptionHeight !== undefined)
      )
        return false;
      setActive((previous) => ({ ...previous, [group.id]: event.questionId! }));
      return true;
    },
    inline: (
      <>
        {groups.map((group) => {
          const members = group.questions.map((id) =>
            questions.find((question) => question.id === id)!,
          );
          const current = selected(group);
          const slot = slots.find((slot) => slot.id === group.id);
          const attached = !!slot && slot.descriptionHeight !== undefined;
          return (
            <div
              key={group.id}
              data-inline-question={current}
              data-question-group={group.id}
              data-inline-slot={group.id}
              className="proposal-inline question-inline"
              data-detached={!attached || undefined}
              data-tab-placement={slot?.beforeTop !== undefined || undefined}
              style={{
                top: slot?.beforeTop ?? slot?.top ?? 0,
                left: slot?.left,
                width: slot?.width,
              }}
              onKeyDown={(event) => {
                if (
                  !event.isTrusted ||
                  event.defaultPrevented ||
                  event.altKey ||
                  event.ctrlKey ||
                  event.metaKey ||
                  event.shiftKey ||
                  !/^[1-9]$/.test(event.key)
                )
                  return;
                const target = event.target as HTMLElement;
                if (
                  target.closest(
                    'input,textarea,[contenteditable],[role="combobox"],[role="listbox"]',
                  )
                )
                  return;
                if (!target.closest('[role="tablist"],[role="tabpanel"],.question-option')) return;
                const option = [
                  ...event.currentTarget.querySelectorAll<HTMLInputElement>(
                    `[data-choice-index="${event.key}"] input`,
                  ),
                ].find((input) => !input.disabled && !input.closest('[hidden]'));
                if (option) {
                  event.preventDefault();
                  option.click();
                }
              }}
            >
              <QuestionGroupCard
                key={`${snapshot.id}:${group.id}`}
                group={group}
                members={members}
                selected={current}
                onSelect={(id) => setActive((previous) => ({ ...previous, [group.id]: id }))}
                associated={associated}
                selectors={
                  new Map(
                    attached ? (anchors ?? []).map((anchor) => [anchor.id, anchor.selector]) : [],
                  )
                }
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
