import { questionText, type Reading } from './questions.js';
import {
  COMMENT_BYTES,
  compareCommentOrder,
  type CommentView,
  type ThreadView,
} from './thread-records.js';
import { resolvedByPerson } from './reviews.js';
import type { PageAsk } from './ask-panel.js';
import { projectStatusNotifications } from './thread-status-notification.js';
import { discussionRefKey } from './thread-status.js';

/** Fixed ordinary-device comment; the reading UUID separates identical headings. */
export function readingComment(
  reading: Reading,
  pageTitle: string,
  recipient: string,
): string | undefined {
  const body = `Read “${reading.title}”\nReading: ${reading.id}\nPage: ${questionText(pageTitle)}${recipient ? `\n${recipient}` : ''}`;
  return new TextEncoder().encode(body).length <= COMMENT_BYTES ? body : undefined;
}

/** Only the complete fixed form associates a device comment with this reading anchor. */
export function isReadingComment(reading: Pick<Reading, 'id' | 'title'>, body: string) {
  const prefix = `Read “${reading.title}”\nReading: ${reading.id}\nPage: `;
  if (!body.startsWith(prefix)) return false;
  const lines = body.slice(prefix.length).split('\n');
  return lines.length === 1 || (lines.length === 2 && /^@[^\r\n]+$/u.test(lines[1]));
}

export function currentReadingComment(
  reading: Pick<Reading, 'id' | 'title'>,
  comments: readonly CommentView[],
) {
  return comments
    .filter(
      (comment) =>
        comment.deviceAuthor === true &&
        !comment.deleted &&
        isReadingComment(reading, comment.body),
    )
    .sort((a, b) =>
      compareCommentOrder(
        { sequence: Number(a.sequence ?? '0'), writer: a.ref.writer, id: a.ref.id },
        { sequence: Number(b.sequence ?? '0'), writer: b.ref.writer, id: b.ref.id },
      ),
    )
    .at(-1);
}

/** An unassociated annotation with the same heading never inherits another reading's state. */
export function canonicalReadingThread<T extends Pick<ThreadView, 'ref' | 'comments'>>(
  reading: Pick<Reading, 'id' | 'title'>,
  threads: readonly T[],
) {
  return threads
    .filter((thread) => !!currentReadingComment(reading, thread.comments))
    .sort((a, b) => {
      const left = `${a.ref.writer}:${a.ref.id}`,
        right = `${b.ref.writer}:${b.ref.id}`;
      return left < right ? -1 : left > right ? 1 : 0;
    })[0];
}

export type ReadingState = { kind: 'unread' } | { kind: 'read'; by?: string; at?: string };
export function readingState(
  reading: Pick<Reading, 'id' | 'title'>,
  thread: ThreadView | undefined,
): ReadingState {
  if (!thread || !currentReadingComment(reading, thread.comments) || !resolvedByPerson(thread))
    return { kind: 'unread' };
  return { kind: 'read', by: thread.status?.deviceName, at: thread.status?.at };
}

/** Reuse the ordinary notification ledger; adoption alone is never delivery confirmation. */
export function readingNotification(thread: ThreadView | undefined, asks: readonly PageAsk[]) {
  if (!thread?.status) return;
  const current = discussionRefKey(thread.status.ref);
  return projectStatusNotifications(thread, asks).find(
    (outcome) => discussionRefKey(outcome.status) === current,
  );
}
