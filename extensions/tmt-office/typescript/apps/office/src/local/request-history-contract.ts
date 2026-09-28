import { canonicalUuid, exactRecord } from '../contracts/record.js';
import { DISPATCH_MESSAGE_LIMIT, requestIdentifier } from './dispatch-contract.js';
import type { DispatchReceipt } from './dispatch-contract.js';

export const REQUEST_HISTORY_LIMIT = 20;
export const REQUEST_HISTORY_MAX_LIMIT = 50;
export const REQUEST_HISTORY_PREVIEW_CHARS = 160;
export const REQUEST_HISTORY_PAGE_BYTES = 256 * 1024;
// Two exact 1 MiB UTF-8 bodies, worst-case JSON escaping, plus bounded metadata.
export const REQUEST_DETAIL_BYTES = DISPATCH_MESSAGE_LIMIT * 12 + 8192;

export interface HistoryCursor {
  preparedAtMs: number;
  requestId: string;
}
export type HistoryScope =
  | { recipientId: string; roomId?: string }
  | { roomId: string; recipientId?: string };
export type HistoryQuery = HistoryScope & { limit?: number; before?: HistoryCursor };
export type RequestFinal<T> =
  | { status: 'not_submitted' | 'not_required' }
  | { status: 'expired' | 'unavailable'; submittedAtMs: number; expiresAtMs: number }
  | {
      status: 'retained';
      submittedAtMs: number;
      expiresAtMs: number;
      bodyBytes: number;
      content: T;
    };
export interface HistoryItem<T> {
  requestId: string;
  roomId: string | null;
  recipientId: string | null;
  sender:
    | { kind: 'unknown'; identityId: null }
    | { kind: 'explicit' | 'verified'; identityId: string };
  kind: 'request' | 'announcement';
  preparedAtMs: number;
  delivery: 'prepared' | 'sending' | 'sent' | 'queued' | 'uncertain' | 'definitely_failed';
  recipientAcknowledged: boolean | null;
  final: RequestFinal<T>;
}
export type HistorySummary = HistoryItem<undefined> & { preview: string | null };
export interface HistoryPage {
  items: HistorySummary[];
  nextBefore: HistoryCursor | null;
}
export type RequestPrompt =
  | { status: 'unavailable' }
  | { status: 'expired'; expiresAtMs: number }
  | { status: 'retained'; message: string; messageBytes: number; expiresAtMs: number };
export type HistoryDetail = HistoryItem<string> & { prompt: RequestPrompt };
export interface RequestHistoryPort {
  list(query: HistoryQuery, signal?: AbortSignal): Promise<HistoryPage>;
  show(requestId: string, signal?: AbortSignal): Promise<HistoryDetail>;
  receipt(operationId: string, signal?: AbortSignal): Promise<DispatchReceipt | null>;
}

function integer(value: unknown, min: number, max = Number.MAX_SAFE_INTEGER): number {
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < min || value > max)
    throw new Error('Invalid request history number.');
  return value;
}
function cursor(value: unknown): HistoryCursor {
  const row = exactRecord(value, ['preparedAtMs', 'requestId'], 'history cursor');
  return {
    preparedAtMs: integer(row.preparedAtMs, 1),
    requestId: requestIdentifier(row.requestId),
  };
}
function precedes(a: HistoryCursor, b: HistoryCursor): boolean {
  return (
    a.preparedAtMs < b.preparedAtMs ||
    (a.preparedAtMs === b.preparedAtMs && a.requestId < b.requestId)
  );
}
export function decodeHistoryQuery(value: unknown): HistoryQuery & { limit: number } {
  const keys = ['recipientId', 'roomId', 'limit', 'before'].filter(
    (key) => value != null && typeof value === 'object' && Object.hasOwn(value, key)
  );
  const row = exactRecord(value, keys, 'history query');
  const recipientId = row.recipientId === undefined ? undefined : canonicalUuid(row.recipientId);
  const roomId = row.roomId === undefined ? undefined : canonicalUuid(row.roomId);
  if (!recipientId && !roomId) throw new Error('Choose an agent or room for request history.');
  const scope: HistoryScope = recipientId
    ? { recipientId, ...(roomId ? { roomId } : {}) }
    : { roomId: roomId! };
  return {
    ...scope,
    limit:
      row.limit === undefined
        ? REQUEST_HISTORY_LIMIT
        : integer(row.limit, 1, REQUEST_HISTORY_MAX_LIMIT),
    ...(row.before === undefined ? {} : { before: cursor(row.before) }),
  };
}

