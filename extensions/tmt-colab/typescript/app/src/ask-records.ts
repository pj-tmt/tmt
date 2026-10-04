import {
  binary,
  coreId,
  decimal,
  decodeText,
  digest,
  equal,
  exactKeys,
  fields,
  generatedId,
  idList,
  requireValue,
  spaceId,
  strictVerify,
  text,
  time,
} from '@tmt/colab-client';
import type { SignedAsk } from './ask-intent.js';
import type { OwnState } from './fold-protocol.js';

export const ASK_INPUT_BYTES = 16 * 1024;
export const ASK_MESSAGE_BYTES = 64 * 1024;
export const ASK_REPLY_BYTES = 16 * 1024;
export const ASK_OBSERVATION_MS = 2 * 60 * 60 * 1000;
export type LedgerState =
  | 'dispatching'
  | 'held'
  | 'accepted'
  | 'uncertain'
  | 'failed'
  | 'refused'
  | 'cancelled'
  | 'expired'
  | 'abandoned';
const STATES: LedgerState[] = [
  'dispatching',
  'held',
  'accepted',
  'uncertain',
  'failed',
  'refused',
  'cancelled',
  'expired',
  'abandoned',
];
export interface DecodedAsk {
  space: string;
  page: string;
  thread: string;
  messageIds: string[];
  machine: string;
  agent: string;
  operationId: string;
  senderDevice: string;
  grantExpiresAt: number | null;
  grantRevision: string;
  issuedAt: number;
  expiresAt: number;
  message: string;
}
export interface AskIntentRecord {
  version: 1;
  kind: 'ask';
  signed: SignedAsk;
  agentName: string;
  deviceName: string;
}
export interface AskStateRecord {
  version: 1;
  kind: 'ask-state';
  operationId: string;
  revision: string;
  state: LedgerState;
  requestId: string | null;
  reason: string | null;
}
export interface AskReplyRecord {
  version: 1;
  kind: 'ask-reply';
  operationId: string;
  requestId: string;
  agentId: string;
  body: string;
}
export type AskRecord = AskIntentRecord | AskStateRecord | AskReplyRecord;
export type AskRoot = 'intents' | 'messages' | 'replies';
export interface AskLedgerView {
  writer: string;
  agentName: string;
  deviceName: string;
  signed: SignedAsk;
  intent: DecodedAsk;
  state: LedgerState;
  revision: string;
  requestId: string | null;
  reason: string | null;
  reply: AskReplyRecord | null;
}
export function requestId(value: unknown): asserts value is string {
  requireValue(typeof value === 'string' && value.startsWith('req_'));
  coreId(value.slice(4));
}
function timestamp(value: Uint8Array) {
  const s = decodeText(value);
  const n = decimal(s, true);
  requireValue(n <= BigInt(Number.MAX_SAFE_INTEGER));
  time(Number(n));
  return Number(n);
}
/** Syntax and scope, not effect authority. Historical records remain readable
 * after expiry; the explicit-send controller separately checks current time. */
