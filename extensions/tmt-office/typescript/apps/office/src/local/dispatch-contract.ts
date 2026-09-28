import { canonicalUuid as identifier, exactRecord } from '../contracts/record.js';

export const DISPATCH_RECIPIENT_LIMIT = 64;
export const DISPATCH_MESSAGE_LIMIT = 1024 * 1024;

export interface DispatchInput {
  kind?: 'request' | 'announcement';
  operationId: string;
  recipientIds: string[];
  message: string;
  room?: DispatchRoom;
}
export type DispatchRoom =
  | { kind: 'direct'; roomId: string }
  | { kind: 'roster'; roomId: string; revision: number };
export class RoomRosterChanged extends Error {
  constructor() {
    super(
      'The room changed. Refresh rooms and explicitly use the current roster before reviewing again.'
    );
  }
}
/** A definitive rejection is not an instruction to replay the same operation. */
export class DispatchRejected extends Error {
  constructor(readonly reason: 'invalid' | 'operationConflict' | 'roomMembership') {
    super(
      reason === 'roomMembership'
        ? 'This agent is no longer a member of the selected room. No new message was queued. Earlier uncertain sends can still be recovered.'
        : reason === 'invalid'
          ? 'The host rejected this message. Discard this composition before starting a new one; this does not cancel earlier queued work.'
          : 'This operation ID belongs to a different message. Do not retry it. Discard this composition before starting a new one; this does not cancel earlier queued work.'
    );
  }
}
export interface DispatchItem {
  recipientId: string;
  requestId: string;
  acceptance: 'queued' | 'recipientUnavailable';
}
export interface DispatchReceipt {
  operationId: string;
  createdAtMs: number;
  items: DispatchItem[];
  /** Advisory pane notification; absence carries no wake evidence, including on replay. */
  wake?: {
    status: 'notAttempted' | 'unknown' | 'sent' | 'unavailable' | 'uncertain';
    paneAttempted: boolean | null;
    agentProcessed: null;
  };
}
export interface DispatchPort {
  send(input: DispatchInput, signal?: AbortSignal): Promise<DispatchReceipt>;
}

export function matchDispatchReceipt(
  receipt: DispatchReceipt,
  intent: DispatchInput
): DispatchReceipt {
  if (
    receipt.operationId !== intent.operationId ||
    JSON.stringify(receipt.items.map((item) => item.recipientId)) !==
      JSON.stringify(intent.recipientIds)
  )
    throw new Error('Unexpected dispatch receipt.');
  return receipt;
}

export function requestIdentifier(value: unknown): string {
  if (typeof value !== 'string' || !value.startsWith('req_'))
    throw new Error('Invalid request identifier.');
  identifier(value.slice(4));
  return value;
}

export function decodeDispatchInput(value: unknown): DispatchInput {
  const hasRoom = Boolean(value && typeof value === 'object' && Object.hasOwn(value, 'room'));
  const hasKind = Boolean(value && typeof value === 'object' && Object.hasOwn(value, 'kind'));
  const input = exactRecord(
    value,
    [
      'operationId',
      'recipientIds',
      'message',
      ...(hasRoom ? ['room'] : []),
      ...(hasKind ? ['kind'] : []),
    ],
    'dispatch'
  );
  if (
    (hasKind && input.kind !== 'request' && input.kind !== 'announcement') ||
    !Array.isArray(input.recipientIds) ||
    input.recipientIds.length < 1 ||
    input.recipientIds.length > DISPATCH_RECIPIENT_LIMIT ||
    typeof input.message !== 'string' ||
    !/[^\p{White_Space}]/u.test(input.message) ||
    /[\uD800-\uDFFF]/u.test(input.message) ||
    new TextEncoder().encode(input.message).length > DISPATCH_MESSAGE_LIMIT
  )
    throw new Error('Choose a valid message kind, 1–64 recipients and a message up to 1 MiB.');
  const recipientIds = [...new Set(input.recipientIds.map(identifier))].sort();
  let room: DispatchRoom | undefined;
  if (input.room != null) {
    const kind = (input.room as Record<string, unknown>).kind;
    const scope = exactRecord(
      input.room,
      kind === 'direct' ? ['kind', 'roomId'] : ['kind', 'roomId', 'revision'],
      'dispatch room'
    );
    if (kind === 'direct') {
      if (recipientIds.length !== 1)
        throw new Error('A direct room message has exactly one recipient.');
      room = { kind, roomId: identifier(scope.roomId) };
    } else if (
      kind === 'roster' &&
      Number.isSafeInteger(scope.revision) &&
      (scope.revision as number) >= 1
    ) {
      room = { kind, roomId: identifier(scope.roomId), revision: scope.revision as number };
    } else throw new Error('Choose direct room context or a valid reviewed room roster.');
  }
  return {
    operationId: identifier(input.operationId),
    recipientIds,
    message: input.message,
    ...(input.kind === 'announcement' ? { kind: 'announcement' as const } : {}),
    ...(room ? { room } : {}),
  };
}

export function decodeDispatchReceipt(value: unknown): DispatchReceipt {
  const hasWake = Boolean(value && typeof value === 'object' && Object.hasOwn(value, 'wake'));
  const receipt = exactRecord(
    value,
    ['operationId', 'createdAtMs', 'items', ...(hasWake ? ['wake'] : [])],
    'dispatch receipt'
  );
  if (
    !Number.isSafeInteger(receipt.createdAtMs) ||
    (receipt.createdAtMs as number) < 1 ||
    !Array.isArray(receipt.items) ||
    receipt.items.length < 1 ||
    receipt.items.length > DISPATCH_RECIPIENT_LIMIT
  )
    throw new Error('Invalid dispatch receipt.');
  const items = receipt.items.map((value): DispatchItem => {
    const item = exactRecord(value, ['recipientId', 'requestId', 'acceptance'], 'dispatch item');
    if (item.acceptance !== 'queued' && item.acceptance !== 'recipientUnavailable')
      throw new Error('Invalid dispatch item.');
    return {
      recipientId: identifier(item.recipientId),
      requestId: requestIdentifier(item.requestId),
      acceptance: item.acceptance,
    };
  });
  if (items.some((item, index) => index > 0 && items[index - 1]!.recipientId >= item.recipientId))
    throw new Error('Invalid dispatch audience.');
  let wake: DispatchReceipt['wake'];
  if (hasWake) {
    const value = exactRecord(receipt.wake, ['status', 'paneAttempted', 'agentProcessed'], 'wake');
    if (
      !['notAttempted', 'unknown', 'sent', 'unavailable', 'uncertain'].includes(
        value.status as string
      ) ||
      value.agentProcessed !== null ||
      value.paneAttempted !==
        (value.status === 'sent' || value.status === 'uncertain'
          ? true
          : value.status === 'unknown'
            ? null
            : false)
    )
      throw new Error('Invalid wake outcome.');
    wake = value as DispatchReceipt['wake'];
  }
  return {
    operationId: identifier(receipt.operationId),
    createdAtMs: receipt.createdAtMs as number,
    items,
    ...(wake ? { wake } : {}),
  };
}
