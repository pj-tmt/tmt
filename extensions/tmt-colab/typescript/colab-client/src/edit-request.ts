/** The typed `edit-request` record of the own `intents` root: what a link holder who cannot seal
 * content asks the page's owner to save. Mirrors `edit_request.rs`; the owner's serve decides. */
import {
  binary,
  encodeBinary,
  exactKeys,
  generatedId,
  requireValue,
  text,
  type Bytes,
} from './bytes.js';
import { digest } from './crypto.js';

/** The most source a request may carry, in UTF-8 bytes. */
export const SOURCE_BYTES = 128 * 1024;
/** The most anchored-hunk text (old and new together), in UTF-8 bytes. */
export const HUNK_BYTES = 64 * 1024;
/** The most hunks, as for a browser Save. */
export const HUNKS = 64;
export interface EditHunk {
  oldStart: number;
  old: string;
  new: string;
}
export interface EditRequest {
  version: 1;
  kind: 'edit-request';
  operationId: string;
  baseSha256: string;
  sourceSha256: string;
  source: string;
  hunks?: EditHunk[];
}
function hunk(value: unknown): EditHunk {
  exactKeys(value, ['oldStart', 'old', 'new']);
  const { oldStart, old, new: replacement } = value;
  requireValue(
    typeof oldStart === 'number' &&
      Number.isInteger(oldStart) &&
      oldStart > 0 &&
      oldStart <= 0xffffffff &&
      typeof old === 'string' &&
      old.length > 0 &&
      typeof replacement === 'string',
  );
  return { oldStart, old, new: replacement };
}
/** Checks the record's shape and sizes; `key` is the `intents` key it is stored under. The
 * source digest is checked by `verifiedEditRequest`, which hashes. */
export function editRequest(value: unknown, root = 'intents', key?: string): EditRequest {
  requireValue(value !== null && typeof value === 'object' && !Array.isArray(value));
  const fields = ['version', 'kind', 'operationId', 'baseSha256', 'sourceSha256', 'source'];
  exactKeys(value, Object.hasOwn(value, 'hunks') ? [...fields, 'hunks'] : fields);
  const v = value as unknown as EditRequest;
  requireValue(v.version === 1 && v.kind === 'edit-request' && root === 'intents');
  generatedId(v.operationId);
  requireValue(key === undefined || key === v.operationId);
  binary(v.baseSha256, 32, 32);
  binary(v.sourceSha256, 32, 32);
  requireValue(typeof v.source === 'string' && text(v.source).length <= SOURCE_BYTES);
  if (v.hunks !== undefined) {
    requireValue(Array.isArray(v.hunks) && v.hunks.length > 0 && v.hunks.length <= HUNKS);
    let bytes = 0;
    for (const h of v.hunks) {
      const checked = hunk(h);
      bytes += text(checked.old).length + text(checked.new).length;
    }
    requireValue(bytes <= HUNK_BYTES);
  }
  return v;
}
export async function sourceDigest(source: string): Promise<Bytes> {
  return digest(text(source));
}
/** `editRequest` plus the check that the source hashes to `sourceSha256`. */
export async function verifiedEditRequest(
  value: unknown,
  root = 'intents',
  key?: string,
): Promise<EditRequest> {
  const v = editRequest(value, root, key);
  requireValue(encodeBinary(await sourceDigest(v.source)) === v.sourceSha256);
  return v;
}
