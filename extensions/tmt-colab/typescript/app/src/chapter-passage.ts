import type { NeedsYouCandidate } from './needs-you.js';

type ChapterTarget = Exclude<NeedsYouCandidate, { kind: 'proposal' }>;
export type ChapterTargetRef = Pick<ChapterTarget, 'id' | 'kind'>;
const priority = { review: 0, answer: 1, reading: 2 } as const;
const pending = { review: 'waiting', answer: 'unanswered', reading: 'unread' } as const;

/** The existing layers own admission and state. This selection owns only chapter presentation;
 * the parent supplies authored order, never an action or a recipient from frame geometry. */
export function chapterTargets(
  candidates: readonly NeedsYouCandidate[],
  positions: readonly { id: string; order: number }[],
): ChapterTarget[] {
  const seen = new Set<string>();
  const order = new Map(
    positions
      .filter((slot) => Number.isSafeInteger(slot.order) && slot.order >= 0)
      .map((slot) => [slot.id, slot.order]),
  );
  return candidates
    .filter((item): item is ChapterTarget => {
      if (item.kind === 'proposal' || seen.has(item.id) || !order.has(item.id)) return false;
      seen.add(item.id);
      return true;
    })
    .sort(
      (left, right) =>
        priority[left.kind] - priority[right.kind] ||
        order.get(left.id)! - order.get(right.id)! ||
        (left.id < right.id ? -1 : left.id > right.id ? 1 : 0),
    );
}

/** Keep the admitted passage's done state until the person asks for Next. A removed source
 * target cannot keep stale controls alive; only a target in the current layers can be retained. */
export function selectChapterTarget(
  targets: readonly ChapterTarget[],
  current?: ChapterTargetRef,
  advance = false,
): ChapterTarget | undefined {
  const retained = targets.find((item) => item.id === current?.id && item.kind === current.kind);
  if (retained && !advance) return retained;
  return (
    targets.find(
      (item) => item.state === pending[item.kind] && (!advance || item.id !== retained?.id),
    ) ?? retained
  );
}
