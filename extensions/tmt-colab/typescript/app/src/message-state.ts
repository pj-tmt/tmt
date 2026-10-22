/**
 * Core's state of one sent message (request-response-v1, Message state), as Remote carries it on a
 * pending `result` (#4317). Colab draws it and keeps no copy of the rules: the wording is Core's
 * `reason.text`, the symbol is one per state, and the countdown runs from the remainder Core gave.
 * It is a live observation, never a ledger record: the remainder changes every second.
 */
export const MESSAGE_STATES = [
  'sent',
  'batched',
  'delivered',
  'working',
  'unconfirmed',
  'not_sent',
] as const;
export type MessageStateName = (typeof MESSAGE_STATES)[number];
export interface MessageState {
  state: MessageStateName;
  /** `code` is an open set (an unknown one still shows its `text`); `text` is Core's wording. */
  reason: { code: string; text: string };
  /** The latest moment a batched message goes out, and the time left at the read; both null when
   * the recipient's Digest policy records no limit. */
  latestMs: number | null;
  remainingMs: number | null;
  /** Display only: Core's answer for the owner's own browser, `false` for every other grant. */
  canSendNow: boolean;
}
/** What the page shows of a Send now it asked for (#4136) while the message is in the state the
 * press was made in: that it is on its way, or why nothing came of it. `reason` is Core's code. */
export type SendNowProgress =
  | { phase: 'sending' }
  | {
      phase: 'outcome';
      outcome: 'refused' | 'unconfirmed' | 'unavailable';
      /** Core's code, and Core's own sentence for it when Remote carried one. */
      reason?: string;
      reasonText?: string;
    };
/** A state with the local time of the read, so the page counts down without a clock of Core's. */
export interface ObservedMessageState extends MessageState {
  receivedAt: number;
  sendNow?: SendNowProgress;
}

const REASON_CODE = /^[a-z][a-z0-9_]{0,63}$/;
const MAX_TEXT_BYTES = 512;
const encoder = new TextEncoder();
const record = (value: unknown): Record<string, unknown> | undefined =>
  value !== null && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;
const exactly = (value: Record<string, unknown>, keys: string[]) =>
  Object.keys(value).sort().join() === [...keys].sort().join();
const count = (value: unknown): value is number =>
  typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;

/**
 * The contract's object exactly, or `undefined`. The field is display only, so a value that does
 * not parse is dropped whole, never repaired or partly drawn, and the ask shows what it showed
 * without it.
 */
export function parseMessageState(value: unknown): MessageState | undefined {
  const row = record(value);
  if (!row || !exactly(row, ['state', 'reason', 'latestMs', 'remainingMs', 'canSendNow']))
    return undefined;
  const reason = record(row.reason);
  if (!reason || !exactly(reason, ['code', 'text'])) return undefined;
  const { code, text } = reason;
  const { state, latestMs, remainingMs, canSendNow } = row;
  if (
    !MESSAGE_STATES.some((name) => name === state) ||
    typeof code !== 'string' ||
    !REASON_CODE.test(code) ||
    typeof text !== 'string' ||
    text.length === 0 ||
    // One line of plain text: Core's wording is a sentence, never markup or a paragraph.
    /\p{Cc}/u.test(text) ||
    encoder.encode(text).length > MAX_TEXT_BYTES ||
    typeof canSendNow !== 'boolean'
  )
    return undefined;
  if (!((latestMs === null && remainingMs === null) || (count(latestMs) && count(remainingMs))))
    return undefined;
  return {
    state: state as MessageStateName,
    reason: { code, text },
    latestMs: latestMs as number | null,
    remainingMs: remainingMs as number | null,
    canSendNow,
  };
}

/** The states the page shows, by operation ID. */
export type MessageStates = Readonly<Record<string, ObservedMessageState>>;

/** What is left of the hold at `now` (the page's `performance.now()`), never below zero. */
export function remainingAt(observed: ObservedMessageState, now: number): number | null {
  if (observed.remainingMs === null) return null;
  return Math.max(0, observed.remainingMs - Math.max(0, now - observed.receivedAt));
}

/** A count of milliseconds as `m:ss`, the one format the Ops TUI uses for the same countdown. */
export function clock(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, '0')}`;
}

/** Core's wording is a lowercase fragment; the line it starts is a sentence. */
export function sentenceCase(text: string): string {
  return text.slice(0, 1).toUpperCase() + text.slice(1);
}

function sameSendNow(shown: SendNowProgress | undefined, read: SendNowProgress | undefined) {
  if (!shown || !read) return shown === read;
  return (
    shown.phase === read.phase &&
    (shown.phase === 'sending' ||
      (read.phase === 'outcome' &&
        shown.outcome === read.outcome &&
        shown.reason === read.reason &&
        shown.reasonText === read.reasonText))
  );
}

/** The most the page's countdown and Core's remainder may differ before a read is worth showing. */
const DRIFT_MS = 2000;

/**
 * Whether a later read says nothing the page does not already show: the same state, wording and
 * button, and a remainder within a couple of seconds of the page's own countdown. A read that is
 * the same changes nothing, so it republishes nothing.
 */
export function sameMessageState(
  shown: ObservedMessageState | undefined,
  read: ObservedMessageState | undefined,
): boolean {
  if (!shown || !read) return shown === read;
  const predicted = remainingAt(shown, read.receivedAt);
  return (
    shown.state === read.state &&
    shown.reason.code === read.reason.code &&
    shown.reason.text === read.reason.text &&
    shown.canSendNow === read.canSendNow &&
    sameSendNow(shown.sendNow, read.sendNow) &&
    shown.latestMs === read.latestMs &&
    (predicted === null || read.remainingMs === null
      ? predicted === read.remainingMs
      : Math.abs(predicted - read.remainingMs) < DRIFT_MS)
  );
}
