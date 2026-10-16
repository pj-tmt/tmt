import {
  canonicalQuestionThread,
  currentAnswer,
  type Question,
  type Review,
  questionText,
} from './questions.js';
import {
  COMMENT_BYTES,
  compareCommentOrder,
  type CommentView,
  type ThreadView,
} from './thread-records.js';
import { asksAbout } from './thread-store.js';
import type { PageAsk } from './ask-panel.js';

/** A Confirm note is one short line; a Needs-change note may be a paragraph. */
export const REVIEW_CONFIRM_NOTE_POINTS = 200;
export const REVIEW_CHANGE_NOTE_BYTES = 8192;

const bytes = (value: string) => new TextEncoder().encode(value).length;
const hasControl = (value: string) =>
  [...value].some(
    (character) => character.codePointAt(0)! < 32 || character === String.fromCharCode(127),
  );

/** The fixed plain-text comments a section's actions record. The title is the authored heading, so
 * the text names the section the way the page does and the reader sees it in Comments. */
export type SectionAction = 'confirmed' | 'changes' | 'answered';
const heads: Record<SectionAction, (title: string) => string> = {
  confirmed: (title) => `Confirmed “${title}”`,
  changes: (title) => `Needs change to “${title}”`,
  answered: (title) => `Answered “${title}”`,
};

/** `body` is what a comment carries after the fixed head; the page title and the visible recipient
 * mentions follow, as for an answer. Undefined when the comment would exceed its bounds. */
function sectionComment(
  action: SectionAction,
  review: Pick<Review, 'title'>,
  note: string,
  pageTitle: string,
  recipient: string,
): string | undefined {
  const head = heads[action](review.title);
  const comment = `${head}${note ? `: ${note}` : ''}\nPage: ${pageTitle}${recipient.trim() ? `\n${recipient.trim()}` : ''}`;
  return bytes(comment) <= COMMENT_BYTES ? comment : undefined;
}

/** `✓ Confirm and close`: an optional one-line note. */
export function confirmComment(
  review: Pick<Review, 'title'>,
  note: string,
  pageTitle: string,
  recipient: string,
): string | undefined {
  const text = questionText(note);
  if ([...text].length > REVIEW_CONFIRM_NOTE_POINTS || hasControl(text)) return;
  return sectionComment('confirmed', review, text, pageTitle, recipient);
}

/** `✎ Needs change…`: a required note, kept as typed apart from the outer whitespace. */
export function needsChangeComment(
  review: Pick<Review, 'title'>,
  note: string,
  pageTitle: string,
  recipient: string,
): string | undefined {
  const text = note.trim();
  if (!text || bytes(text) > REVIEW_CHANGE_NOTE_BYTES) return;
  return sectionComment('changes', review, text, pageTitle, recipient);
}

/** The opening comment of a section a reader's answers closed: no recipient, no note. */
export function answeredComment(
  review: Pick<Review, 'title'>,
  pageTitle: string,
): string | undefined {
  return sectionComment('answered', review, '', pageTitle, '');
}

/** Which action a comment body records for `review`, if any. The head must be followed by the
 * note separator, a line break or the end, so a longer title never matches a shorter one. */
export function sectionAction(
  review: Pick<Review, 'title'>,
  body: string,
): SectionAction | undefined {
  for (const action of ['confirmed', 'changes', 'answered'] as const) {
    const head = heads[action](review.title);
    if (!body.startsWith(head)) continue;
    const next = body.slice(head.length)[0];
    if (next === undefined || next === ':' || next === '\n') return action;
  }
}

/** The note a recorded section comment carries, without the page and recipient lines. */
export function sectionNote(review: Pick<Review, 'title'>, body: string): string | undefined {
  const action = sectionAction(review, body);
  if (!action) return;
  const rest = body.slice(heads[action](review.title).length);
  if (!rest.startsWith(': ')) return;
  const page = rest.lastIndexOf('\nPage: ');
  return (page >= 0 ? rest.slice(2, page) : rest.slice(2)) || undefined;
}