function exactText(value: unknown, bytes: number): string {
  if (
    typeof value !== 'string' ||
    /[\uD800-\uDFFF]/u.test(value) ||
    new TextEncoder().encode(value).length !== bytes
  )
    throw new Error('Invalid exact request text.');
  return value;
}
function decodeFinal<T>(
  value: unknown,
  contentKeys: string[],
  read: (row: Record<string, unknown>, bytes: number) => T
): RequestFinal<T> {
  if (!value || typeof value !== 'object' || !('status' in value))
    throw new Error('Invalid request final.');
  const status = value.status;
  if (status === 'not_submitted' || status === 'not_required') {
    exactRecord(value, ['status'], 'request final');
    return { status };
  }
  if (status !== 'retained' && status !== 'expired' && status !== 'unavailable')
    throw new Error('Invalid request final state.');
  const row = exactRecord(
    value,
    [
      'status',
      'submittedAtMs',
      'expiresAtMs',
      ...(status === 'retained' ? ['bodyBytes', ...contentKeys] : []),
    ],
    'request final'
  );
  const submittedAtMs = integer(row.submittedAtMs, 1);
  const expiresAtMs = integer(row.expiresAtMs, submittedAtMs);
  if (status !== 'retained') return { status, submittedAtMs, expiresAtMs };
  const bodyBytes = integer(row.bodyBytes, 0, DISPATCH_MESSAGE_LIMIT);
  return { status, submittedAtMs, expiresAtMs, bodyBytes, content: read(row, bodyBytes) };
}
function historyItem<T>(row: Record<string, unknown>, final: RequestFinal<T>): HistoryItem<T> {
  const sender = exactRecord(row.sender, ['kind', 'identityId'], 'request sender');
  if (sender.kind !== 'unknown' && sender.kind !== 'explicit' && sender.kind !== 'verified')
    throw new Error('Invalid request sender.');
  if (sender.kind === 'unknown' && sender.identityId !== null)
    throw new Error('Unknown sender must not name an identity.');
  if (row.kind !== 'request' && row.kind !== 'announcement')
    throw new Error('Invalid request kind.');
  if ((row.kind === 'announcement') !== (final.status === 'not_required'))
    throw new Error('Inconsistent reply requirement.');
  const delivery = row.delivery;
  if (
    delivery !== 'prepared' &&
    delivery !== 'sending' &&
    delivery !== 'sent' &&
    delivery !== 'queued' &&
    delivery !== 'uncertain' &&
    delivery !== 'definitely_failed'
  )
    throw new Error('Invalid request delivery.');
  if (row.recipientAcknowledged !== null && typeof row.recipientAcknowledged !== 'boolean')
    throw new Error('Invalid request acknowledgment.');
  return {
    requestId: requestIdentifier(row.requestId),
    roomId: row.roomId === null ? null : canonicalUuid(row.roomId),
    recipientId: row.recipientId === null ? null : canonicalUuid(row.recipientId),
    sender:
      sender.kind === 'unknown'
        ? { kind: 'unknown', identityId: null }
        : { kind: sender.kind, identityId: canonicalUuid(sender.identityId) },
    kind: row.kind,
    preparedAtMs: integer(row.preparedAtMs, 1),
    delivery,
    recipientAcknowledged: row.recipientAcknowledged,
    final,
  };
}
const ITEM_FIELDS = [
  'requestId',
  'roomId',
  'recipientId',
  'sender',
  'kind',
  'preparedAtMs',
  'delivery',
  'recipientAcknowledged',
  'final',
];

export function decodeHistoryPage(value: unknown, query: HistoryQuery): HistoryPage {
  const intent = decodeHistoryQuery(query);
  const page = exactRecord(value, ['items', 'nextBefore'], 'history page');
  if (!Array.isArray(page.items) || page.items.length > intent.limit)
    throw new Error('Invalid history page size.');
  const items = page.items.map((value): HistorySummary => {
    const row = exactRecord(value, [...ITEM_FIELDS, 'preview'], 'history summary');
    const item = historyItem(
      row,
      decodeFinal(row.final, [], () => undefined)
    );
    if (
      row.preview !== null &&
      (typeof row.preview !== 'string' ||
        /[\uD800-\uDFFF]/u.test(row.preview) ||
        Array.from(row.preview).length > REQUEST_HISTORY_PREVIEW_CHARS)
    )
      throw new Error('Invalid request preview.');
    if (
      (intent.recipientId && item.recipientId !== intent.recipientId) ||
      (intent.roomId && item.roomId !== intent.roomId)
    )
      throw new Error('Unexpected request history scope.');
    return { ...item, preview: row.preview as string | null };
  });
  for (let index = 0; index < items.length; index++) {
    const previous = index === 0 ? intent.before : items[index - 1];
    if (previous && !precedes(items[index]!, previous))
      throw new Error('Invalid request history order.');
  }
  const nextBefore = page.nextBefore === null ? null : cursor(page.nextBefore);
  const last = items.at(-1);
  if (
    nextBefore &&
    (items.length !== intent.limit ||
      !last ||
      nextBefore.requestId !== last.requestId ||
      nextBefore.preparedAtMs !== last.preparedAtMs)
  )
    throw new Error('Invalid next history cursor.');
  return { items, nextBefore };
}

export function decodeHistoryDetail(value: unknown): HistoryDetail {
  const row = exactRecord(value, [...ITEM_FIELDS, 'prompt'], 'request detail');
  const item = historyItem(
    row,
    decodeFinal(row.final, ['response'], (row, bytes) => exactText(row.response, bytes))
  );
  const valuePrompt = row.prompt;
  if (!valuePrompt || typeof valuePrompt !== 'object' || !('status' in valuePrompt))
    throw new Error('Invalid request prompt.');
  let prompt: RequestPrompt;
  switch (valuePrompt.status) {
    case 'unavailable':
      exactRecord(valuePrompt, ['status'], 'request prompt');
      prompt = { status: 'unavailable' };
      break;
    case 'expired': {
      const value = exactRecord(valuePrompt, ['status', 'expiresAtMs'], 'request prompt');
      prompt = { status: 'expired', expiresAtMs: integer(value.expiresAtMs, 1) };
      break;
    }
    case 'retained': {
      const value = exactRecord(
        valuePrompt,
        ['status', 'message', 'messageBytes', 'expiresAtMs'],
        'request prompt'
      );
      const messageBytes = integer(value.messageBytes, 0, DISPATCH_MESSAGE_LIMIT);
      prompt = {
        status: 'retained',
        message: exactText(value.message, messageBytes),
        messageBytes,
        expiresAtMs: integer(value.expiresAtMs, 1),
      };
      break;
    }
    default:
      throw new Error('Invalid request prompt state.');
  }
  return { ...item, prompt };
}
