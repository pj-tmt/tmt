import { exactKeys, requireValue } from '@tmt/colab-client';
import { SOURCE_BYTES } from './fold-protocol.js';
import { text } from './strings.js';

/** The most one page source can hold; mirrors decoder.rs `BASELINE_BYTES`. */
export const SAVE_SOURCE_BYTES = SOURCE_BYTES;

export type SaveState = 'committed' | 'unchanged' | 'absent' | 'rejected';
/** The one reply to a save or a status request (`saveresult`). */
export interface SaveResult {
  operationId: string;
  state: SaveState;
  revision?: string;
  code?: string;
  message?: string;
}

/** The source is over the page limit; nothing was sent. */
export class SaveTooLarge extends Error {
  constructor(
    readonly size: number,
    readonly limit: number,
  ) {
    super('Source exceeds the page limit');
  }
}
/** Native preparation or commit refused the save; nothing changed. */
export class SaveRefused extends Error {
  constructor(
    readonly code: string,
    message: string,
  ) {
    super(message);
  }
}
/** The operation never reached commit: nothing changed, and a new save is a new operation. */
export class SaveNotApplied extends Error {
  constructor(readonly operationId: string) {
    super('Save did not reach the page');
  }
}
/** The reply was lost and one status check could not settle it. Never resent automatically. */
export class SaveOutcomeUnknown extends Error {
  constructor(readonly operationId: string) {
    super('Save outcome unknown');
  }
}

const STATES: readonly string[] = ['committed', 'unchanged', 'absent', 'rejected'];
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
    ...(state === 'rejected' ? ['code'] : []),
    ...(state === 'rejected' && Object.hasOwn(frame, 'message') ? ['message'] : []),
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
  return {
    operationId,
    state: state as SaveState,
    ...(typeof frame.revision === 'string' ? { revision: frame.revision } : {}),
    ...(typeof frame.code === 'string' ? { code: frame.code } : {}),
    ...(typeof frame.message === 'string' ? { message: frame.message } : {}),
  };
}
/** A settled result as the outcome the caller sees: success returns, everything else throws. */
export function settled(result: SaveResult): SaveResult {
  if (result.state === 'absent') throw new SaveNotApplied(result.operationId);
  if (result.state === 'rejected')
    throw new SaveRefused(result.code ?? 'COLAB_UNAVAILABLE', result.message ?? '');
  return result;
}
/** The sentence an editor shows for a save that did not succeed. */
export function saveMessage(error: unknown): string {
  if (error instanceof SaveTooLarge) return text.saveTooLarge(error.size, error.limit);
  if (error instanceof SaveOutcomeUnknown) return text.saveUnknown(error.operationId);
  if (error instanceof SaveNotApplied) return text.saveNotApplied;
  if (error instanceof SaveRefused)
    return error.code === 'COLAB_STALE_BASE' ? text.saveStale : error.message || text.editFailed;
  return text.editFailed;
}
