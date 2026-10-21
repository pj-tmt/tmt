import { exactKeys, requireValue } from '@tmt/colab-client';
import { SOURCE_BYTES } from './fold-protocol.js';
import { OwnAppendRefused } from './own-publication.js';
import { SAVE_HUNKS, type SaveHunk } from './save-hunks.js';
import { text } from './strings.js';

/** The most one page source can hold; mirrors decoder.rs `BASELINE_BYTES`. */
export const SAVE_SOURCE_BYTES = SOURCE_BYTES;

export type SaveState = 'committed' | 'unchanged' | 'absent' | 'pending' | 'rejected';
/** The one reply to a save or a status request (`saveresult`). */
export interface SaveResult {
  operationId: string;
  state: SaveState;
  revision?: string;
  /** Committed only: the page had changed since the editor opened it and the edit was merged. */
  merged?: true;
  /** Merged only: the digest of the source the commit holds, which the editor never had. */
  sourceSha256?: string;
  code?: string;
  message?: string;
  /** Rejected as `COLAB_PATCH_CONFLICT` only: the hunks, as sent (0-based), that do not fit. */
  conflicts?: number[];
  /** A `task.convert` only: the page the operation made, whatever became of its link. */
  childPage?: string;
}

/** The one reply to a `work` frame: a save's fields, plus the ID a committed `task.add` made. */
export interface WorkResult extends SaveResult {
  taskId?: string;
}
/** The closed work actions a browser sends (contract: Work); the serve refuses any other. */
export type WorkAction =
  | 'summary.set'
  | 'summary.clear'
  | 'task.add'
  | 'task.set'
  | 'task.state'
  | 'task.assign'
  | 'task.link'
  | 'task.unlink'
  | 'task.remove'
  /** Makes a page from the task, then links it; the one action that makes a page. */
  | 'task.convert';

/** The source is over the page limit; nothing was sent. */
export class SaveTooLarge extends Error {
  constructor(
    readonly size: number,
    readonly limit: number,
  ) {
    super('Source exceeds the page limit');
  }
}
/** The edit is over what a link can send as an edit request; nothing was sent. */
export class EditTooLarge extends SaveTooLarge {}
/** Native preparation or commit refused the save; nothing changed. */
export class SaveRefused extends Error {
  constructor(
    readonly code: string,
    message: string,
    readonly conflicts?: number[],
    /** A convert refused after its page was made: the page exists and the task is not linked. */
    readonly childPage?: string,
  ) {
    super(message);
  }
}
/** The page changed elsewhere where the edit is anchored, so nothing was saved and the draft is
 * kept. `first` names the first hunk that does not fit, in the source the editor opened. */
export class SaveConflict extends Error {
  constructor(
    readonly count: number,
    readonly first: { line: number; excerpt: string },
  ) {
    super('The edit conflicts with a change made elsewhere');
  }
}
/** The operation never reached commit: nothing changed, and a new save is a new operation. */
export class SaveNotApplied extends Error {
  constructor(
    readonly operationId: string,
    /** A convert whose page was made but whose link never landed. */
    readonly childPage?: string,
  ) {
    super('Save did not reach the page');
  }
}
/** The reply was lost and one status check could not settle it. Never resent automatically. */
export class SaveOutcomeUnknown extends Error {
  constructor(readonly operationId: string) {
    super('Save outcome unknown');
  }
}