/** A verified device comment recording a section action; agent replies never count. */
export function currentSectionComment(
  review: Pick<Review, 'title'>,
  comments: readonly CommentView[],
) {
  return comments
    .filter(
      (comment) =>
        comment.deviceAuthor === true &&
        !comment.deleted &&
        sectionAction(review, comment.body) !== undefined,
    )
    .sort((a, b) =>
      compareCommentOrder(
        { sequence: Number(a.sequence ?? '0'), writer: a.ref.writer, id: a.ref.id },
        { sequence: Number(b.sequence ?? '0'), writer: b.ref.writer, id: b.ref.id },
      ),
    )
    .at(-1);
}

/** A closed section is one whose canonical thread's effective status is a person's Resolve. An
 * agent's `threads resolve` is labelled `agent` and leaves the section waiting. The label is the
 * publisher's assertion (not authentication), but page HTML cannot publish any status at all. */
export function resolvedByPerson(thread: ThreadView | undefined) {
  return (
    !!thread &&
    !thread.deleted &&
    thread.resolved &&
    thread.status?.resolved === true &&
    thread.status.actor === 'person'
  );
}

/** Concurrent first actions can create sibling threads. Like an answer block, prefer a thread that
 * holds a verified section comment, then writer/thread identity order. */
export function canonicalReviewThread<T extends Pick<ThreadView, 'ref' | 'comments'>>(
  review: Pick<Review, 'title'>,
  threads: readonly T[],
) {
  return [...threads].sort((a, b) => {
    const recorded =
      Number(!!currentSectionComment(review, b.comments)) -
      Number(!!currentSectionComment(review, a.comments));
    if (recorded) return recorded;
    const left = `${a.ref.writer}:${a.ref.id}`,
      right = `${b.ref.writer}:${b.ref.id}`;
    return left < right ? -1 : left > right ? 1 : 0;
  })[0];
}

export type ReviewState =
  | { kind: 'waiting' }
  | { kind: 'changes'; note?: string }
  | { kind: 'closed'; by?: string; at?: string; note?: string };

/**
 * Presentation state of one section, from its canonical thread only. `replied` holds the IDs of
 * comments an agent has answered, so a Needs-change note that was answered goes back to waiting.
 * Nothing here comes from page HTML, and nothing authorizes an action.
 */
export function reviewState(
  review: Pick<Review, 'title'>,
  thread: ThreadView | undefined,
  replied: ReadonlySet<string> = new Set(),
): ReviewState {
  const latest = thread && currentSectionComment(review, thread.comments);
  if (thread && resolvedByPerson(thread))
    return {
      kind: 'closed',
      by: thread.status?.deviceName,
      at: thread.status?.at,
      note:
        latest && sectionAction(review, latest.body) === 'confirmed'
          ? sectionNote(review, latest.body)
          : undefined,
    };
  if (latest && sectionAction(review, latest.body) === 'changes' && !replied.has(latest.messageId))
    return { kind: 'changes', note: sectionNote(review, latest.body) };
  return { kind: 'waiting' };
}

/** `n / m closed` for the page head. */
export function reviewProgress(states: readonly ReviewState[]) {
  return { closed: states.filter((state) => state.kind === 'closed').length, total: states.length };
}

/**
 * Whether a reader's answers have answered every question inside a section: the condition under
 * which the trusted Send that recorded them also closes the section. An empty section is never
 * closed this way, and agent replies never count (`currentAnswer` reads verified device answers).
 */
export function sectionAnswered(
  review: Pick<Review, 'questions'>,
  questions: readonly Question[],
  associated: (questionId: string) => readonly ThreadView[],
) {
  return (
    review.questions.length > 0 &&
    review.questions.every((id) => {
      const question = questions.find((candidate) => candidate.id === id);
      return (
        !!question &&
        !!currentAnswer(question, canonicalQuestionThread(question, associated(id))?.comments ?? [])
      );
    })
  );
}

/** The comments an agent has answered, from the page's admitted Ask records. A Needs-change note
 * leaves `changes requested` once its Ask has a reply. */
export function repliedComments(thread: ThreadView | undefined, asks: readonly PageAsk[]) {
  const replied = new Set<string>();
  for (const comment of thread?.comments ?? [])
    if (
      !comment.deleted &&
      asks.some((ask) => ask.reply !== undefined && asksAbout(ask, thread!, comment))
    )
      replied.add(comment.messageId);
  return replied;
}
