import { canonicalQuestionThread, currentAnswer, type Question } from './questions.js';
import type { ThreadView } from './thread-records.js';

type Target = { id: string; title: string; agent: string; targetId?: string };
export type NeedsYouCandidate = Target &
  (
    | { kind: 'proposal'; state: 'waiting' | 'settled' }
    | { kind: 'reading'; state: 'unread' | 'read' }
    | { kind: 'answer'; state: 'unanswered' | 'answered' }
    | { kind: 'review'; state: 'waiting' | 'changes' | 'closed' }
  );

/** Each admitted owner supplies its existing state. The header adds no independent state. */
export function pendingNeedsYou(candidates: readonly NeedsYouCandidate[]): NeedsYouCandidate[] {
  const seen = new Set<string>();
  const pending = {
    proposal: 'waiting',
    reading: 'unread',
    answer: 'unanswered',
    review: 'waiting',
  } as const;
  return (['proposal', 'reading', 'answer', 'review'] as const).flatMap((kind) =>
    candidates.filter((item) => {
      if (item.kind !== kind || item.state !== pending[kind] || seen.has(item.id)) return false;
      seen.add(item.id);
      return true;
    }),
  );
}

/** Shared by the chip's question count and the header's one-item-per-block projection. */
export function unansweredQuestions(
  questions: readonly Question[],
  associated: (id: string) => readonly Pick<ThreadView, 'ref' | 'comments'>[],
): Question[] {
  return questions.filter(
    (question) =>
      !currentAnswer(
        question,
        canonicalQuestionThread(question, associated(question.id))?.comments ?? [],
      ),
  );
}

/** Parent-drawn targets only; navigation never invokes their actions. */
export function focusNeedsYouTarget(target: HTMLElement | undefined): boolean {
  if (!target || !target.isConnected || !target.getClientRects().length) return false;
  target.scrollIntoView({ block: 'center', behavior: 'instant' });
  const action = [
    ...target.querySelectorAll<HTMLElement>(
      'button:not(:disabled), input:not(:disabled), textarea:not(:disabled), a[href]',
    ),
  ].find((node) => node.getClientRects().length);
  if (!action) target.tabIndex = -1;
  (action ?? target).focus({ preventScroll: true });
  return true;
}