const STATES: readonly string[] = ['committed', 'unchanged', 'absent', 'pending', 'rejected'];
export function saveResult(frame: Record<string, unknown>, operationId: string): SaveResult {
  const state = frame.state;
  requireValue(typeof state === 'string' && STATES.includes(state));
  exactKeys(frame, [
    'version',
    'type',
    'space',
    'page',
    'epoch',
    'operationId',
    'state',
    ...(state === 'committed' || state === 'unchanged' ? ['revision'] : []),
    ...(state === 'committed' && Object.hasOwn(frame, 'merged') ? ['merged', 'sourceSha256'] : []),
    ...(state === 'rejected' ? ['code'] : []),
    ...(state === 'rejected' && Object.hasOwn(frame, 'message') ? ['message'] : []),
    ...(state === 'rejected' && Object.hasOwn(frame, 'conflicts') ? ['conflicts'] : []),
    ...(Object.hasOwn(frame, 'childPage') ? ['childPage'] : []),
  ]);
  requireValue(frame.operationId === operationId);
  if (state === 'committed' || state === 'unchanged')
    // The revision is opaque (`v1:<hash>`): compared for equality, never parsed.
    requireValue(typeof frame.revision === 'string' && /^[\x21-\x7e]{1,128}$/.test(frame.revision));
  if (state === 'rejected')
    requireValue(
      typeof frame.code === 'string' &&
        /^[A-Z_]{1,64}$/.test(frame.code) &&
        (frame.message === undefined ||
          (typeof frame.message === 'string' && frame.message.length <= 512)),
    );
  // A made page is named only where a convert's outcome is final: committed, absent or rejected.
  requireValue(
    frame.childPage === undefined ||
      (typeof frame.childPage === 'string' &&
        /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(
          frame.childPage,
        ) &&
        (state === 'committed' || state === 'absent' || state === 'rejected')),
  );
  requireValue(frame.merged === undefined || frame.merged === true);
  if (frame.merged === true)
    requireValue(
      typeof frame.sourceSha256 === 'string' && /^[A-Za-z0-9_-]{43}$/.test(frame.sourceSha256),
    );
  const conflicts = frame.conflicts;
  if (conflicts !== undefined)
    // The hunks that do not fit, ascending and distinct, only for a patch conflict.
    requireValue(
      frame.code === 'COLAB_PATCH_CONFLICT' &&
        Array.isArray(conflicts) &&
        conflicts.length >= 1 &&
        conflicts.length <= SAVE_HUNKS &&
        conflicts.every(
          (at, i) =>
            Number.isSafeInteger(at) &&
            at >= 0 &&
            at < SAVE_HUNKS &&
            (i === 0 || at > conflicts[i - 1]),
        ),
    );
  return {
    operationId,
    state: state as SaveState,
    ...(typeof frame.revision === 'string' ? { revision: frame.revision } : {}),
    ...(frame.merged === true
      ? { merged: true as const, sourceSha256: frame.sourceSha256 as string }
      : {}),
    ...(typeof frame.code === 'string' ? { code: frame.code } : {}),
    ...(typeof frame.message === 'string' ? { message: frame.message } : {}),
    ...(Array.isArray(conflicts) ? { conflicts: conflicts as number[] } : {}),
    ...(typeof frame.childPage === 'string' ? { childPage: frame.childPage } : {}),
  };
}
export function workResult(frame: Record<string, unknown>, operationId: string): WorkResult {
  const { taskId, ...rest } = frame;
  requireValue(
    taskId === undefined ||
      (frame.state === 'committed' && typeof taskId === 'string' && /^T-[0-9]{1,7}$/.test(taskId)),
  );
  return { ...saveResult(rest, operationId), ...(typeof taskId === 'string' ? { taskId } : {}) };
}
/** A settled result as the outcome the caller sees: success returns, everything else throws. */
export function settled(result: SaveResult): SaveResult {
  // Still preparing on the page: the outcome is not final, and the save is never sent again.
  if (result.state === 'pending') throw new SaveOutcomeUnknown(result.operationId);
  if (result.state === 'absent') throw new SaveNotApplied(result.operationId, result.childPage);
  if (result.state === 'rejected')
    throw new SaveRefused(
      result.code ?? 'COLAB_UNAVAILABLE',
      result.message ?? '',
      result.conflicts,
      result.childPage,
    );
  return result;
}
/** How long a lost save waits for the page to reopen, and for the saved source to show. */
export const SAVE_REOPEN_MS = 20_000;
/** `settled` for a save that sent anchored `hunks`: a refusal that names hunks which no longer fit
 * is the conflict the first of them anchors, in the source the editor opened. */
export function settledAgainst(result: SaveResult, hunks?: readonly SaveHunk[]): SaveResult {
  try {
    return settled(result);
  } catch (error) {
    const conflicts = error instanceof SaveRefused ? (error.conflicts ?? []) : [];
    const named = conflicts.filter((at) => at < (hunks?.length ?? 0));
    if (hunks && named.length > 0)
      throw new SaveConflict(named.length, conflictAt(hunks[named[0]]));
    throw error;
  }
}
/** The sentence an editor shows for a save that did not succeed. */
export function saveMessage(error: unknown): string {
  if (error instanceof OwnAppendRefused) return text.linkEditRaced;
  if (error instanceof EditTooLarge) return text.linkEditTooLarge(error.size, error.limit);
  if (error instanceof SaveTooLarge) return text.saveTooLarge(error.size, error.limit);
  if (error instanceof SaveOutcomeUnknown) return text.saveUnknown(error.operationId);
  if (error instanceof SaveNotApplied) return text.saveNotApplied;
  if (error instanceof SaveConflict)
    return error.count === 1
      ? text.saveConflict(error.first.excerpt, error.first.line)
      : text.saveConflicts(error.count, error.first.excerpt, error.first.line);
  if (error instanceof SaveRefused)
    return error.code === 'COLAB_STALE_BASE' ? text.saveStale : error.message || text.editFailed;
  return text.editFailed;
}

/** The most characters of the first changed line a conflict quotes. */
const EXCERPT_CHARACTERS = 60;
/** Control characters and the bidi marks and overrides, which a quote must not carry. */
const INERT = /[\p{Cc}\u061c\u200e\u200f\u202a-\u202e\u2066-\u2069]/gu;
/**
 * What a conflict names: the first line of the hunk's base region that the editor changed (past
 * the context it carries) and holds text, whitespace collapsed and made inert, clipped to 60
 * characters, and its line number in the source the editor opened. The editor shows no line
 * numbers and the person's own edits shift lines, so the quote is what they search for.
 */
export function conflictAt(hunk: SaveHunk): { line: number; excerpt: string } {
  const split = (source: string) => {
    const lines = source.split('\n');
    if (lines.at(-1) === '') lines.pop();
    return lines;
  };
  const lines = split(hunk.old);
  const after = split(hunk.new);
  // The lines both sides share at the start are context; the change begins where they differ.
  let changed = 0;
  while (changed < lines.length - 1 && lines[changed] === after[changed]) changed++;
  const clean = (line: string) => line.replace(/\s+/gu, ' ').replace(INERT, '').trim();
  const named = [...lines.keys()]
    .filter((at) => at >= changed)
    .concat([...lines.keys()].filter((at) => at < changed))
    .find((at) => clean(lines[at]) !== '');
  const at = named ?? changed;
  const characters = Array.from(clean(lines[at] ?? ''));
  return {
    line: hunk.oldStart + at,
    excerpt:
      characters.length > EXCERPT_CHARACTERS
        ? `${characters.slice(0, EXCERPT_CHARACTERS).join('')}…`
        : characters.join(''),
  };
}
