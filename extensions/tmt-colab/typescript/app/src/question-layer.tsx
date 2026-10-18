import type { ReplyNotification } from './reply-notifications.js';
import type { ComposerConnectionPresentation } from './router.js';
import { BrowserAction } from '@tmt/browser-ui/react';
import {
  Fragment,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type RefObject,
} from 'react';
import { NeedsYouBar } from './components/needs-you-bar.js';
import { QuestionChip } from './components/question-chip.js';
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
  slots: readonly AnchorPosition[];
  blocked: boolean;
  connectionPresentation?: ComposerConnectionPresentation;
}) {
  const [anchors, setAnchors] = useState<QuestionAnchor[] | null>(null);
  const onQuestions = useCallback((next: QuestionAnchor[], checked = true) => {
    setAnchors(checked ? next : null);
  }, []);
  const [active, setActive] = useState<Record<string, string>>({});
  // Chip-form panels the reader has opened. A panel stays mounted while collapsed, so its drafts survive.
  const [opened, setOpened] = useState<ReadonlySet<string>>(new Set());
  const chips = useRef(new Map<string, HTMLButtonElement>());
  const panels = useRef(new Map<string, HTMLElement>());
  const focusPanel = useRef<string | undefined>(undefined);
  const navigationFocus = useRef<string | undefined>(undefined);
  const panelPrefix = useId();
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
  const renderedReadings = JSON.stringify(readings);
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
      JSON.parse(renderedReadings),
    );
  }, [state, questions, renderedGroups, renderedReviews, renderedReadings, identity, renderer]);
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
    if (!id || (groups.find((group) => group.id === id)?.form === 'chip' && !opened.has(id)))
      return;
    focusPanel.current = undefined;
    if (navigationFocus.current === id) {
      navigationFocus.current = undefined;
      focusNeedsYouTarget(panels.current.get(id));
    } else
      panels.current.get(id)?.querySelector<HTMLElement>('.question-group-card')?.focus({
        preventScroll: true,
      });
  }, [opened, active, groups]);
  /** Opens the chip-form panel of a group (or of the group holding a question) and selects that
   * question. False when the ID names no chip-form group, so a caller can fall back to jumping. */
  const open = useCallback(
    (id: string) => {
      const group = groups.find((value) => value.id === id || value.questions.includes(id));
      if (!group || group.form !== 'chip') return false;
      if (group.questions.includes(id)) setActive((previous) => ({ ...previous, [group.id]: id }));
      focusPanel.current = group.id;
      setOpen(group.id, true);
      return true;
    },
    [groups, setOpen],
  );
  const rows = groups.map((group) => {
    const members = group.questions.map((id) => questions.find((question) => question.id === id)!);
    return { group, members, pending: unansweredQuestions(members, associated) };
  });
  const navigation: NeedsYouCandidate[] =
    state !== 'ready' || anchors === null
      ? []
      : rows.flatMap(({ group, members, pending }) => {
          const slot = slots.find((value) => value.id === group.id);
          const attached =
            group.form === 'chip' ? !!slot?.chip : slot?.descriptionHeight !== undefined;
          if (
            !attached ||
            !members.every((member) => anchors.some((anchor) => anchor.id === member.id))
          )
            return [];
          return [
            {
              kind: 'answer',
              state: pending.length ? 'unanswered' : 'answered',
              id: group.id,
              title: group.title ?? members[0].title,
              agent: [...new Set(members.map((member) => member.agent).filter(Boolean))].join(', '),
              targetId: (pending[0] ?? members[0]).id,
            },
          ];
        });
  return {
    onQuestions,
    associated,
    open,
    navigation,
    ready: state === 'ready' && anchors !== null,
    jump: (id: string) => {
      const item = navigation.find((value) => value.id === id || value.targetId === id);
      if (!item) return false;
      const group = groups.find((value) => value.id === item.id)!;
      if (group.form === 'chip') {
        open(item.targetId ?? item.id);
        navigationFocus.current = group.id;
        return true;
      }
      focusPanel.current = group.id;
      navigationFocus.current = group.id;
      setActive((previous) => ({ ...previous, [group.id]: item.targetId ?? group.questions[0] }));
      return true;
    },
    notificationThreads,
    activateNotification: (event: ReplyNotification) => {
      const group = groups.find((group) => group.questions.includes(event.questionId!));
      if (
        !group ||
        !slots.some(
          (slot) =>
            slot.id === group.id &&
            (group.form === 'chip'
              ? slot.chip !== undefined
              : slot.descriptionHeight !== undefined),
        )
      )
        return false;
      if (group.form === 'chip') return open(event.questionId!);
      setActive((previous) => ({ ...previous, [group.id]: event.questionId! }));
      return true;
    },
    inline: (
      <>
        {rows.map(({ group, members, pending }) => {
          const current = selected(group);
          const slot = slots.find((slot) => slot.id === group.id);
          const chip = group.form === 'chip';
          const attached = chip ? !!slot?.chip : !!slot && slot.descriptionHeight !== undefined;
          const isOpen = opened.has(group.id);
          const panelId = `${panelPrefix}-panel-${group.id}`;
          const close = () => {
            setOpen(group.id, false);
            chips.current.get(group.id)?.focus({ preventScroll: true });
          };
          const unanswered = chip ? pending.length : 0;
          const panel = (
            <div
              key={chip ? undefined : group.id}
              id={chip ? panelId : undefined}
              ref={(node) => {
                if (node) panels.current.set(group.id, node);
                else panels.current.delete(group.id);
              }}
              hidden={chip && !isOpen ? true : undefined}
              data-inline-question={current}
              data-question-group={group.id}
              data-inline-slot={group.id}
              className="proposal-inline question-inline"
              data-detached={!attached || undefined}
              data-tab-placement={slot?.beforeTop !== undefined || undefined}
              style={{
                top: chip ? (slot?.top ?? 0) : (slot?.beforeTop ?? slot?.top ?? 0),
                left: slot?.left,
                width: slot?.width,
              }}
              onKeyDown={(event) => {
                if (
                  chip &&
                  event.key === 'Escape' &&
                  event.isTrusted &&
                  !event.defaultPrevented &&
                  !event.repeat &&
                  !event.nativeEvent.isComposing &&
                  event.keyCode !== 229 &&
                  !event.altKey &&
                  !event.ctrlKey &&
                  !event.metaKey &&
                  !event.shiftKey
                ) {
                  event.preventDefault();
                  close();
                  return;
                }
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
              {chip && (
                <NeedsYouBar
                  role="status"
                  tone={unanswered ? 'needs' : 'done'}
                  word={
                    unanswered
                      ? `Please answer ${unanswered} ${unanswered === 1 ? 'question' : 'questions'}`
                      : `Answered ${members.length} ${members.length === 1 ? 'question' : 'questions'}`
                  }
                  who={[...new Set(members.map((member) => member.agent).filter(Boolean))].join(
                    ', ',
                  )}
                  actions={
                    <BrowserAction
                      label="Collapse"
                      type="button"
                      variant="text"
                      onActivate={(event) => {
                        if (event.isTrusted) close();
                      }}
                    />
                  }
                />
              )}
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
                chip={chip}
                connectionPresentation={connectionPresentation}
                onRecorded={onRecorded}
              />
            </div>
          );
          if (!chip) return panel;
          return (
            <Fragment key={group.id}>
              <div
                data-inline-chip={group.id}
                className="proposal-inline question-chip-inline"
                data-detached={!slot?.chip || undefined}
                style={{ top: slot?.chip?.top ?? 0, left: slot?.chip?.left ?? 0 }}
              >
                <QuestionChip
                  ref={(node) => {
                    if (node) chips.current.set(group.id, node);
                    else chips.current.delete(group.id);
                  }}
                  answered={unanswered === 0}
                  count={unanswered || members.length}
                  open={isOpen}
                  panelId={panelId}
                  onToggle={() => {
                    if (isOpen) close();
                    else open(group.id);
                  }}
                />
              </div>
              {panel}
            </Fragment>
          );
        })}
      </>
    ),
  };
}