export function decodeAsk(value: unknown): DecodedAsk {
  exactKeys(value, ['operationId', 'senderDevice', 'input', 'signature', 'finalBytes']);
  generatedId(value.operationId as string);
  generatedId(value.senderDevice as string);
  binary(value.signature, 64, 64);
  const f = fields(binary(value.input, ASK_INPUT_BYTES), 15, ASK_INPUT_BYTES);
  requireValue(decodeText(f[0]) === 'tmt-colab-send-v1' && decodeText(f[1]) === '1');
  const s = f.map((value, i) => (i === 5 || i === 9 ? '' : decodeText(value)));
  spaceId(s[2]);
  for (const i of [3, 4, 6, 8, 10]) generatedId(s[i]);
  coreId(s[7]);
  decimal(s[11]);
  const grantExpiresAt = s[12] === 'none' ? null : timestamp(f[12]);
  requireValue(f[9].length === 32 && f[5].length >= 4);
  const count = new DataView(f[5].buffer, f[5].byteOffset).getUint32(0);
  requireValue(count <= 256);
  const ids = fields(f[5].slice(4), count, 10240).map(decodeText);
  requireValue(equal(f[5], idList(ids)));
  const issuedAt = timestamp(f[13]),
    expiresAt = timestamp(f[14]);
  requireValue(expiresAt > issuedAt && expiresAt - issuedAt <= 86400000);
  requireValue(s[8] === value.operationId && s[10] === value.senderDevice);
  const message = decodeText(binary(value.finalBytes, ASK_MESSAGE_BYTES));
  return {
    space: s[2],
    page: s[3],
    thread: s[4],
    messageIds: ids,
    machine: s[6],
    agent: s[7],
    operationId: s[8],
    senderDevice: s[10],
    grantRevision: s[11],
    grantExpiresAt,
    issuedAt,
    expiresAt,
    message,
  };
}
export async function verifyAsk(value: SignedAsk, key: Uint8Array) {
  const intent = decodeAsk(value),
    input = binary(value.input, ASK_INPUT_BYTES);
  requireValue(
    equal(
      fields(input, 15, ASK_INPUT_BYTES)[9],
      await digest(binary(value.finalBytes, ASK_MESSAGE_BYTES)),
    ),
  );
  requireValue(await strictVerify(key, binary(value.signature, 64, 64), input));
  return intent;
}
export function recordKey(record: AskRecord): { root: AskRoot; key: string } {
  if (record.kind === 'ask') return { root: 'intents', key: record.signed.operationId };
  if (record.kind === 'ask-state')
    return { root: 'messages', key: `${record.operationId}:${record.revision}` };
  return { root: 'replies', key: record.operationId };
}
export function validateRecord(value: unknown): asserts value is AskRecord {
  requireValue(value !== null && typeof value === 'object');
  const r = value as Record<string, unknown>;
  if (r.kind === 'ask') {
    exactKeys(r, ['version', 'kind', 'signed', 'agentName', 'deviceName']);
    requireValue(typeof r.agentName === 'string' && text(r.agentName).length <= 128);
    requireValue(typeof r.deviceName === 'string' && text(r.deviceName).length <= 128);
    decodeAsk(r.signed);
  } else if (r.kind === 'ask-state') {
    exactKeys(r, ['version', 'kind', 'operationId', 'revision', 'state', 'requestId', 'reason']);
    generatedId(r.operationId as string);
    decimal(r.revision as string);
    requireValue(STATES.includes(r.state as LedgerState));
    if (r.requestId !== null) requestId(r.requestId);
    requireValue(r.state !== 'accepted' || r.requestId !== null);
    requireValue(
      r.reason === null ||
        (typeof r.reason === 'string' && /^[A-Z][A-Z0-9_]{0,63}$/.test(r.reason)),
    );
  } else {
    exactKeys(r, ['version', 'kind', 'operationId', 'requestId', 'agentId', 'body']);
    requireValue(r.kind === 'ask-reply');
    generatedId(r.operationId as string);
    requestId(r.requestId);
    coreId(r.agentId as string);
    requireValue(typeof r.body === 'string' && text(r.body).length <= ASK_REPLY_BYTES);
  }
  requireValue(r.version === 1);
}
export function canTransition(from: LedgerState, to: LedgerState) {
  if (from === to) return true;
  if (from === 'dispatching')
    return ['held', 'accepted', 'uncertain', 'failed', 'refused', 'cancelled', 'expired'].includes(
      to,
    );
  if (from === 'held') return ['accepted', 'uncertain', 'refused', 'cancelled'].includes(to);
  if (from === 'uncertain')
    return ['held', 'accepted', 'refused', 'cancelled', 'abandoned'].includes(to);
  return false;
}
/** Values are taken only from the admitted per-writer projection. A claimed
 * writer, request ID or agent in a JSON body can never select another stream. */
export async function readAskRecords(
  own: OwnState,
  scope: { space: string; page: string },
  signer: (writer: string) => Uint8Array | undefined,
): Promise<AskLedgerView[]> {
  const views: AskLedgerView[] = [];
  for (const [writer, roots] of Object.entries(own)) {
    const publicKey = signer(writer);
    if (!publicKey) continue;
    for (const [id, value] of Object.entries(roots.intents)) {
      if (!value || typeof value !== 'object' || Array.isArray(value) || value.kind !== 'ask')
        continue;
      try {
        validateRecord(value);
        requireValue(value.kind === 'ask');
        const intent = await verifyAsk(value.signed, publicKey);
        requireValue(
          intent.operationId === id &&
            intent.senderDevice === writer &&
            intent.space === scope.space &&
            intent.page === scope.page,
        );
        const view: AskLedgerView = {
          writer,
          agentName: value.agentName,
          deviceName: value.deviceName,
          signed: structuredClone(value.signed),
          intent,
          state: 'uncertain',
          revision: '0',
          requestId: null,
          reason: null,
          reply: null,
        };
        const states: AskStateRecord[] = [];
        for (const [key, item] of Object.entries(roots.messages)) {
          if (
            !item ||
            typeof item !== 'object' ||
            Array.isArray(item) ||
            item.kind !== 'ask-state' ||
            item.operationId !== id
          )
            continue;
          validateRecord(item);
          requireValue(item.kind === 'ask-state' && recordKey(item).key === key);
          states.push(item);
        }
        states.sort((a, b) =>
          BigInt(a.revision) < BigInt(b.revision)
            ? -1
            : BigInt(a.revision) > BigInt(b.revision)
              ? 1
              : 0,
        );
        for (const state of states) {
          requireValue(view.revision === '0' || canTransition(view.state, state.state));
          requireValue(view.requestId === null || state.requestId === view.requestId);
          Object.assign(view, {
            state: state.state,
            revision: state.revision,
            requestId: state.requestId,
            reason: state.reason,
          });
        }
        const reply = roots.replies[id];
        if (reply !== undefined) {
          validateRecord(reply);
          requireValue(
            reply.kind === 'ask-reply' &&
              view.state === 'accepted' &&
              reply.operationId === id &&
              reply.agentId === intent.agent &&
              reply.requestId === view.requestId,
          );
          view.reply = structuredClone(reply);
        }
        views.push(view);
      } catch {
        // Malformed/unrelated content has no attribution and no effect capability.
      }
    }
  }
  return views;
}

export const readAskViews = readAskRecords;
