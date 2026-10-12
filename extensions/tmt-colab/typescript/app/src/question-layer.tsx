import type { ComposerConnectionPresentation } from './router.js';
import { BrowserTabs, type BrowserTab } from '@tmt/browser-ui/react';
import { useCallback, useLayoutEffect, useMemo, useState, type RefObject } from 'react';
import { QuestionCard } from './components/question-card.js';
import {
  canonicalQuestionThread,
  currentAnswer,
  type Question,
  type QuestionGroup,
} from './questions.js';
import type { PageSnapshot, PageView } from './transport.js';
import type { AnchorPosition, QuestionAnchor, RenderState, mountRenderer } from './renderer.js';

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
}: {
  questions: readonly Question[];
  groups: readonly QuestionGroup[];
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
  const answers = useMemo(
    () =>
      new Set(
        questions
          .filter((question) =>
            currentAnswer(
              question,
              canonicalQuestionThread(question, associated(question.id))?.comments ?? [],
            ),
          )
          .map((question) => question.id),
      ),
    [questions, associated],
  );
  const answered = (question: Question) => answers.has(question.id);
  const selected = useCallback(
    (group: QuestionGroup) =>
      group.questions.includes(active[group.id])
        ? active[group.id]
        : (group.questions.find((id) => !answers.has(id)) ?? group.questions[0]),
    [active, answers],
  );
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
    );
  }, [state, questions, renderedGroups, identity, renderer]);
  return {
    onQuestions,
    inline: (
      <>
        {groups.map((group) => {
          const members = group.questions.map((id) =>
            questions.find((question) => question.id === id)!,
          );
          const current = selected(group);
          const slot = slots.find((slot) => slot.id === group.id);
          const attached = !!slot && slot.descriptionHeight !== undefined;
          const count = members.filter(answered).length;
          const tabs: BrowserTab[] = members.map((question, index) => ({
            id: question.id,
            tabId: `question-tab-${question.id}`,
            panelId: `question-panel-${question.id}`,
            label: question.label ?? `Q${index + 1}`,
            state: answered(question)
              ? { mark: '✓', word: 'Answered', tone: 'working' }
              : { mark: '◆', word: 'Waiting', tone: 'waiting' },
          }));
          const card = (question: Question) => (
            <QuestionCard
              connectionPresentation={connectionPresentation}
              key={question.id}
              readOnly={view.archived}
              question={question}
              selector={
                attached
                  ? anchors?.find((anchor) => anchor.id === question.id)?.selector
                  : undefined
              }
              threads={associated(question.id)}
              discussion={snapshot.binding?.discussion}
              ask={snapshot.binding?.ask}
              asks={view.asks ?? []}
              title={view.title || snapshot.title}
              blocked={blocked}
              descriptionHeight={slot?.descriptionHeight ?? 0}
              sendNext={members.some((member) => member.id !== question.id && !answered(member))}
              onVerified={() => {
                // A committed verified device answer, never a callback success or frame claim, advances.
                if (group.questions.includes(active[group.id]) && active[group.id] !== question.id)
                  return;
                const index = members.findIndex((member) => member.id === question.id);
                const next = [...members.slice(index + 1), ...members.slice(0, index)].find(
                  (member) => !answered(member),
                );
                if (next) {
                  const tab = document.getElementById(`question-tab-${question.id}`);
                  const ownsFocus = tab
                    ?.closest('[data-question-group]')
                    ?.contains(document.activeElement);
                  setActive((previous) => ({ ...previous, [group.id]: next.id }));
                  if (ownsFocus)
                    queueMicrotask(() =>
                      document
                        .getElementById(`question-tab-${next.id}`)
                        ?.focus({ preventScroll: true }),
                    );
                }
              }}
            />
          );
          return (
            <div
              key={group.id}
              data-inline-question={current}
              data-question-group={group.id}
              data-theme="light"
              data-inline-slot={group.id}
              className="proposal-inline question-inline"
              data-detached={!attached || undefined}
              data-tab-placement={slot?.beforeTop !== undefined || undefined}
              style={{ top: slot?.beforeTop ?? slot?.top ?? 0 }}
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
              {members.length > 1 ? (
                <BrowserTabs
                  label={group.title ?? 'Questions'}
                  tabs={tabs}
                  selected={current}
                  status={`${count}/${members.length} answered`}
                  onSelect={(id, _reason, event) => {
                    if (event.isTrusted) setActive((previous) => ({ ...previous, [group.id]: id }));
                  }}
                >
                  {(tab) => card(members.find((question) => question.id === tab.id)!)}
                </BrowserTabs>
              ) : (
                <div className="question-group-card">{card(members[0])}</div>
              )}
            </div>
          );
        })}
      </>
    ),
  };
}
