import { useEffect, useState, type RefObject } from 'react';
import { QuestionCard } from './components/question-card.js';
import type { Question } from './questions.js';
import type { PageSnapshot, PageView } from './transport.js';
import type { AnchorPosition, QuestionAnchor, RenderState, mountRenderer } from './renderer.js';

/** Question declarations come from admitted source. The frame supplies cosmetic quote/placement only. */
export function useQuestionLayer({
  snapshot,
  view,
  renderer,
  state,
  slots,
  blocked,
  questions,
}: {
  questions: readonly Question[];
  snapshot: PageSnapshot;
  view: PageView;
  renderer: RefObject<Awaited<ReturnType<typeof mountRenderer>> | null>;
  state: RenderState | 'loading';
  slots: readonly AnchorPosition[];
  blocked: boolean;
}) {
  const [anchors, onQuestions] = useState<QuestionAnchor[]>([]);
  const threads = (view.threads ?? []).filter(
    (thread) => !thread.deleted && !thread.proposal && thread.anchor,
  );
  const identity = JSON.stringify(
    threads.map((thread) => ({
      id: `${thread.ref.writer}:${thread.ref.id}`,
      selector: thread.anchor!,
    })),
  );
  useEffect(() => {
    onQuestions([]);
    if (state !== 'ready') return;
    renderer.current?.questions(
      questions.map(({ id, title }) => ({ id, title })),
      JSON.parse(identity),
    );
  }, [state, questions, identity, renderer]);
  return {
    onQuestions,
    inline: (
      <>
        {questions.map((question) => {
          const slot = slots.find((slot) => slot.id === question.id);
          const anchor = anchors.find((anchor) => anchor.id === question.id);
          const associated = threads.filter((thread) =>
            anchor?.threads.includes(`${thread.ref.writer}:${thread.ref.id}`),
          );
          return (
            <div
              key={question.id}
              data-inline-question={question.id}
              data-theme="light"
              data-inline-slot={question.id}
              className="proposal-inline question-inline"
              data-detached={!slot || undefined}
              style={{ top: slot?.top ?? 0 }}
            >
              <QuestionCard
                question={question}
                selector={anchor?.selector}
                threads={associated}
                discussion={snapshot.binding?.discussion}
                ask={snapshot.binding?.ask}
                asks={view.asks ?? []}
                title={view.title || snapshot.title}
                blocked={blocked || !slot}
              />
            </div>
          );
        })}
      </>
    ),
  };
}
