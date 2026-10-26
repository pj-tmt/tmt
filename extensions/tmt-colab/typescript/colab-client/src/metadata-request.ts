/** Closed version-2 edit requests and terminal receipts. Syntax never grants writer authority. */
import { binary, encodeBinary, exactKeys, generatedId, requireValue, text } from './bytes.js';
import { digest } from './crypto.js';
import { canonical } from './json.js';

export type MetadataRequest = {
  version: 2;
  kind: 'edit-request';
  operationId: string;
  target: Record<string, unknown>;
} & (
  | { action: 'creator.set'; body: { selector: string } }
  | { action: 'question.handle' | 'question.reopen'; body: { id: string } }
);
/** Observed metadata is JSON with browser-safe integral times, as on the native side. */
function targetValue(value: unknown, depth = 0): void {
  requireValue(depth <= 128);
  if (value === null || typeof value === 'boolean') return;
  if (typeof value === 'string') {
    text(value);
    return;
  }
  if (typeof value === 'number') {
    requireValue(Number.isSafeInteger(value) && value >= 0);
    return;
  }
  requireValue(value !== null && typeof value === 'object');
  if (Array.isArray(value)) {
    for (const item of value) targetValue(item, depth + 1);
    return;
  }
  for (const [key, item] of Object.entries(value)) {
    requireValue(
      [...key].every((character) => character.charCodeAt(0) < 128) && !/^\d+$/.test(key),
    );
    targetValue(item, depth + 1);
  }
}
export function metadataRequest(value: unknown, root = 'intents', key?: string): MetadataRequest {
  exactKeys(value, ['version', 'kind', 'operationId', 'action', 'body', 'target']);
  const v = value as unknown as MetadataRequest;
  requireValue(v.version === 2 && v.kind === 'edit-request' && root === 'intents');
  generatedId(v.operationId);
  requireValue(key === undefined || key === v.operationId);
  targetValue(v.target);
  requireValue(text(canonical(v)).length <= 8192);
  if (v.action === 'creator.set') {
    exactKeys(v.body, ['selector']);
    const selector = v.body.selector;
    requireValue(
      typeof selector === 'string' &&
        selector.length > 0 &&
        text(selector).length <= 256 &&
        selector.trim() === selector &&
        !/[\p{Cc}]/u.test(selector),
    );
    exactKeys(v.target, ['creator', 'creationRecipient']);
  } else {
    requireValue(v.action === 'question.handle' || v.action === 'question.reopen');
    exactKeys(v.body, ['id']);
    generatedId(v.body.id);
    requireValue(
      v.target !== null &&
        typeof v.target === 'object' &&
        !Array.isArray(v.target) &&
        v.target.id === v.body.id,
    );
  }
  return v;
}
export interface EditResult {
  version: 1;
  kind: 'edit-result';
  writer: string;
  operationId: string;
  requestSha256: string;
  result:
    | { operationId: string; state: 'committed' | 'unchanged'; revision: string }
    | { operationId: string; state: 'rejected'; code: string; message: string };
}
export const outcomeKey = (writer: string, operation: string) =>
  `${writer}:${operation}:edit-result`;
export function editResult(value: unknown, root = 'messages', key?: string): EditResult {
  exactKeys(value, ['version', 'kind', 'writer', 'operationId', 'requestSha256', 'result']);
  const v = value as unknown as EditResult;
  generatedId(v.writer);
  generatedId(v.operationId);
  requireValue(
    v.version === 1 &&
      v.kind === 'edit-result' &&
      root === 'messages' &&
      (key === undefined || key === outcomeKey(v.writer, v.operationId)),
  );
  binary(v.requestSha256, 32, 32);
  const r = v.result;
  requireValue(r !== null && typeof r === 'object' && r.operationId === v.operationId);
  if (r.state === 'committed' || r.state === 'unchanged') {
    exactKeys(r, ['operationId', 'state', 'revision']);
    requireValue(typeof r.revision === 'string' && /^v1:[0-9a-f]{64}$/.test(r.revision));
  } else {
    requireValue(r.state === 'rejected');
    exactKeys(r, ['operationId', 'state', 'code', 'message']);
    requireValue(
      typeof r.code === 'string' && r.code.startsWith('COLAB_') && text(r.code).length <= 80,
    );
    requireValue(
      typeof r.message === 'string' && r.message.length > 0 && text(r.message).length <= 2048,
    );
  }
  return v;
}
/** Call only on verified own projections. A valid link-authored receipt is still untrusted. */
export async function resultFor(
  value: unknown,
  ownerMember: boolean,
  writer: string,
  request: MetadataRequest,
): Promise<EditResult['result'] | undefined> {
  if (!ownerMember) return undefined;
  const result = editResult(value);
  if (
    result.writer !== writer ||
    result.operationId !== request.operationId ||
    result.requestSha256 !== encodeBinary(await digest(text(canonical(request))))
  )
    return undefined;
  return result.result;
}
